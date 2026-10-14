use super::{
    app::{App, Focus, View},
    model::{AGENT_TABS, RUN_TABS},
    theme::{Row, Theme, draw_rows, split_row, wrap},
};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::Span,
    widgets::{Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

fn lines_view(theme: &Theme, text: &str, offset: usize, height: usize) -> String {
    // The viewport creates strings only for rendered lines, even for large reports.
    text.lines()
        .skip(offset)
        .take(height)
        .map(|line| theme.clean(line))
        .collect::<Vec<_>>()
        .join("\n")
}
fn styled(text: impl Into<String>, style: Style) -> Span<'static> {
    Span::styled(text.into(), style)
}
/// Shrinks an area horizontally; never underflows on tiny terminals.
fn inset(area: Rect, left: u16, right: u16) -> Rect {
    let left = left.min(area.width);
    Rect::new(
        area.x + left,
        area.y,
        area.width.saturating_sub(left + right),
        area.height,
    )
}
fn vsplit(area: Rect, heights: &[Constraint]) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints(heights.to_vec())
        .split(area)
        .to_vec()
}
/// A thin scroll indicator; nothing is drawn when everything fits.
fn scrollbar(buf: &mut Buffer, theme: &Theme, area: Rect, total: usize, top: usize, view: usize) {
    let height = area.height as usize;
    if area.width == 0 || height == 0 || total <= view {
        return;
    }
    let thumb = (view * height / total).clamp(1, height);
    let start = top.min(total - view) * (height - thumb) / (total - view);
    let glyphs = theme.glyphs();
    for y in 0..height {
        let (symbol, style) = if (start..start + thumb).contains(&y) {
            (glyphs.thumb, theme.dim())
        } else {
            (glyphs.track, theme.faint())
        };
        buf[(area.x, area.y + y as u16)]
            .set_symbol(symbol)
            .set_style(style);
    }
}
/// Section headings replace panel borders.
fn section(theme: &Theme, title: &str, meta: String, focused: bool, width: u16) -> Row {
    let title_style = if focused {
        theme.accent()
    } else {
        theme.title()
    };
    let marker = if focused { theme.glyphs().pointer } else { " " };
    split_row(
        Style::default(),
        vec![
            styled(marker, theme.accent()),
            styled(theme.clean(title), title_style),
        ],
        styled(
            theme.fit(&theme.clean(&meta), width as usize / 2),
            theme.dim(),
        ),
        width as usize,
    )
}
/// A card: a colored accent bar, a heading row, wrapped body rows and a gap.
#[allow(clippy::too_many_arguments)]
fn card(
    theme: &Theme,
    fill: Style,
    accent: Style,
    head: Vec<Span<'static>>,
    right: String,
    body: &str,
    max_body: usize,
    width: u16,
) -> Vec<Row> {
    let width = width as usize;
    let bar = || styled(theme.glyphs().bar, fill.patch(accent));
    let mut left = vec![bar(), styled(" ", fill)];
    left.extend(
        head.into_iter()
            .map(|span| styled(span.content.into_owned(), fill.patch(span.style))),
    );
    let mut out = vec![split_row(
        fill,
        left,
        styled(theme.fit(&right, width / 2), fill.patch(theme.dim())),
        width,
    )];
    let room = width.saturating_sub(3);
    let lines = if body.is_empty() {
        vec![]
    } else {
        wrap(&theme.clean(body), room)
    };
    let cut = lines.len() > max_body;
    for (i, line) in lines.iter().take(max_body).enumerate() {
        let line = if cut && i + 1 == max_body {
            theme.fit(&format!("{line}{}", theme.glyphs().ellipsis), room)
        } else {
            line.clone()
        };
        out.push(Row::new(
            fill,
            vec![bar(), styled(" ", fill), styled(line, fill)],
        ));
    }
    out.push(Row::gap());
    out
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    app.width = area.width;
    app.height = area.height;
    if area.width < 12 || area.height < 5 {
        frame.render_widget(Paragraph::new("Resize terminal. q detaches."), area);
        return;
    }
    let theme = Theme::new(!app.no_color, app.ascii);
    let parts = vsplit(
        area,
        &[
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(2),
        ],
    );
    header(frame, app, &theme, parts[0]);
    let main = parts[1];
    if main.width >= 85 {
        // The chat takes a third; the team graph gets the rest.
        let chat_width = (main.width / 3).max(34);
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(chat_width),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(main);
        chat(frame, app, &theme, columns[0]);
        // The only rule lines separate the main regions.
        let separator = theme.glyphs().vertical;
        let buf = frame.buffer_mut();
        for y in columns[1].top()..columns[1].bottom() {
            buf[(columns[1].x, y)]
                .set_symbol(separator)
                .set_style(theme.separator(false));
        }
        team(frame, app, &theme, columns[2]);
    } else if app.focus == Focus::Chat {
        chat(frame, app, &theme, main);
    } else {
        team(frame, app, &theme, main);
    }
    footer(frame, app, &theme, parts[2]);
    match app.view.clone() {
        View::Main => (),
        View::Agent { tab } => {
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
                &theme,
                main,
                &format!("{title} | [ ] tabs · Esc back"),
                &AGENT_TABS,
                tab,
                &text,
            );
        }
        View::Run { tab } => {
            let text = app
                .snapshot
                .run_details
                .get(RUN_TABS[tab])
                .cloned()
                .unwrap_or_else(|| "No recorded data available.".into());
            detail(
                frame,
                app,
                &theme,
                main,
                "Run details | [ ] tabs · Esc back",
                &RUN_TABS,
                tab,
                &text,
            );
        }
        View::CommentTargets => {
            let items = app
                .annotation_targets
                .iter()
                .map(|e| e.label.clone())
                .collect();
            list(
                frame,
                &theme,
                main,
                "Comment on element | Up/Down · Enter · Esc",
                items,
                app.annotation_index,
                "No commentable elements.",
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
                &theme,
                main,
                "Element comments | Up/Down · r resolve/reopen · d delete · Esc",
                &[],
                0,
                &text,
            );
        }
        View::CreateRun => {
            detail(
                frame,
                app,
                &theme,
                main,
                "Create run | Enter / Ctrl+S create · Esc back",
                &[],
                0,
                "Enter a SPEC.json path in the editor below.
The specification supplies objective, workspace, provider, requirements and permissions.
Creation does not start an agent or approve execution.
Your chat draft is retained. Relative paths use the launch directory.",
            );
        }
        View::Help => {
            let text = "Tab / Shift+Tab: cycle chat, team graph and task carousel (also on a narrow terminal)\nTeam: arrows select nodes by direction in the graph\nTasks: checklist carousel over the graph; [x] done, [*] in progress, [ ] to do\nTasks: Up/Down, PageUp/PageDown, Home/End turn it; the graph follows the owner\nTasks: Enter opens the owner; unfocused, it follows the active task\nEnter: agent details; [ / ] or Tab: detail tabs; Esc: back\nc: comment on any agent element; u: comment history and reactions\n/: search active panel; Enter finishes; Esc leaves the filter\ng: group by stage; r: graph relations; C: hide completed branches\ni: write orchestrator; m: write selected agent; b: broadcast\nEditor: arrows, Home/End, grapheme Backspace/Delete; Enter: newline\nCtrl+S / Ctrl+Enter: send; Esc: keep draft and leave editor\nChat: Up/Down, PageUp/PageDown scroll; / searches history\nCtrl+P: control actions, approvals, run details and run selection\nq outside editor / Ctrl+C: detach; daemon and run continue\nNO_COLOR=1 disables color; ARCHITECT_ASCII=1 uses ASCII display\n\nDelivery states are runtime observations, not animation.\nQueueing does not mean delivery. No hidden reasoning is shown.\n\nNew task: i, type a prompt, Ctrl+S starts an interview.\nNo SPEC.json is required. n optionally imports an existing spec.\nUse `antislop --help` for configuration and intake commands.";
            detail(frame, app, &theme, main, "Help | Esc back", &[], 0, text);
        }
        View::Palette => {
            let items = app.palette().into_iter().map(|(label, _)| label).collect();
            list(
                frame,
                &theme,
                main,
                "Commands | Up/Down select · Enter · Esc",
                items,
                app.palette_index,
                "No commands available.",
            );
        }
        View::Runs => {
            let items = app
                .runs
                .iter()
                .map(|r| format!("{}  {}  {}", r.id, r.stage, r.status))
                .collect();
            list(
                frame,
                &theme,
                main,
                "Select run | Enter attaches · Esc",
                items,
                app.run_index,
                "No runs. Esc, then n creates a run from SPEC.json.",
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
                | super::backend::Action::RecoverOrchestration { scope_hash, .. }
                | super::backend::Action::AcceptIntake { scope_hash, .. }
                | super::backend::Action::AcceptTeam { scope_hash, .. }
                | super::backend::Action::StepTeam { scope_hash }
                | super::backend::Action::AcceptTeamStage { scope_hash, .. }
                | super::backend::Action::RetryTaskVerification { scope_hash, .. }
                | super::backend::Action::ResumeTaskSession { scope_hash, .. }
                | super::backend::Action::ReplaceTaskSession { scope_hash, .. }
                | super::backend::Action::ReconcileWaveEffects { scope_hash, .. }
                | super::backend::Action::ReconcileTaskEffects { scope_hash, .. }
                | super::backend::Action::RestoreTaskResult { scope_hash, .. }
                | super::backend::Action::RestoreTaskCheckResult { scope_hash, .. }
                | super::backend::Action::RefreshTaskSources { scope_hash, .. }
                | super::backend::Action::RepairTask { scope_hash, .. }
                | super::backend::Action::ReviewDiff { scope_hash } => scope_hash.as_str(),
                _ => "not applicable",
            };
            let text = format!(
                "Action: {label}\nScope hash: {scope}\n\n{details}\n\nPress Enter to submit this displayed scope to runtime validation.\nUp/Down or PageUp/PageDown scroll; Esc returns without submitting."
            );
            detail(
                frame,
                app,
                &theme,
                main,
                "Confirm control action",
                &[],
                0,
                &text,
            );
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
        editor(frame, app, &theme, editor_area);
    }
}

