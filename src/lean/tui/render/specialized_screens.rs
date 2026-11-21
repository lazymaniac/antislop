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
        if historical_diff {
            frame.render_widget(
                Paragraph::new(theme.clean("No historical diff is retained for this round."))
                    .style(theme.dim()),
                areas.content,
            );
        } else {
            diff.paint_with_theme(frame, areas.content, &app.view, theme);
        }
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
        if app.diff.is_none() {
            app.text_area = Some(details.text_area);
            app.text_origin = details.scroll;
        }
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
        WorkbenchTab::Summary => "Podsumowanie",
        WorkbenchTab::Conversation => "Rozmowa",
        WorkbenchTab::Results => "Wyniki",
        WorkbenchTab::Checks => "Sprawdzenia",
        WorkbenchTab::Comments => "Komentarze",
        WorkbenchTab::Diff => "Diff",
    };
    frame.render_widget(
        Paragraph::new(format!(
            " {identity} {sep} {tab} {sep} CLI {terminal_owner} {sep} r{} {sep} {}{}",
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
        "Tab panel · F6 pause/continue · Ctrl+X cancel · Ctrl+P actions · Alt+↑/↓ resize · z zoom · Ctrl+B list"
    } else {
        "Tab panel · F6 pause/continue · Ctrl+X cancel · Ctrl+P actions · Ctrl+B list · Esc back"
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
    for index in 0..usize::from(area.height) {
        let Some((item, label)) = super::super::workbench::target_at(view, index) else {
            break;
        };
        let selected = match target {
            WorkbenchTarget::Session => item == "conversation",
            WorkbenchTarget::Agent(key) => item == *key,
        };
        let style = if selected {
            theme.title().add_modifier(Modifier::REVERSED)
        } else {
            theme.text()
        };
        frame.render_widget(
            Paragraph::new(label).style(style),
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
        if area.width >= SIDE_BY_SIDE_BREAKPOINT {
            let chat_width = (body.width / 3).max(34);
            let [chat, _, team] = Layout::horizontal([
                Constraint::Length(chat_width),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .areas(body);
            self.chat_area = chat;
            self.draw_chat(frame, chat, theme);
            self.draw_graph(frame, team, theme);
        } else if self.focus == Focus::Chat {
            self.chat_area = body;
            self.draw_chat(frame, body, theme);
        } else if self.approval.is_some() && body.height > 1 {
            let desired = self.approval.as_ref().map_or(6, |approval| {
                let text_width = usize::from(body.width.saturating_sub(2)).max(1);
                super::super::approval::approval_panel_height(
                    &approval.approval.summary,
                    text_width,
                    approval.approval.options.len(),
                    usize::from(body.height),
                    7,
                    usize::from(body.height),
                )
            });
            let chat_height = desired
                .min((body.height / 2).max(5))
                .min(body.height.saturating_sub(1));
            let [team, chat] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(chat_height)]).areas(body);
            self.chat_area = chat;
            self.draw_graph(frame, team, theme);
            self.draw_chat(frame, chat, theme);
        } else {
            self.chat_area = body;
            self.draw_graph(frame, body, theme);
        }

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
