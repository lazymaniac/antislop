use super::super::*;

mod graph;

pub(super) fn draw_secondary_or_main(app: &mut App, frame: &mut Frame, theme: &Theme) {
    if draw_plan(app, frame, theme) || draw_workbench(app, frame, theme) {
        return;
    }
    app.draw_main(frame, theme);
}

fn draw_plan(app: &mut App, frame: &mut Frame, theme: &Theme) -> bool {
    let Some(plan) = &mut app.plan_view else {
        return false;
    };
    // PlanView::draw and its presentation helpers remain pending ownership transfer
    // to tui-specialized-screens; this coordinator only dispatches and syncs viewport state.
    plan.draw(frame, frame.area(), &app.view, theme);
    app.text_area = Some(Rect::new(
        plan.doc_area.x.saturating_add(planview::GUTTER as u16),
        plan.doc_area.y,
        plan.doc_area.width.saturating_sub(planview::GUTTER as u16),
        plan.doc_area.height,
    ));
    app.text_origin = plan.scroll;
    true
}

fn draw_workbench(app: &mut App, frame: &mut Frame, theme: &Theme) -> bool {
    if app.graph_expanded.is_some()
        || app.navigation.screen != Screen::Workbench
        || (app.diff.is_none() && app.details.is_none())
    {
        return false;
    }
    let presentation = app.navigation.presentation();
    let terminal_owner = app.workbench_terminal_owner();
    let terminal_view_role = app.workbench_terminal_view_role();
    let terminal_round = workbench_terminal_round(
        &app.workbench_sessions,
        app.workbench_sessions_after,
        app.details.as_ref(),
        &presentation,
        &terminal_owner,
    );
    let historical_terminal = historical_workbench_terminal(app.details.as_ref(), &terminal_owner);
    let historical_diff = app.historical_diff_selected();
    let areas = super::super::workbench::areas(frame.area(), &presentation, app.navigation.focus);
    app.workbench_areas = areas;
    draw_workbench_chrome(
        frame,
        areas,
        &app.navigation.target,
        &presentation,
        &terminal_owner,
        terminal_round,
        theme,
    );
    if areas.list_visible {
        draw_workbench_list(frame, areas.list, &app.view, &app.navigation.target, theme);
    }
    if let Some(diff) = &mut app.diff {
        let [tabs, content] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(areas.content);
        if let Some(details) = &app.details {
            frame.render_widget(
                super::super::workbench::tab_line(&details.key, true, WorkbenchTab::Diff, theme),
                tabs,
            );
        }
        if historical_diff {
            frame.render_widget(
                Paragraph::new(theme.clean("No historical diff is retained for this round."))
                    .style(theme.dim()),
                content,
            );
        } else {
            diff.paint_with_theme(frame, content, &app.view, theme);
        }
    } else if app
        .details
        .as_ref()
        .is_some_and(|details| details.key == "conversation")
        && presentation.tab == WorkbenchTab::Conversation
    {
        app.draw_chat(frame, areas.content, theme);
    } else if let Some(details) = &mut app.details {
        details.draw_workbench_at(
            frame,
            areas.content,
            Rect::default(),
            &app.view,
            presentation.tab,
            theme,
        );
    }
    if let Some(details) = &mut app.details {
        draw_workbench_terminal(
            frame,
            areas.terminal,
            details,
            &app.view,
            &terminal_view_role,
            presentation.native_input,
            historical_terminal,
            theme,
        );
        if app.diff.is_none() && details.key != "conversation" {
            app.text_area = Some(details.text_area);
            app.text_origin = details.scroll;
        }
    }
    if let Some(notice) = &app.notice {
        frame.render_widget(
            Paragraph::new(theme.clean(notice)).style(theme.warning()),
            areas.footer,
        );
    }
    app.sync_native_views();
    true
}