fn header(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let fill = theme.bar();
    let snapshot = &app.snapshot;
    let glyphs = theme.glyphs();
    let dot = format!(" {} ", glyphs.dot);
    let run = if snapshot.run_id.is_empty() {
        "no run".to_string()
    } else {
        theme.clean(&snapshot.run_id)
    };
    let mut left = vec![
        styled(" ", fill),
        styled(glyphs.logo, fill.patch(theme.accent())),
        styled(" Antislop  ", fill.patch(theme.title())),
        styled(run, fill.patch(theme.text())),
    ];
    for part in [&snapshot.mode, &snapshot.stage] {
        if !part.is_empty() {
            left.push(styled(dot.clone(), fill.patch(theme.faint())));
            left.push(styled(theme.clean(part), fill.patch(theme.dim())));
        }
    }
    if !snapshot.status.is_empty() {
        let status = fill.patch(theme.status(&snapshot.status));
        left.push(styled("  ", fill));
        left.push(styled(theme.status_glyph(&snapshot.status), status));
        left.push(styled(
            format!(" {}", theme.clean(&snapshot.status)),
            status,
        ));
    }
    let used: usize = left.iter().map(|s| s.content.width()).sum();
    let room = (area.width as usize).saturating_sub(used + 3);
    let row = split_row(
        fill,
        left,
        styled(
            format!("{} ", theme.fit(&theme.clean(&snapshot.workspace), room)),
            fill.patch(theme.dim()),
        ),
        area.width as usize,
    );
    draw_rows(frame.buffer_mut(), area, [&row]);
}

