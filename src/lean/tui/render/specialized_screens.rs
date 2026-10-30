use super::super::*;

mod graph;

pub(super) fn draw_secondary_or_main(app: &mut App, frame: &mut Frame) {
    if draw_plan(app, frame) || draw_diff(app, frame) || draw_details(app, frame) {
        return;
    }
    app.draw_main(frame);
}

fn draw_plan(app: &mut App, frame: &mut Frame) -> bool {
    let Some(plan) = &mut app.plan_view else {
        return false;
    };
    let theme = Theme::from_env();
    // PlanView::draw and its presentation helpers remain pending ownership transfer
    // to tui-specialized-screens; this coordinator only dispatches and syncs viewport state.
    plan.draw(frame, frame.area(), &app.view, &theme);
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
    let Some(diff) = &mut app.diff else {
        return false;
    };
    diff.draw(frame, &app.view);
    true
}

fn draw_details(app: &mut App, frame: &mut Frame) -> bool {
    let Some(details) = &mut app.details else {
        return false;
    };
    details.draw(frame, &app.view);
    app.text_area = Some(details.text_area);
    app.text_origin = details.scroll;
    app.sync_native_views();
    true
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

    pub(super) fn draw_main(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let bar_height = Self::bar_height(area);
        let [body, _, footer] = Self::main_areas(area);

        let theme = Theme::from_env();
        if area.width >= SIDE_BY_SIDE_BREAKPOINT {
            let chat_width = (body.width / 3).max(34);
            let [chat, _, team] = Layout::horizontal([
                Constraint::Length(chat_width),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .areas(body);
            self.chat_area = chat;
            self.draw_chat(frame, chat, &theme);
            self.draw_graph(frame, team);
        } else if self.focus == Focus::Chat {
            self.chat_area = body;
            self.draw_chat(frame, body, &theme);
        } else {
            self.chat_area = body;
            self.draw_graph(frame, body);
        }

        self.draw_footer(frame, footer, bar_height, &theme);
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