fn draw_workbench_chrome(
    frame: &mut Frame,
    areas: super::super::workbench::WorkbenchAreas,
    target: &WorkbenchTarget,
    presentation: &WorkbenchPresentation,
    terminal_owner: &str,
    terminal_round: Option<u64>,
    theme: &Theme,
) {
    let sep = theme.glyphs().dot;
    let context = super::super::workbench::context(target, presentation);
    let identity = match &context.target {
        WorkbenchTarget::Session => "Session".to_owned(),
        WorkbenchTarget::Agent(key) => key.clone(),
    };
    let tab = match presentation.tab {
        WorkbenchTab::Summary => "Summary",
        WorkbenchTab::Conversation => "Conversation",
        WorkbenchTab::Results => "Results",
        WorkbenchTab::Checks => "Checks",
        WorkbenchTab::Comments => "Comments",
        WorkbenchTab::Diff => "Diff",
    };
    frame.render_widget(
        Paragraph::new(format!(
            " {} antislop / {identity} {sep} {tab} {sep} CLI {terminal_owner} {sep} r{} {sep} {}{}",
            theme.glyphs().arrow_right,
            terminal_round.map_or_else(|| theme.chrome("—"), |round| round.to_string()),
            if presentation.native_input {
                "native input"
            } else {
                "preview"
            },
            if presentation.pinned {
                theme.chrome(" · pinned")
            } else {
                theme.chrome(" · following")
            }
        ))
        .style(theme.bar()),
        areas.header,
    );
    let split_hint = if areas.split {
        "Ctrl+K direction · Tab panes · Space pause/continue · Ctrl+X cancel · Ctrl+P actions · Alt+↑/↓ resize · z zoom · Ctrl+B list"
    } else {
        "Ctrl+K direction · Tab panes · Space pause/continue · Ctrl+X cancel · Ctrl+P actions · Ctrl+B list · Esc back"
    };
    frame.render_widget(
        Paragraph::new(theme.chrome(split_hint)).style(theme.bar()),
        areas.footer,
    );
    if !areas.separator.is_empty() {
        frame.render_widget(
            Paragraph::new(if theme.ascii { "-" } else { "─" }),
            areas.separator,
        );
    }
    if areas.list_visible {
        frame.render_widget(
            Block::default()
                .borders(Borders::RIGHT)
                .border_set(theme.borders())
                .border_style(theme.faint()),
            areas.list,
        );
    }
}

fn workbench_terminal_round(
    sessions: &[crate::lean::store::SessionInfo],
    sessions_after: u64,
    details: Option<&super::super::Details>,
    presentation: &WorkbenchPresentation,
    owner: &str,
) -> Option<u64> {
    let matching_details = details.filter(|details| details_match_terminal_owner(details, owner));
    matching_details
        .and_then(|details| details.selected_round)
        .or_else(|| {
            matching_details
                .filter(|details| details.fetched_events >= sessions_after)
                .and_then(|details| details.task.as_ref()?.get("round")?.as_u64())
        })
        .or_else(|| {
            sessions
                .iter()
                .filter(|session| session.role == owner)
                .count()
                .checked_sub(1)
                .map(|index| index as u64)
        })
        .or(presentation.round.filter(|_| matching_details.is_some()))
}

fn historical_workbench_terminal(details: Option<&super::super::Details>, owner: &str) -> bool {
    details.is_some_and(|details| {
        details.selected_round.is_some()
            && !details.resume_round
            && details_match_terminal_owner(details, owner)
    })
}

fn details_match_terminal_owner(details: &super::super::Details, owner: &str) -> bool {
    details.composer_recipient(&details.key) == owner
        || details.task_uuid.as_deref().is_some_and(|task| {
            owner
                .split_once(':')
                .is_some_and(|(_, owner_task)| owner_task == task)
        })
}

#[allow(clippy::too_many_arguments)]
fn draw_workbench_terminal(
    frame: &mut Frame,
    area: Rect,
    details: &mut super::super::Details,
    view: &View,
    role: &str,
    native_input: bool,
    historical: bool,
    theme: &Theme,
) {
    if historical {
        details.native_focus = false;
        details.native_areas = [Rect::default(); 2];
        frame.render_widget(
            Paragraph::new(
                theme.clean(
                    "Historical PTY is read-only; no retained terminal snapshot is available.",
                ),
            )
            .style(theme.dim()),
            area,
        );
    } else {
        details.native_focus = native_input;
        details.draw_workbench_terminal_for(frame, area, view, role, theme);
    }
}