fn footer(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let hints: &[(&str, &str)] = if app.editing {
        &[
            ("Ctrl+S", "send"),
            ("Enter", "newline"),
            ("Esc", "keep draft"),
            ("Tab", "leave"),
        ]
    } else if app.view != View::Main {
        &[
            ("Esc", "back"),
            ("↑↓", "scroll"),
            ("Enter", "select"),
            ("?", "help"),
        ]
    } else {
        match app.focus {
            Focus::Chat => &[
                ("Tab", "focus"),
                ("i", "write"),
                ("b", "broadcast"),
                ("↑↓", "scroll"),
                ("/", "search"),
                ("a", "approve"),
                ("Ctrl+P", "commands"),
                ("?", "help"),
                ("q", "detach"),
            ],
            Focus::Team => &[
                ("Tab", "focus"),
                ("←↑↓→", "move"),
                ("Enter", "inspect"),
                ("m", "message"),
                ("c", "comment"),
                ("u", "comments"),
                ("g", "group"),
                ("r", "relations"),
                ("/", "search"),
                ("Ctrl+P", "commands"),
                ("q", "detach"),
            ],
            Focus::Tasks => &[
                ("Tab", "focus"),
                ("↑↓", "select"),
                ("PgUp/PgDn", "page"),
                ("Enter", "open owner"),
                ("Ctrl+P", "commands"),
                ("?", "help"),
                ("q", "detach"),
            ],
        }
    };
    let fill = theme.bar();
    let mut spans = vec![styled(" ", fill)];
    let mut used = 1;
    for (key, label) in hints {
        let key = if theme.ascii {
            key.replace("←↑↓→", "arrows").replace("↑↓", "Up/Down")
        } else {
            key.to_string()
        };
        let width = key.width() + label.width() + 3;
        if used + width > area.width as usize {
            break;
        }
        spans.push(styled(key, fill.patch(theme.accent())));
        spans.push(styled(format!(" {label}  "), fill.patch(theme.dim())));
        used += width;
    }
    let notice = Row::new(
        Style::default(),
        vec![styled(
            format!(
                " {}",
                theme.fit(
                    &theme.clean(&app.notice).replace('\n', " "),
                    (area.width as usize).saturating_sub(2)
                )
            ),
            if app.notice.contains("rejected") || app.notice.contains("unavailable") {
                theme.error()
            } else {
                theme.dim()
            },
        )],
    );
    draw_rows(frame.buffer_mut(), area, [&notice, &Row::new(fill, spans)]);
}

