use super::super::*;

impl App {
    pub(super) fn draw_chat(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let (input_lines, areas) = self.chat_layout(area);
        self.draw_phase_action(frame, areas[2], theme);
        self.draw_sending(frame, areas[1], theme);
        self.draw_transcript(frame, areas[0], theme);
        self.draw_chat_input(frame, areas[3], input_lines, theme);
    }

    fn chat_layout(&mut self, area: Rect) -> (Vec<String>, [Rect; 4]) {
        let input_width = usize::from(area.width.saturating_sub(3)).max(1);
        self.input_width = input_width;
        let lines = visual_text_lines(&self.input.text, input_width);
        let input_height =
            (lines.len().clamp(1, 6) as u16 + 2).min(area.height.saturating_sub(1).max(1));
        let status_height = u16::from(self.sending);
        let action_height = u16::from(self.phase_action().is_some());
        let areas = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(status_height),
            Constraint::Length(action_height),
            Constraint::Length(input_height),
        ])
        .areas(area);
        self.phase_action_area = None;
        self.phase_action_command = None;
        (lines, areas)
    }

    fn phase_action(&self) -> Option<(&'static str, &'static str)> {
        if self.view.requirements_approval_run.is_some() {
            Some(("Review requirements changes", "/requirements"))
        } else if self.view.workflow_phase.as_deref() == Some("interview")
            && !self.view.requirements_summary.is_empty()
        {
            Some(("Proceed to planning", "/planning"))
        } else if self.view.plan_proposed() {
            Some(("Proceed to implementation", "/accept"))
        } else {
            None
        }
    }

    fn draw_phase_action(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if let Some((label, command)) = self.phase_action() {
            let text = format!("  {label} {}  ", if theme.ascii { ">" } else { "▶" });
            let width = (text.chars().count() as u16).min(area.width);
            let button = Rect::new(area.x + 1.min(area.width - width), area.y, width, 1);
            self.phase_action_area = Some(button);
            self.phase_action_command = Some(command);
            frame.render_widget(Paragraph::new(text).style(theme.button()), button);
        }
    }

    fn draw_sending(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if self.sending {
            frame.render_widget(
                Paragraph::new(format!(
                    "{} orchestrator sending…",
                    theme.spinner(self.tick)
                ))
                .style(theme.bar()),
                area,
            );
        }
    }

    fn draw_chat_input(
        &mut self,
        frame: &mut Frame,
        input: Rect,
        input_lines: Vec<String>,
        theme: &Theme,
    ) {
        let visible_box_lines = usize::from(input.height.saturating_sub(2)).max(1);
        let (cursor_col, cursor_row) = self.input.visual_cursor(self.input_width);
        let shown_lines = self.chat_input_window(&input_lines, cursor_row, visible_box_lines);
        let focused = self.focus == Focus::Chat
            && self.editor.is_none()
            && self.picker.is_none()
            && self.help.is_none()
            && self.trust_prompt.is_none()
            && self.diff.is_none()
            && self.details.is_none();
        self.paint_input(frame, input, shown_lines, focused, theme);
        self.place_chat_cursor(
            frame,
            input,
            cursor_col,
            cursor_row,
            visible_box_lines,
            focused,
        );
    }

    fn chat_input_window<'a>(
        &mut self,
        lines: &'a [String],
        cursor: usize,
        visible: usize,
    ) -> &'a [String] {
        self.input_shown = cursor.saturating_sub(visible.saturating_sub(1));
        self.input_shown = self.input_shown.min(lines.len().saturating_sub(visible));
        let end = (self.input_shown + visible).min(lines.len());
        &lines[self.input_shown..end]
    }

    fn paint_input(
        &self,
        frame: &mut Frame,
        input: Rect,
        lines: &[String],
        focused: bool,
        theme: &Theme,
    ) {
        let fill = if focused {
            theme.raised()
        } else {
            theme.card()
        };
        let accent = if focused {
            theme.accent()
        } else {
            theme.faint()
        };
        paint_input_background(frame, input, fill, accent, focused, theme);
        paint_input_lines(frame, input, lines, fill, theme);
    }

    fn place_chat_cursor(
        &self,
        frame: &mut Frame,
        input: Rect,
        column: u16,
        row: usize,
        visible: usize,
        focused: bool,
    ) {
        if focused && input.width >= 3 && input.height >= 2 {
            let x = (input.x + 1 + column).clamp(
                input.x + 1,
                input.right().saturating_sub(2).max(input.x + 1),
            );
            let y = (input.y
                + 1
                + row
                    .saturating_sub(self.input_shown)
                    .min(visible.saturating_sub(1)) as u16)
                .clamp(
                    input.y + 1,
                    input.bottom().saturating_sub(1).max(input.y + 1),
                );
            frame.set_cursor_position((x, y));
        }
    }
}

fn paint_input_background(
    frame: &mut Frame,
    input: Rect,
    fill: Style,
    accent: Style,
    focused: bool,
    theme: &Theme,
) {
    let buffer = frame.buffer_mut();
    for y in input.top()..input.bottom() {
        for x in input.left()..input.right() {
            buffer[(x, y)].set_style(fill);
        }
        if input.width > 0 {
            buffer[(input.x, y)]
                .set_symbol(theme.glyphs().bar)
                .set_style(fill.patch(accent));
        }
    }
    if input.height > 0 && input.width > 2 {
        let style = fill.patch(if focused { theme.accent() } else { theme.dim() });
        buffer.set_string(input.x + 1, input.y, " message ", style);
    }
}

fn paint_input_lines(frame: &mut Frame, input: Rect, lines: &[String], fill: Style, theme: &Theme) {
    let inner = Rect::new(
        input.x + 1,
        input.y + 1,
        input.width.saturating_sub(2),
        input.height.saturating_sub(1),
    );
    let rows = lines.iter().cloned().map(Line::raw).collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(rows).style(fill.patch(theme.text())), inner);
}
