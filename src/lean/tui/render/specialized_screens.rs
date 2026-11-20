use super::super::*;

mod graph;

pub(super) fn draw_secondary_or_main(app: &mut App, frame: &mut Frame, theme: &Theme) {
    if draw_plan(app, frame, theme) || draw_diff(app, frame) || draw_details(app, frame) {
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

fn draw_diff(app: &mut App, frame: &mut Frame) -> bool {
    if app.graph_expanded.is_some() {
        return false;
    }
    let Some(diff) = &mut app.diff else {
        return false;
    };
    if app.navigation.screen == Screen::Workbench {
        let presentation = app.navigation.presentation();
        let areas =
            super::super::workbench::areas(frame.area(), &presentation, app.navigation.focus);
        app.workbench_areas = areas;
        draw_workbench_chrome(frame, areas, &app.navigation.target, &presentation);
        if areas.list_visible {
            draw_workbench_list(frame, areas.list, &app.view, &app.navigation.target);
        }
        diff.paint_at(frame, areas.content, &app.view);
        if let Some(details) = &mut app.details {
            details.native_focus = app.navigation.focus == NavigationFocus::WorkbenchTerminal;
            details.draw_workbench_terminal(frame, areas.terminal, &app.view);
        }
    } else {
        diff.paint_at(frame, frame.area(), &app.view);
    }
    true
}

fn draw_details(app: &mut App, frame: &mut Frame) -> bool {
    if app.graph_expanded.is_some() {
        return false;
    }
    let Some(details) = &mut app.details else {
        return false;
    };
    if app.navigation.screen == Screen::Workbench {
        let presentation = app.navigation.presentation();
        let areas =
            super::super::workbench::areas(frame.area(), &presentation, app.navigation.focus);
        app.workbench_areas = areas;
        draw_workbench_chrome(frame, areas, &app.navigation.target, &presentation);
        if areas.list_visible {
            draw_workbench_list(frame, areas.list, &app.view, &app.navigation.target);
        }
        details.draw_workbench_at(frame, areas.content, areas.terminal, &app.view);
    } else {
        app.workbench_areas = Default::default();
        details.draw_at(frame, frame.area(), &app.view);
    }
    app.text_area = Some(details.text_area);
    app.text_origin = details.scroll;
    app.sync_native_views();
    true
}

fn draw_workbench_chrome(
    frame: &mut Frame,
    areas: super::super::workbench::WorkbenchAreas,
    target: &WorkbenchTarget,
    presentation: &WorkbenchPresentation,
) {
    let theme = Theme::from_env();
    let context = super::super::workbench::context(target, presentation);
    let identity = match &context.target {
        WorkbenchTarget::Session => "Session".to_owned(),
        WorkbenchTarget::Agent(key) => key.clone(),
    };
    frame.render_widget(
        Paragraph::new(format!(
            " {identity} · {:?} · {:?} · round {:?} · {:?}",
            context.role, presentation.tab, context.round, context.focus
        ))
        .style(theme.bar()),
        areas.header,
    );
    let split_hint = if areas.split {
        "Tab panel · Alt+↑/↓ resize · z zoom · Ctrl+B list"
    } else {
        "Tab panel · Ctrl+B list · Esc back"
    };
    frame.render_widget(Paragraph::new(split_hint).style(theme.bar()), areas.footer);
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
                .border_style(theme.faint()),
            areas.list,
        );
    }
}

fn draw_workbench_list(frame: &mut Frame, area: Rect, view: &View, target: &WorkbenchTarget) {
    let theme = Theme::from_env();
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
        let line = self.header_line(content_width);
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