fn chat(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let editor_height = if app.editing {
        area.height.min(7)
    } else {
        area.height.min(3)
    };
    let parts = vsplit(
        area,
        &[
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(editor_height),
        ],
    );
    let body = inset(parts[1], 1, 2);
    let width = body.width;
    let glyphs = theme.glyphs();
    let mut cards: Vec<Row> = vec![];
    if !app.snapshot.diagnostic.is_empty() {
        cards.extend(card(
            theme,
            theme.card(),
            theme.error(),
            vec![styled("Runtime", theme.error())],
            String::new(),
            &app.snapshot.diagnostic,
            usize::MAX,
            width,
        ));
    }
    let query = app.chat_filter.to_lowercase();
    for message in &app.snapshot.messages {
        if !query.is_empty()
            && !format!("{} {} {}", message.sender, message.target, message.text)
                .to_lowercase()
                .contains(&query)
        {
            continue;
        }
        let fill = if message.sender == "user" {
            theme.raised()
        } else {
            theme.card()
        };
        cards.extend(card(
            theme,
            fill,
            theme.person(&message.sender),
            vec![
                styled(
                    theme.clean(&message.sender),
                    theme.person(&message.sender).patch(theme.title()),
                ),
                styled(format!(" {} ", glyphs.to), theme.faint()),
                styled(theme.clean(&message.target), theme.person(&message.target)),
            ],
            format!("{} {} {}", message.state, glyphs.dot, message.id),
            &message.text,
            usize::MAX,
            width,
        ));
    }
    let mut notice = |text: &str| {
        cards.extend(card(
            theme,
            theme.raised(),
            theme.warning(),
            vec![styled("Action needed", theme.warning())],
            String::new(),
            text,
            usize::MAX,
            width,
        ))
    };
    if app.snapshot.awaiting_user_reply.is_some() {
        notice(
            "Orkiestrator czeka na Twoją odpowiedź. Odpowiedz na pytania powyżej w jednej wiadomości. Enter lub i otwiera pole odpowiedzi; Ctrl+S wysyła. Po wywiadzie zobaczysz propozycję zadania do akceptacji.",
        );
    }
    if app.snapshot.reviewed_intake_proposal.is_some() {
        notice(
            "Reviewed task contract and agent routing ready. Esc leaves the editor; a opens the approval card. Enter on that card accepts the displayed scope and starts team planning/review.",
        );
    }
    if app.snapshot.reviewed_team_round.is_some() {
        notice(
            "Reviewed agent team ready. Esc leaves the editor; a opens its roles, configurations and task plan for acceptance.",
        );
    }
    if cards.is_empty() {
        cards.extend(card(
            theme,
            theme.card(),
            theme.accent(),
            vec![styled(
                if app.snapshot.run_id.is_empty() {
                    "Start a new task"
                } else {
                    "No recorded conversation yet"
                },
                theme.title(),
            )],
            String::new(),
            if app.snapshot.run_id.is_empty() {
                "i, type your request, Ctrl+S.\nThe orchestrator interviews you before planning execution.\nCtrl+P > Select run resumes an existing conversation."
            } else {
                "i writes to the orchestrator.\nAdapter delivery: next-turn queue; receipt required before delivered."
            },
            usize::MAX,
            width,
        ));
    }
    let view = body.height as usize;
    let total = cards.len();
    let top = total.saturating_sub(view).saturating_sub(app.chat_scroll);
    let below = total.saturating_sub(top + view);
    let mut meta = if app.searching && app.focus == Focus::Chat {
        format!("Search: {}_", app.chat_filter)
    } else if !app.chat_filter.is_empty() {
        format!("filter: {}", app.chat_filter)
    } else {
        String::new()
    };
    if below > 0 {
        meta = format!(
            "{meta}  {} {below} lines",
            if theme.ascii { "v" } else { "↓" }
        );
    }
    if app.snapshot.awaiting_user_reply.is_some() && meta.is_empty() {
        meta = "Czeka na Ciebie".into();
    }
    let heading = section(
        theme,
        "Orchestrator",
        meta,
        app.focus == Focus::Chat,
        parts[0].width,
    );
    let buf = frame.buffer_mut();
    draw_rows(buf, parts[0], [&heading]);
    draw_rows(buf, body, &cards[top..]);
    scrollbar(
        buf,
        theme,
        Rect::new(body.right() + 1, body.y, 1, body.height).intersection(parts[1]),
        total,
        top,
        view,
    );
    editor(frame, app, theme, parts[2]);
}

