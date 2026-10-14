# Current terminal shell
`draw` splits header, chat/agent graph, footer. Wide terminals use 52/48 split; narrow terminals display focused panel. Detail views replace the main region.
## `src/tui/render.rs`
```rust
use super::{
    app::{App, Focus, View},
    model::{AGENT_TABS, RUN_TABS},
    sanitize::sanitize,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

fn style(app: &App, active: bool) -> Style {
    if app.no_color {
        if active {
            Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default()
        }
    } else if active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    }
}
fn block<'a>(app: &App, title: impl Into<Line<'a>>, active: bool) -> Block<'a> {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(style(app, active));
    if app.ascii {
        block.border_set(ratatui::symbols::border::Set {
            top_left: "+",
            top_right: "+",
            bottom_left: "+",
            bottom_right: "+",
            vertical_left: "|",
            vertical_right: "|",
            horizontal_top: "-",
            horizontal_bottom: "-",
        })
    } else {
        block
    }
}
fn clean(app: &App, text: &str) -> String {
    let text = sanitize(text);
    if app.ascii {
        text.graphemes(true)
            .map(|g| {
                if g.is_ascii() {
                    g.to_string()
                } else {
                    "?".repeat(g.width())
                }
            })
            .collect()
    } else {
        text
    }
}
fn lines_view(app: &App, text: &str, offset: usize, height: usize) -> String {
    // The viewport creates strings only for rendered lines, even for large reports.
    text.lines()
        .skip(offset)
        .take(height)
        .map(|line| clean(app, line))
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    app.width = area.width;
    app.height = area.height;
    if area.width < 12 || area.height < 5 {
        frame.render_widget(Paragraph::new("Resize terminal. q detaches."), area);
        return;
    }
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(area);
    let header = format!(
        " Antislop  run: {}  mode: {}  stage: {}  status: {}\n {}",
        if app.snapshot.run_id.is_empty() {
            "none"
        } else {
            &app.snapshot.run_id
        },
        app.snapshot.mode,
        app.snapshot.stage,
        app.snapshot.status,
        app.snapshot.workspace
    );
    frame.render_widget(Paragraph::new(clean(app, &header)), parts[0]);
    let main = parts[1];
    if main.width >= 85 {
        let panels = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
            .split(main);
        chat(frame, app, panels[0]);
        team(frame, app, panels[1]);
    } else if app.focus == Focus::Chat {
        chat(frame, app, main);
    } else {
        team(frame, app, main);
    }
    let footer = format!(
        "{}\nTab focus | i write | m agent | c comment | u comments | Enter inspect | / search | Ctrl+P commands | ? help | q detach",
        app.notice
    );
    frame.render_widget(Paragraph::new(clean(app, &footer)), parts[2]);
    match app.view.clone() {
        View::Main => (),
        View::Agent { tab } => {
            let tabs = AGENT_TABS
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    if i == tab {
                        format!("[{t}]")
                    } else {
                        t.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            let title = app
                .selected()
                .map(|a| format!("{} / {}", a.role, a.id))
                .unwrap_or_else(|| "Agent no longer available".into());
            let text = if tab == 7 {
                "c: choose any visible element to comment on. u: inspect comment history, orchestrator reactions, resolve/reopen or delete. Comments bind to the observed element version.".into()
            } else if tab == 4 {
                let id = app.selected_agent_id.as_deref().unwrap_or("");
                let messages = app
                    .snapshot
                    .messages
                    .iter()
                    .filter(|m| m.target == id || m.sender == id)
                    .map(|m| {
                        format!(
                            "{} -> {} [{}] session:{}\n{}",
                            m.sender, m.target, m.state, m.session, m.text
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                if messages.is_empty() {
                    "No recorded messages. m writes to this agent. Adapter delivery: next-turn queue; delivery requires a session receipt.".into()
                } else {
                    messages
                }
            } else {
                app.selected()
                    .and_then(|a| a.details.get(AGENT_TABS[tab]))
                    .cloned()
                    .unwrap_or_else(|| "No recorded data available.".into())
            };
            detail(
                frame,
                app,
                main,
                &format!("{title} | Esc back"),
                &tabs,
                &text,
            );
        }
        View::Run { tab } => {
            let tabs = RUN_TABS
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    if i == tab {
                        format!("[{t}]")
                    } else {
                        t.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            let text = app
                .snapshot
                .run_details
                .get(RUN_TABS[tab])
                .cloned()
                .unwrap_or_else(|| "No recorded data available.".into());
            detail(frame, app, main, "Run details | Esc back", &tabs, &text);
        }
        View::CommentTargets => {
            let targets = &app.annotation_targets;
            let offset = app
                .annotation_index
                .saturating_sub(main.height.saturating_sub(4) as usize);
            let text = targets
                .iter()
                .enumerate()
                .skip(offset)
                .map(|(i, e)| {
                    format!(
                        "{} {}",
                        if i == app.annotation_index { ">" } else { " " },
                        e.label
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            detail(
                frame,
                app,
                main,
                "Comment on element | Up/Down; Enter; Esc",
                "",
                &text,
            );
        }
        View::Annotations => {
            let current: std::collections::BTreeSet<_> = app
                .snapshot
                .comment_targets
                .iter()
                .map(|e| (&e.reference.id, &e.reference.version_hash))
                .collect();
            let comments: Vec<_> = app
                .snapshot
                .element_comments
                .iter()
                .filter(|c| !c.deleted)
                .collect();
            let selected = comments.get(app.annotation_index);
            let text = selected
                .map(|c| {
                    format!(
                        "{}/{} | {:?} {} | {} | {}\n{}\n\nReactions:\n{}",
                        app.annotation_index + 1,
                        comments.len(),
                        c.target.kind,
                        c.target.id,
                        if c.resolved { "resolved" } else { "unresolved" },
                        if current.contains(&(&c.target.id, &c.target.version_hash)) {
                            "current version"
                        } else {
                            "stale element version"
                        },
                        c.text,
                        serde_json::to_string_pretty(&c.reactions).unwrap_or_default()
                    )
                })
                .unwrap_or_else(|| "No element comments.".into());
            detail(
                frame,
                app,
                main,
                "Element comments | Up/Down; r resolve/reopen; d delete; Esc",
                "",
                &text,
            );
        }
        View::CreateRun => {
            detail(
                frame,
                app,
                main,
                "Create run | Enter / Ctrl+S create; Esc back",
                "",
                "Enter a SPEC.json path in the editor below.
The specification supplies objective, workspace, provider, requirements and permissions.
Creation does not start an agent or approve execution.
Your chat draft is retained. Relative paths use the launch directory.",
            );
        }
        View::Help => {
            let text = "Tab / Shift+Tab: switch panels (also switches a narrow terminal)\nTeam: arrows select nodes by direction in the graph\nEnter: agent details; [ / ] or Tab: detail tabs; Esc: back\nc: comment on any agent element; u: comment history and reactions\n/: search active panel; Enter finishes; Esc leaves the filter\ng: group by stage; r: graph relations; C: hide completed branches\ni: write orchestrator; m: write selected agent; b: broadcast\nEditor: arrows, Home/End, grapheme Backspace/Delete; Enter: newline\nCtrl+S / Ctrl+Enter: send; Esc: keep draft and leave editor\nChat: Up/Down, PageUp/PageDown scroll; / searches history\nCtrl+P: control actions, approvals, run details and run selection\nq outside editor / Ctrl+C: detach; daemon and run continue\nNO_COLOR=1 disables color; ARCHITECT_ASCII=1 uses ASCII display\n\nDelivery states are runtime observations, not animation.\nQueueing does not mean delivery. No hidden reasoning is shown.\n\nNew task: i, type a prompt, Ctrl+S starts an interview.\nNo SPEC.json is required. n optionally imports an existing spec.\nUse `antislop --help` for configuration and intake commands.";
            detail(frame, app, main, "Help | Esc back", "", text);
        }
        View::Palette => {
            let items = app.palette();
            let visible = main.height.saturating_sub(3) as usize;
            let offset = app.palette_index.saturating_sub(visible.saturating_sub(1));
            let text = items
                .iter()
                .enumerate()
                .skip(offset)
                .take(visible)
                .map(|(i, (label, _))| {
                    format!("{} {label}", if i == app.palette_index { ">" } else { " " })
                })
                .collect::<Vec<_>>()
                .join("\n");
            detail(
                frame,
                app,
                main,
                "Commands | Up/Down select; Enter; Esc",
                "",
                &text,
            );
        }
        View::Runs => {
            let visible = main.height.saturating_sub(3) as usize;
            let offset = app.run_index.saturating_sub(visible.saturating_sub(1));
            let text = if app.runs.is_empty() {
                "No runs. Esc, then n creates a run from SPEC.json.".into()
            } else {
                app.runs
                    .iter()
                    .enumerate()
                    .skip(offset)
                    .take(visible)
                    .map(|(i, r)| {
                        format!(
                            "{} {}  {}  {}",
                            if i == app.run_index { ">" } else { " " },
                            r.id,
                            r.stage,
                            r.status
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            detail(
                frame,
                app,
                main,
                "Select run | Enter attaches; Esc",
                "",
                &text,
            );
        }
        View::Confirm {
            action,
            label,
            details,
        } => {
            let scope = match &action {
                super::backend::Action::ApproveStart { scope_hash }
                | super::backend::Action::AcceptResult { scope_hash }
                | super::backend::Action::ApproveRepair { scope_hash }
                | super::backend::Action::Reconcile { scope_hash, .. }
                | super::backend::Action::Orchestrate { scope_hash }
                | super::backend::Action::AcceptIntake { scope_hash, .. }
                | super::backend::Action::AcceptTeam { scope_hash, .. }
                | super::backend::Action::StepTeam { scope_hash }
                | super::backend::Action::AcceptTeamStage { scope_hash, .. }
                | super::backend::Action::RetryTaskVerification { scope_hash, .. }
                | super::backend::Action::ReconcileWaveEffects { scope_hash, .. }
                | super::backend::Action::ReconcileTaskEffects { scope_hash, .. }
                | super::backend::Action::RestoreTaskResult { scope_hash, .. }
                | super::backend::Action::RefreshTaskSources { scope_hash, .. }
                | super::backend::Action::RepairTask { scope_hash, .. }
                | super::backend::Action::ReviewDiff { scope_hash } => scope_hash.as_str(),
                _ => "not applicable",
            };
            let text = format!(
                "Action: {label}\nScope hash: {scope}\n\n{details}\n\nPress Enter to submit this displayed scope to runtime validation.\nUp/Down or PageUp/PageDown scroll; Esc returns without submitting."
            );
            detail(frame, app, main, "Confirm control action", "", &text);
        }
    }
    // A direct message from an agent detail uses the same real editor, on top.
    if app.editing && !matches!(app.view, View::Main) {
        let editor_area = Rect::new(
            main.x,
            main.y + main.height.saturating_sub(7),
            main.width,
            main.height.min(7),
        );
        frame.render_widget(Clear, editor_area);
        editor(frame, app, editor_area);
    }
}
fn chat(frame: &mut Frame, app: &App, area: Rect) {
    let editor_height = if app.editing {
        area.height.min(7)
    } else {
        area.height.min(3)
    };
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(editor_height)])
        .split(area);
    let mut history = vec![];
    if !app.snapshot.diagnostic.is_empty() {
        history.push(format!("Runtime: {}", app.snapshot.diagnostic));
    }
    let query = app.chat_filter.to_lowercase();
    for message in &app.snapshot.messages {
        if query.is_empty()
            || format!("{} {} {}", message.sender, message.target, message.text)
                .to_lowercase()
                .contains(&query)
        {
            history.push(format!(
                "{} -> {} [{}] id:{}\n{}",
                message.sender, message.target, message.state, message.id, message.text
            ));
        }
    }
    if app.snapshot.reviewed_intake_proposal.is_some() {
        history.push("Reviewed task contract and agent routing ready. Esc leaves the editor; a opens the approval card. Enter on that card accepts the displayed scope and starts team planning/review.".into());
    }
    if app.snapshot.reviewed_team_round.is_some() {
        history.push("Reviewed agent team ready. Esc leaves the editor; a opens its roles, configurations and task plan for acceptance.".into());
    }
    if history.is_empty() {
        history.push(if app.snapshot.run_id.is_empty(){"Start a new task: i, type your request, Ctrl+S.\nThe orchestrator interviews you before planning execution.\nCtrl+P > Select run resumes an existing conversation.".into()}else{"No recorded conversation yet. i writes to the orchestrator.\nAdapter delivery: next-turn queue; receipt required before delivered.".into()});
    }
    let history = history.join("\n\n");
    let inner_height = parts[0].height.saturating_sub(2) as usize;
    let count = history.lines().count();
    let offset = count
        .saturating_sub(inner_height)
        .saturating_sub(app.chat_scroll);
    let title = if app.searching && app.focus == Focus::Chat {
        format!("Orchestrator | Search: {}_", app.chat_filter)
    } else {
        format!("Orchestrator | filter: {}", app.chat_filter)
    };
    frame.render_widget(
        Paragraph::new(lines_view(app, &history, offset, inner_height)).block(block(
            app,
            title,
            app.focus == Focus::Chat,
        )),
        parts[0],
    );
    editor(frame, app, parts[1]);
}
fn editor(frame: &mut Frame, app: &App, area: Rect) {
    let title = if app.view == View::CreateRun {
        "SPEC.json path | Enter / Ctrl+S create | Esc back".into()
    } else if let Some(target) = &app.comment_target {
        format!(
            "Comment {:?} {} | Ctrl+S save / Esc leave",
            target.kind, target.id
        )
    } else if app.editing
        && (app.snapshot.run_id.is_empty() || app.snapshot.intake || app.snapshot.chat_origin)
    {
        "To orchestrator | Ctrl+S starts reply | Esc keeps draft".into()
    } else if app.editing {
        format!(
            "To {} | next-turn queue | Ctrl+S send / Esc leave",
            app.message_target
        )
    } else {
        "i: write orchestrator | b: broadcast | draft retained".into()
    };
    let border = block(app, title, app.editing);
    let inner = border.inner(area);
    let (text, x, y) = app
        .active_editor()
        .viewport(inner.width as usize, inner.height as usize);
    frame.render_widget(Paragraph::new(clean(app, &text)).block(border), area);
    if app.editing && inner.width > 0 && inner.height > 0 {
        frame.set_cursor_position((inner.x + x, inner.y + y));
    }
}
fn team(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = format!(
        "Team: graph{} | / {}{}",
        if app.relations { " / relations" } else { "" },
        app.team_filter,
        if app.searching && app.focus == Focus::Team {
            "_"
        } else {
            ""
        }
    );
    let border = block(app, title, app.focus == Focus::Team);
    let inner = border.inner(area);
    frame.render_widget(border, area);
    app.graph.update(&app.snapshot.agents, app.grouped);
    let visible = app.nodes().into_iter().map(|n| n.id).collect();
    let overlay_height = inner.height.min(5);
    let graph_area = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(overlay_height),
    );
    frame.render_widget(
        super::graph::Graph {
            app,
            visible: &visible,
        },
        graph_area,
    );
    if graph_area.is_empty() {
        return;
    }
    let active: Vec<_> = app
        .snapshot
        .agents
        .iter()
        .filter(|a| {
            app.snapshot.status == "running"
                && matches!(
                    a.status.as_str(),
                    "running" | "verifying" | "starting" | "sent" | "pending" | "pending_unknown"
                )
        })
        .collect();
    let discovering = app.snapshot.status == "running"
        && matches!(app.snapshot.discovery_status.as_str(), "running" | "queued");
    let progress = if discovering {
        if app.snapshot.discovery_status == "running" {
            "Discover CLI capabilities"
        } else {
            "CLI capability discovery queued"
        }
        .into()
    } else if app.snapshot.status == "running" && !app.snapshot.running_checks.is_empty() {
        app.snapshot
            .running_checks
            .iter()
            .map(|(task, check)| format!("{task}: check {check}"))
            .collect::<Vec<_>>()
            .join("\n")
    } else if active.is_empty()
        || matches!(
            app.snapshot.status.as_str(),
            "paused" | "blocked" | "cancelled" | "completed" | "changes_requested"
        )
    {
        format!("{}: {}", app.snapshot.stage, app.snapshot.status)
    } else {
        active
            .iter()
            .map(|a| format!("{} / {}: {}", a.group, a.role, a.task))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let width = inner.width.min(42);
    let overlay = Rect::new(
        inner.right() - width,
        inner.bottom() - overlay_height,
        width,
        overlay_height,
    );
    frame.render_widget(Clear, overlay);
    let progress_title = if discovering {
        "Current step | CLI discovery".into()
    } else if app.snapshot.status == "running" {
        format!(
            "Current step | {} active",
            active.len() + app.snapshot.running_checks.len()
        )
    } else {
        format!("Current step | {}", app.snapshot.status)
    };
    frame.render_widget(
        Paragraph::new(clean(app, &progress))
            .wrap(Wrap { trim: false })
            .block(block(app, progress_title, false)),
        overlay,
    );
}

fn detail(frame: &mut Frame, app: &App, area: Rect, title: &str, tabs: &str, text: &str) {
    frame.render_widget(Clear, area);
    let border = block(app, clean(app, title), true);
    let inner = border.inner(area);
    frame.render_widget(border, area);
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(if tabs.is_empty() { 0 } else { 2 }),
            Constraint::Min(1),
        ])
        .split(inner);
    frame.render_widget(
        Paragraph::new(clean(app, tabs)).wrap(Wrap { trim: false }),
        parts[0],
    );
    frame.render_widget(
        Paragraph::new(lines_view(app, text, app.scroll, parts[1].height as usize)),
        parts[1],
    );
}

```
