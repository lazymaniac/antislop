//! Terminal frontend for one conversation: Canvas, inspector, Workbench and
//! the session composer. Drawing uses local projections and cached documents;
//! daemon requests and event polling run on background threads and deliver
//! updates to the frontend. See `docs/grouped-work-tui.md` for the surface map.

mod buffer;
mod app {
    mod commands;
    mod compose;
    mod conversation;
    mod diff;
    mod graph;
    mod interview;
    mod picker;
    mod pointer;
    mod projects;
    pub(super) mod requests;
    mod workflow;
}
#[cfg(test)]
mod capture;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod theme_modes_tests;

#[cfg(test)]
std::thread_local! {
    static GRAPH_LAYOUT_REBUILDS: std::cell::RefCell<std::collections::HashMap<usize, usize>> =
        std::cell::RefCell::default();
}

mod approval;
pub mod clipboard;
pub mod config;
pub mod consent;
mod contracts;
mod dashboard;
mod decision_inbox;
pub mod details;
pub mod diffview;
pub mod document;
pub mod editor;
mod events;
pub mod flow_editor;
pub mod graph;
mod graph_filters;
mod history;
mod input;
mod input_owner;
mod inspection;
pub mod interview;
mod layout;
mod message_inspector;
mod native_view;
mod navigation;
pub mod palette;
mod paste;
mod planhistory;
pub mod planview;
mod render;
mod result_text;
pub mod sanitize;
pub mod selection;
mod shell;
mod state;
pub mod taskinfo;
pub mod theme;
pub mod unblock;
pub mod update_queue;
pub mod view;
pub mod work_units;
mod workbench;
pub mod workbench_summary;
pub mod workspace;

pub use contracts::{
    ConversationState, Editor, Focus, NavigationFocus, NavigationState, Screen, Update,
    WorkbenchPresentation, WorkbenchTab, WorkbenchTarget,
};
pub use details::Details;
pub use diffview::DiffView;
pub use editor::{InputBuffer, sanitize_paste, sanitize_single_line_paste, visual_text_lines};
pub use flow_editor::{FlowEditMode, FlowEditor, FlowEditorOutcome};
pub use graph::{GroupingMode, Relations, Scope};
pub use palette::{Command, Help, Kind, Picker};
pub use planview::{PlanAction, PlanView};
#[cfg(test)]
use render::transcript_cache::TranscriptLayout;
pub use sanitize::{sanitize, sanitize_line};
pub use shell::run;
pub use state::App;
pub use theme::{
    CardProps, Glyphs, Row, Theme, Tone, card, draw_rows, scrollbar, section, split_row, tone,
};
pub use update_queue::{QueueClosed, QueueEmpty, UpdateReceiver, UpdateSender, update_channel};
pub use view::{MessageEntry, TranscriptCard};
pub use workbench::{
    WorkbenchAreas, WorkbenchContext, WorkbenchRole, areas as workbench_areas,
    context as workbench_context, default_tab as workbench_default_tab,
    neighbour_tab as workbench_neighbour_tab, resize_at as resize_workbench_at,
    tabs_for as workbench_tabs_for,
};

use super::daemon::{Client, Request};
use super::projects;
use crate::Result;
use crossterm::event::{self, KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use graph::{Canvas, Graph};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
#[cfg(test)]
use view::SIDE_BY_SIDE_BREAKPOINT;
#[cfg(test)]
use view::dim;
use view::{ORCHESTRATOR, PanelFocus, View, draw_min_term_guard, target_name, truncate, wrap};

/// Bound a centered panel while preserving the full viewport on small terminals.
fn panel_area(area: Rect, max_width: u16) -> Rect {
    if area.width < 16 || area.height < 7 {
        return area;
    }
    let width = area.width.min(max_width.saturating_add(4));
    let height = area.height.saturating_sub(4).max(1);
    let x = area.x + (area.width - width) / 2;
    let y = area.y + (area.height - height) / 2;
    let panel = Rect::new(x, y, width, height);
    if width > 4 && height > 4 {
        Rect::new(panel.x + 2, panel.y + 2, panel.width - 4, panel.height - 4)
    } else {
        panel
    }
}

/// Center a list viewport with its own 72-column content limit.
fn list_area(area: Rect) -> Rect {
    panel_area(area, 72)
}

/// The ASCII stand-in for a character the screen uses; `?` for others.
fn ascii(ch: char) -> char {
    theme::ascii_character(ch)
}

/// Replaces every character with its ASCII stand-in (design §8.3).
pub fn in_ascii(buffer: &mut ratatui::buffer::Buffer) {
    for cell in &mut buffer.content {
        if !cell.symbol().is_ascii() {
            let first = cell.symbol().chars().next().unwrap_or(' ');
            cell.set_char(ascii(first));
        }
    }
}

/// Drops every color; what was dim stays dim, selection is reversed anyway.
pub fn without_color(buffer: &mut ratatui::buffer::Buffer) {
    for cell in &mut buffer.content {
        if cell.fg == theme::ayu::DIM || cell.fg == theme::ayu::FAINT {
            cell.modifier.insert(Modifier::DIM);
        }
        cell.set_fg(Color::Reset).set_bg(Color::Reset);
    }
}

/// Whether the terminal advertises 24-bit color; `ANTISLOP_TRUECOLOR=1` forces it.
pub fn truecolor_terminal() -> bool {
    let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    set("ANTISLOP_TRUECOLOR")
        || std::env::var("COLORTERM")
            .is_ok_and(|value| matches!(value.as_str(), "truecolor" | "24bit"))
}

/// The xterm-256 color closest to an RGB value: the 6x6x6 cube or the gray ramp.
pub fn nearest_indexed(r: u8, g: u8, b: u8) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let level = |v: u8| {
        (0..6)
            .min_by_key(|&i| (i32::from(LEVELS[i]) - i32::from(v)).abs())
            .unwrap_or(0)
    };
    let distance = |a: (u8, u8, u8)| {
        let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
        d(a.0, r) + d(a.1, g) + d(a.2, b)
    };
    let (ri, gi, bi) = (level(r), level(g), level(b));
    let cube = (LEVELS[ri], LEVELS[gi], LEVELS[bi]);
    let average = ((u32::from(r) + u32::from(g) + u32::from(b)) / 3) as i32;
    let step = ((average - 8 + 5) / 10).clamp(0, 23);
    let gray = (8 + 10 * step) as u8;
    if distance((gray, gray, gray)) < distance(cube) {
        232 + step as u8
    } else {
        (16 + 36 * ri + 6 * gi + bi) as u8
    }
}