fn editor(frame: &mut Frame, app: &App, theme: &Theme, area: Rect) {
    let title = if app.view == View::CreateRun {
        "SPEC.json path | Enter / Ctrl+S create | Esc back".into()
    } else if let Some(target) = &app.comment_target {
        format!(
            "Comment {:?} {} | Ctrl+S save / Esc leave",
            target.kind, target.id
        )
    } else if app.snapshot.awaiting_user_reply.is_some() && app.message_target == "orchestrator" {
        if app.editing {
            "Twoja odpowiedź | Ctrl+S wyślij | Esc zachowaj szkic".into()
        } else {
            "Czeka na odpowiedź | Enter / i: odpowiedz | Ctrl+S: wyślij".into()
        }
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
    let card = inset(area, 1, 1);
    if card.is_empty() {
        return;
    }
    let fill = if app.editing {
        theme.raised()
    } else {
        theme.card()
    };
    let accent = if app.editing {
        theme.accent()
    } else {
        theme.faint()
    };
    let bar = theme.glyphs().bar;
    let mut rows = vec![Row::new(
        fill,
        vec![
            styled(bar, fill.patch(accent)),
            styled(" ", fill),
            styled(
                theme.fit(
                    &theme.clean(&title),
                    (card.width as usize).saturating_sub(3),
                ),
                fill.patch(if app.editing {
                    theme.accent()
                } else {
                    theme.dim()
                }),
            ),
        ],
    )];
    rows.extend((1..card.height).map(|_| Row::new(fill, vec![styled(bar, fill.patch(accent))])));
    draw_rows(frame.buffer_mut(), card, &rows);
    let inner = Rect::new(
        card.x + 2,
        card.y + 1,
        card.width.saturating_sub(3),
        card.height.saturating_sub(1),
    );
    let (text, x, y) = app
        .active_editor()
        .viewport(inner.width as usize, inner.height as usize);
    frame.render_widget(
        Paragraph::new(theme.clean(&text)).style(fill.patch(theme.text())),
        inner,
    );
    if app.editing && inner.width > 0 && inner.height > 0 {
        frame.set_cursor_position((inner.x + x, inner.y + y));
    }
}

fn team(frame: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    let parts = vsplit(area, &[Constraint::Length(1), Constraint::Min(0)]);
    let filter = if app.searching && app.focus == Focus::Team {
        format!("/ {}_", app.team_filter)
    } else if !app.team_filter.is_empty() {
        format!("/ {}", app.team_filter)
    } else {
        String::new()
    };
    let title = format!(
        "Team: graph{}",
        if app.relations { " / relations" } else { "" }
    );
    let mut heading = section(theme, &title, filter, app.focus == Focus::Team, area.width);
    // Legend: edge kinds are also labelled on the graph for NO_COLOR.
    let used = heading.line.width();
    if app.team_filter.is_empty() && !app.searching && area.width as usize > used + 44 {
        let line = theme.glyphs().horizontal.repeat(2);
        let mut legend = vec![];
        for (kind, label) in [
            ("supervises", "supervises"),
            ("verifies", "V verifies"),
            ("depends on", "D depends"),
            ("communicates with", "M messages"),
        ] {
            if kind == "communicates with" && !app.relations {
                continue;
            }
            legend.push(styled(line.clone(), theme.edge(kind)));
            legend.push(styled(format!(" {label}  "), theme.faint()));
        }
        let width: usize = legend.iter().map(|s| s.content.width()).sum();
        heading.line.spans.push(styled(
            " ".repeat((area.width as usize).saturating_sub(used + width)),
            Style::default(),
        ));
        heading.line.spans.extend(legend);
    }
    draw_rows(frame.buffer_mut(), parts[0], [&heading]);
    app.graph.update(&app.snapshot.agents, app.grouped);
    let visible = app.nodes().into_iter().map(|n| n.id).collect();
    let graph_area = inset(parts[1], 1, 0);
    if graph_area.is_empty() {
        return;
    }
    let (overlay, rows) = carousel(app, theme, graph_area);
    frame.render_widget(
        super::graph::Graph {
            app,
            theme,
            visible: &visible,
            avoid: overlay,
        },
        graph_area,
    );
    frame.render_widget(Clear, overlay);
    draw_rows(frame.buffer_mut(), overlay, &rows);
}

/// The observed current step: discovery, running checks or active agents.
fn progress(app: &App) -> (String, Vec<String>) {
    let snapshot = &app.snapshot;
    let active: Vec<_> = snapshot
        .agents
        .iter()
        .filter(|a| {
            snapshot.status == "running"
                && matches!(
                    a.status.as_str(),
                    "running" | "verifying" | "starting" | "sent" | "pending" | "pending_unknown"
                )
        })
        .collect();
    let discovering = snapshot.status == "running"
        && matches!(snapshot.discovery_status.as_str(), "running" | "queued");
    let lines = if snapshot.awaiting_user_reply.is_some() {
        vec![
            "Wywiad: odpowiedz na pytania w czacie".into(),
            "Enter / i: odpowiedź; Ctrl+S: wyślij".into(),
        ]
    } else if discovering {
        vec![
            if snapshot.discovery_status == "running" {
                "Discover CLI capabilities"
            } else {
                "CLI capability discovery queued"
            }
            .into(),
        ]
    } else if snapshot.status == "running" && !snapshot.running_checks.is_empty() {
        snapshot
            .running_checks
            .iter()
            .map(|(task, check)| format!("{task}: check {check}"))
            .collect()
    } else if active.is_empty()
        || matches!(
            snapshot.status.as_str(),
            "paused" | "blocked" | "cancelled" | "completed" | "changes_requested"
        )
    {
        vec![format!("{}: {}", snapshot.stage, snapshot.status)]
    } else {
        active
            .iter()
            .map(|a| format!("{} / {}: {}", a.group, a.role, a.task))
            .collect()
    };
    let title = if snapshot.awaiting_user_reply.is_some() {
        "Czeka na Twoją odpowiedź".into()
    } else if discovering {
        "Current step | CLI discovery".into()
    } else if snapshot.status == "running" {
        format!(
            "Current step | {} active",
            active.len() + snapshot.running_checks.len()
        )
    } else {
        format!("Current step | {}", snapshot.status)
    };
    (title, lines)
}

/// The task checklist overlay in the graph's lower-right corner. It is a
/// carousel: the followed task stays centered and neighbours fade with
/// distance; a summary counts done, in-progress and open tasks.
fn carousel(app: &App, theme: &Theme, area: Rect) -> (Rect, Vec<Row>) {
    let glyphs = theme.glyphs();
    let width = area.width.min(60);
    let w = width as usize;
    let room = w.saturating_sub(3);
    let max_height = if area.height >= 14 {
        area.height as usize * 3 / 5
    } else {
        area.height as usize
    };
    let fill = theme.panel();
    let tasks = &app.snapshot.tasks;
    let focused = app.focus == Focus::Tasks;
    let follow = app.followed_task();
    let bar = |style: Style| styled(glyphs.bar, fill.patch(style));
    let (title, lines) = progress(app);
    let status = theme.status(&app.snapshot.status);
    let mut rows = vec![split_row(
        fill,
        vec![
            bar(status),
            styled(" ", fill),
            styled(
                theme.fit(&title, room.saturating_sub(8)),
                fill.patch(if focused {
                    theme.accent()
                } else {
                    theme.title()
                }),
            ),
        ],
        styled(
            if tasks.is_empty() {
                String::new()
            } else {
                format!("{}/{} ", follow + 1, tasks.len())
            },
            fill.patch(theme.dim()),
        ),
        w,
    )];
    let step_lines = if max_height >= 10 { 2 } else { 1 };
    for line in lines.iter().take(step_lines) {
        rows.push(Row::new(
            fill,
            vec![
                bar(status),
                styled(" ", fill),
                styled(
                    theme.fit(&theme.clean(line), room.saturating_sub(1)),
                    fill.patch(theme.dim()),
                ),
            ],
        ));
    }
    rows.push(Row::new(fill, vec![bar(theme.faint())]));
    let task_row = |i: usize, distance: usize| -> Vec<Row> {
        let task = &tasks[i];
        let tone = theme.status(&task.state);
        let center = distance == 0;
        let fill = if center && focused {
            theme.selected()
        } else if center {
            theme.raised()
        } else {
            fill
        };
        let fade = match distance {
            0 => fill.patch(theme.title()),
            1 => fill.patch(theme.text()),
            2 => fill.patch(theme.dim()),
            _ => fill.patch(theme.faint()),
        };
        let row = vec![
            styled(glyphs.bar, fill.patch(tone)),
            styled(" ", fill),
            styled(
                if center { glyphs.pointer } else { " " },
                fill.patch(theme.accent()),
            ),
            styled(" ", fill),
            styled(theme.checkbox(&task.state), fill.patch(tone)),
            styled(" ", fill),
            styled(
                theme.fit(
                    &theme.clean(&task.description).replace('\n', " "),
                    room.saturating_sub(8),
                ),
                fade,
            ),
        ];
        let mut out = vec![Row::new(fill, row)];
        if center {
            let mut meta = vec![theme.clean(&task.state)];
            if !task.owner.is_empty() {
                let mut people = theme.clean(&task.owner);
                if !task.verifier.is_empty() {
                    people = format!("{people} {} {}", glyphs.to, theme.clean(&task.verifier));
                }
                meta.push(people);
            }
            meta.push(theme.clean(&task.id));
            if !task.stage.is_empty() {
                meta.push(theme.clean(&task.stage));
            }
            out.push(Row::new(
                fill,
                vec![
                    styled(glyphs.bar, fill.patch(tone)),
                    styled("       ", fill),
                    styled(
                        theme.fit(
                            &meta.join(&format!(" {} ", glyphs.dot)),
                            room.saturating_sub(7),
                        ),
                        if focused {
                            fill
                        } else {
                            fill.patch(theme.dim())
                        },
                    ),
                ],
            ));
        }
        out
    };
    if tasks.is_empty() {
        rows.push(Row::new(
            fill,
            vec![
                bar(theme.faint()),
                styled(" ", fill),
                styled(
                    theme.fit(
                        "No tasks yet. The plan appears once the team is accepted.",
                        room.saturating_sub(1),
                    ),
                    fill.patch(theme.dim()),
                ),
            ],
        ));
    } else {
        // At most two previous tasks, the followed one and two next ones,
        // within the rows left after the meta and summary rows.
        let window = max_height.saturating_sub(rows.len() + 2).clamp(1, 5);
        if tasks.len() <= window {
            for i in 0..tasks.len() {
                rows.extend(task_row(i, i.abs_diff(follow)));
            }
        } else {
            // Picker-style: the center is fixed, so the ends pad with blanks.
            let side = window.saturating_sub(1) / 2;
            for offset in 0..=side * 2 {
                let distance = offset.abs_diff(side);
                match (follow + offset).checked_sub(side) {
                    Some(i) if i < tasks.len() => rows.extend(task_row(i, distance)),
                    _ => rows.push(Row::new(fill, vec![bar(theme.faint())])),
                }
            }
        }
        let count = |tone: super::theme::Tone| {
            tasks
                .iter()
                .filter(|t| super::theme::tone(&t.state) == tone)
                .count()
        };
        use super::theme::Tone;
        let mut summary = vec![bar(theme.faint()), styled(" ", fill)];
        for (n, glyph, label, style) in [
            (
                count(Tone::Done),
                glyphs.done,
                "done",
                theme.status("completed"),
            ),
            (
                count(Tone::Active) + count(Tone::Waiting),
                glyphs.active,
                "in progress",
                theme.status("running"),
            ),
            (
                count(Tone::Queued),
                glyphs.queued,
                "to do",
                theme.status("queued"),
            ),
            (
                count(Tone::Failed),
                glyphs.failed,
                "failed",
                theme.status("failed"),
            ),
        ] {
            if n == 0 && label == "failed" {
                continue;
            }
            summary.push(styled(format!("{glyph} {n}"), fill.patch(style)));
            summary.push(styled(format!(" {label}   "), fill.patch(theme.dim())));
        }
        rows.push(Row::new(fill, summary));
    }
    let height = (rows.len() as u16).min(area.height);
    let overlay = Rect::new(area.right() - width, area.bottom() - height, width, height);
    (overlay, rows)
}

fn title_row(theme: &Theme, title: &str, width: u16) -> Row {
    let fill = theme.overlay();
    let (name, hint) = title.split_once(" | ").unwrap_or((title, ""));
    split_row(
        fill,
        vec![
            styled(" ", fill),
            styled(
                theme.fit(&theme.clean(name), width as usize / 2),
                fill.patch(theme.accent()),
            ),
        ],
        styled(
            format!("{} ", theme.fit(&theme.clean(hint), width as usize / 2)),
            fill.patch(theme.dim()),
        ),
        width as usize,
    )
}

#[allow(clippy::too_many_arguments)]
fn detail(
    frame: &mut Frame,
    app: &App,
    theme: &Theme,
    area: Rect,
    title: &str,
    tabs: &[&str],
    tab: usize,
    text: &str,
) {
    frame.render_widget(Clear, area);
    let fill = theme.overlay();
    frame.buffer_mut().set_style(area, fill);
    let mut header = vec![title_row(theme, title, area.width)];
    if !tabs.is_empty() {
        let mut spans = vec![styled(" ", fill)];
        for (i, name) in tabs.iter().enumerate() {
            let style = if i == tab {
                theme.selected()
            } else {
                fill.patch(theme.dim())
            };
            spans.push(styled(format!(" {name} "), style));
            spans.push(styled(" ", fill));
        }
        header.push(Row::new(fill, spans));
    }
    header.push(Row::new(fill, vec![]));
    let parts = vsplit(
        area,
        &[Constraint::Length(header.len() as u16), Constraint::Min(1)],
    );
    draw_rows(frame.buffer_mut(), parts[0], &header);
    let body = inset(parts[1], 2, 2);
    frame.render_widget(
        Paragraph::new(lines_view(theme, text, app.scroll, body.height as usize))
            .style(fill.patch(theme.text())),
        body,
    );
}

fn list(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    title: &str,
    items: Vec<String>,
    selected: usize,
    empty: &str,
) {
    frame.render_widget(Clear, area);
    let fill = theme.overlay();
    frame.buffer_mut().set_style(area, fill);
    let parts = vsplit(area, &[Constraint::Length(2), Constraint::Min(1)]);
    draw_rows(
        frame.buffer_mut(),
        parts[0],
        [&title_row(theme, title, area.width)],
    );
    let body = inset(parts[1], 1, 2);
    let view = body.height as usize;
    let offset = selected.saturating_sub(view.saturating_sub(1));
    let width = body.width as usize;
    let mut rows: Vec<Row> = items
        .iter()
        .enumerate()
        .skip(offset)
        .take(view)
        .map(|(i, item)| {
            let current = i == selected;
            let style = if current { theme.selected() } else { fill };
            Row::new(
                style,
                vec![
                    styled(
                        if current { theme.glyphs().pointer } else { " " },
                        style.patch(if current {
                            Style::default()
                        } else {
                            theme.accent()
                        }),
                    ),
                    styled(" ", style),
                    styled(
                        theme.fit(&theme.clean(item), width.saturating_sub(3)),
                        if current {
                            style
                        } else {
                            style.patch(theme.text())
                        },
                    ),
                ],
            )
        })
        .collect();
    if items.is_empty() {
        rows.push(Row::new(
            fill,
            vec![styled(format!("  {empty}"), fill.patch(theme.dim()))],
        ));
    }
    let buf = frame.buffer_mut();
    draw_rows(buf, body, &rows);
    scrollbar(
        buf,
        theme,
        Rect::new(body.right() + 1, body.y, 1, body.height).intersection(parts[1]),
        items.len(),
        offset,
        view,
    );
}
