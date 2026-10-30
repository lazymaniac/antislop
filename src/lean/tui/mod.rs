//! Terminal frontend for one conversation: the chat with the orchestrator
//! beside the team graph. It never waits on the daemon in its draw loop: a
//! poller thread long-polls events and every action runs on its own thread,
//! so typing, scrolling and sending stay instant.

mod buffer;
mod app {
    mod commands;
    mod compose;
    mod confirmation;
    mod conversation;
    mod graph;
    mod interview;
    mod picker;
    mod pointer;
    mod projects;
    mod requests;
    mod workflow;
}
#[cfg(test)]
mod tests;

pub mod clipboard;
pub mod config;
mod contracts;
mod dashboard;
pub mod details;
pub mod diffview;
pub mod document;
pub mod editor;
mod events;
pub mod graph;
mod history;
mod input;
mod inspection;
pub mod interview;
mod layout;
pub mod ledger;
mod native_view;
mod navigation;
pub mod palette;
mod paste;
mod planhistory;
pub mod planview;
mod render;
pub mod sanitize;
pub mod selection;
mod shell;
mod state;
pub mod taskinfo;
pub mod theme;
pub mod unblock;
pub mod view;
pub mod workspace;

pub use contracts::{ConversationState, Editor, Focus, Update};
pub use details::Details;
pub use diffview::DiffView;
pub use editor::{InputBuffer, sanitize_paste, sanitize_single_line_paste, visual_text_lines};
pub use graph::{Filter, GroupingMode};
pub use palette::{Help, Kind, Picker};
pub use planview::{PlanAction, PlanView};
pub use sanitize::{sanitize, sanitize_line};
pub use shell::run;
pub use state::App;
#[cfg(test)]
use state::TranscriptLayout;
use state::TranscriptLayoutEntry;
pub use theme::{
    CardProps, Glyphs, Row, Theme, Tone, card, draw_rows, scrollbar, section, split_row, tone,
};
pub use view::{
    ConfirmationAction, ConfirmationCard, ConfirmationStatus, MessageEntry, TranscriptCard,
};

use super::daemon::{Client, Request};
use super::{projects, trust};
use crate::Result;
use crossterm::event::{self, KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use details::{CHECKS, COMMENTS, RESULT};
use graph::{Canvas, Graph};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;
use view::{
    ORCHESTRATOR, PanelFocus, SIDE_BY_SIDE_BREAKPOINT, View, dim, draw_min_term_guard, target_name,
    truncate, wrap,
};

/// Whether to draw in ASCII: asked for with `ANTISLOP_ASCII`, or the
/// locale is not UTF-8.
/// Shrinks `area` by one cell on every side so content sits centred with a margin.
/// Tiny areas are returned unchanged.
fn padded(area: Rect) -> Rect {
    if area.width > 4 && area.height > 4 {
        Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2)
    } else {
        area
    }
}

fn ascii_terminal() -> bool {
    if std::env::var_os("ANTISLOP_ASCII").is_some_and(|value| !value.is_empty()) {
        return true;
    }
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .unwrap_or_default()
        .to_ascii_lowercase();
    !(locale.contains("utf-8") || locale.contains("utf8"))
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
    Ok(reply["notice"].as_str().unwrap_or("done").to_owned())
}

fn recovery_token(client: &Client, plan: &str, task: &str) -> std::result::Result<String, String> {
    let stored = client
        .call(&Request::Conversation(
            crate::lean::daemon::ConversationRequest::Task {
                plan: plan.to_owned(),
                id: task.to_owned(),
            },
        ))
        .map_err(|e| e.to_string())?;
    let token = stored["freshness_token"]
        .as_str()
        .ok_or("the daemon gave no freshness token")?
        .to_owned();
    Ok(token)
}