/// Maps every RGB color to its nearest xterm-256 index for terminals without true color.
pub fn to_indexed_colors(buffer: &mut ratatui::buffer::Buffer) {
    let map = |color: Color| match color {
        Color::Rgb(r, g, b) => Color::Indexed(nearest_indexed(r, g, b)),
        other => other,
    };
    for cell in &mut buffer.content {
        cell.fg = map(cell.fg);
        cell.bg = map(cell.bg);
    }
}

/// `~` and `~/…` mean the user's home; other paths are used as typed.
fn expand_home(path: &str) -> std::path::PathBuf {
    let path = path.trim();
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => std::path::PathBuf::from(home).join(rest),
        (None, Some(home)) if path == "~" => std::path::PathBuf::from(home),
        _ => std::path::PathBuf::from(path),
    }
}

/// Asks the daemon to unblock a task, with a freshly read token so the
/// request is about the state the user is looking at. Returns what the
/// daemon says happened.
fn recover(
    client: &Client,
    conversation: &str,
    plan: &str,
    task: &str,
    action: crate::lean::RecoveryAction,
    message: Option<String>,
) -> std::result::Result<String, String> {
    if task == ORCHESTRATOR && action == crate::lean::RecoveryAction::RestartAgent {
        return recover_orchestrator(client, conversation);
    }
    if task == view::AUDITOR && action == crate::lean::RecoveryAction::RestartAgent {
        return client
            .call(&Request::Conversation(
                crate::lean::daemon::ConversationRequest::Resume {
                    conversation: conversation.into(),
                    operation_id: Some(crate::id()),
                    plan: plan.into(),
                    revision_token: None,
                },
            ))
            .map(|_| "final audit restarted; approved task results preserved".into())
            .map_err(|error| error.to_string());
    }
    recover_task(client, conversation, plan, task, action, message)
}

fn recover_orchestrator(
    client: &Client,
    conversation: &str,
) -> std::result::Result<String, String> {
    client
        .call(&Request::Conversation(
            crate::lean::daemon::ConversationRequest::RestartOrchestrator {
                conversation: conversation.into(),
                operation_id: Some(crate::id()),
            },
        ))
        .map(|reply| {
            reply
                .as_str()
                .unwrap_or("restarting the orchestrator")
                .to_owned()
        })
        .map_err(|error| error.to_string())
}

fn recover_task(
    client: &Client,
    conversation: &str,
    plan: &str,
    task: &str,
    action: crate::lean::RecoveryAction,
    message: Option<String>,
) -> std::result::Result<String, String> {
    let token = recovery_token(client, plan, task)?;
    let reply = client
        .call(&Request::Conversation(
            crate::lean::daemon::ConversationRequest::Recover(crate::lean::TaskRecoveryRequest {
                conversation: conversation.to_owned(),
                plan: plan.to_owned(),
                task: task.to_owned(),
                action,
                consent_files: None,
                freshness_token: token,
                message,
            }),
        ))
        .map_err(|e| e.to_string())?;
    let status = reply["status"]
        .as_str()
        .ok_or_else(|| format!("recovery response has no final status: {reply}"))?;
    let notice = reply["notice"].as_str().unwrap_or("");
    Ok(if notice.is_empty() {
        status.to_owned()
    } else {
        format!("{status}: {notice}")
    })
}

fn recovery_token(client: &Client, plan: &str, task: &str) -> std::result::Result<String, String> {
    let stored = client
        .call(&Request::Conversation(
            crate::lean::daemon::ConversationRequest::Task {
                plan: plan.to_owned(),
                id: task.to_owned(),
                conversation: None,
            },
        ))
        .map_err(|e| e.to_string())?;
    let token = stored["freshness_token"]
        .as_str()
        .ok_or("the daemon gave no freshness token")?
        .to_owned();
    Ok(token)
}