fn draw_workbench_list(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    target: &WorkbenchTarget,
    theme: &Theme,
) {
    let rows = super::super::workbench::sidebar(view);
    let start = super::super::workbench::visible_start(view, target, area.height);
    let selected = match target {
        WorkbenchTarget::Session => "conversation",
        WorkbenchTarget::Agent(key) => key,
    };
    for (index, row) in rows
        .iter()
        .skip(start)
        .take(usize::from(area.height))
        .enumerate()
    {
        let style = if row.stage && row.key == selected {
            theme.selected()
        } else if row.stage {
            theme.dim()
        } else {
            theme.text()
        };
        let label = if row.stage {
            let health = crate::lean::tui::graph::health(view, &row.key);
            format!(
                "  {} {}",
                theme.status_glyph(health.name()),
                row.label.trim_start()
            )
        } else {
            row.label.clone()
        };
        frame.render_widget(
            Paragraph::new(theme.fit(
                &theme.clean(&label),
                usize::from(area.width.saturating_sub(1)),
            ))
            .style(style),
            Rect::new(
                area.x,
                area.y + index as u16,
                area.width.saturating_sub(1),
                1,
            ),
        );
    }
}

impl App {
    fn draw_footer(&self, frame: &mut Frame, footer: Rect, bar_height: u16, theme: &Theme) {
        frame.render_widget(Block::default().style(theme.bar()), footer);
        let text_area = Rect::new(footer.x, footer.y + (bar_height - 1) / 2, footer.width, 1);
        let content_width = footer.width.saturating_sub(4);
        let [status, help] =
            Layout::horizontal([Constraint::Length(content_width), Constraint::Length(4)])
                .areas(text_area);
        let line = self.header_line_themed(content_width, theme);
        // A taller bar makes its buttons taller too: each is filled on the
        // rows around its label.
        let buttons = button_rows(&line);
        for y in footer.y..footer.y + bar_height {
            if y != status.y {
                let row = Rect::new(status.x, y, status.width, 1);
                frame.render_widget(Paragraph::new(buttons.clone()).style(theme.bar()), row);
            }
        }
        frame.render_widget(Paragraph::new(line).style(theme.bar()), status);
        frame.render_widget(
            Paragraph::new(Span::styled(" [?]", theme.title())).style(theme.bar()),
            help,
        );
    }

    pub(super) fn draw_main(&mut self, frame: &mut Frame, theme: &Theme) {
        let area = frame.area();
        let bar_height = Self::bar_height(area);
        let [body, _, footer] = Self::main_areas(area);
        self.chat_area = body;
        let [identity, canvas, invitation] = Layout::vertical([
            Constraint::Length(u16::from(body.height >= 10)),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .areas(body);
        let pending = self.view.approvals.len()
            + usize::from(
                self.view.interview_question.is_some()
                    && (self.view.user_decision
                        || self.view.workflow_phase.as_deref() == Some("interview")),
            );
        frame.render_widget(
            Paragraph::new(format!(
                " {} antislop   {} / {}   Canvas   {pending} decisions",
                theme.glyphs().arrow_right,
                theme.fit(&theme.clean(&self.project), 26),
                theme.fit(&theme.clean(&self.conversation), 22)
            ))
            .style(theme.text()),
            identity,
        );
        self.draw_graph(frame, canvas, theme);
        frame.render_widget(
            Paragraph::new(theme.chrome(
                "  Ctrl+K  Give direction, change scope, ask a question…
  Arrows select · Enter Workbench · Tab stages · d decisions · h history · Space pause",
            ))
            .style(theme.card()),
            invitation,
        );

        self.draw_footer(frame, footer, bar_height, theme);
    }
}

/// The bar line with every label blanked, keeping only the buttons' fill.
fn button_rows(line: &Line<'static>) -> Line<'static> {
    let spans = line.spans.iter().map(|span| {
        let width = unicode_width::UnicodeWidthStr::width(span.content.as_ref());
        let button = span.content.starts_with('[') && span.content.ends_with(']');
        let style = if button { span.style } else { Style::default() };
        Span::styled(" ".repeat(width), style)
    });
    Line::from(spans.collect::<Vec<_>>())
}
