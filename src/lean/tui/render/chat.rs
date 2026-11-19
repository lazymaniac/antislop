use super::super::*;
use crate::lean::tui::layout::lines::{compute_visual_lines, visual_text_lines_from_layout};

impl App {
    pub(super) fn draw_chat(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let (input_lines, cursor, areas) = self.chat_layout(area);
        self.draw_phase_action(frame, areas[2], theme);
        self.draw_sending(frame, areas[1], theme);
        self.draw_transcript(frame, areas[0], theme);
        if let Some(approval) = &self.approval {
            approval.draw(frame, areas[3]);
        }
        self.draw_chat_input(frame, areas[4], input_lines, cursor, theme);
    }

    fn chat_layout(&mut self, area: Rect) -> (Vec<String>, (u16, usize), [Rect; 5]) {
        let input_width = usize::from(area.width.saturating_sub(3)).max(1);
        self.input_width = input_width;
        let layout = compute_visual_lines(&self.input.text, input_width);
        let cursor = self.input.visual_cursor_from_lines(&layout);
        let lines = visual_text_lines_from_layout(&self.input.text, &layout);
        let input_height =
            (lines.len().clamp(1, 6) as u16 + 2).min(area.height.saturating_sub(1).max(1));
        let status_height = u16::from(self.sending);
        // A framed button three rows high, when there is room for it.
        let action_height = match self.phase_action() {
            Some(_) if area.height >= input_height + 8 => 3,
            Some(_) => 1,
            None => 0,
        };
        let approval_height = self.approval.as_ref().map_or(0, |approval| {
            let fixed = input_height
                .saturating_add(status_height)
                .saturating_add(action_height);
            super::super::approval::approval_panel_height(
                &approval.approval.summary,
                input_width,
                approval.approval.options.len(),
                usize::from(area.height),
                5,
                usize::from(area.height.saturating_sub(fixed)),
            )
        });
        let areas = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(status_height),
            Constraint::Length(action_height),
            Constraint::Length(approval_height),
            Constraint::Length(input_height),
        ])
        .areas(area);
        self.phase_action_area = None;
        self.phase_action_command = None;
        (lines, cursor, areas)
    }

    pub(in crate::lean::tui) fn phase_action(&self) -> Option<(&'static str, Command)> {
        if self.view.interview_question.is_some() && !self.interview_visible() {
            Some(("Answer scope question · F3", Command::InterviewQuestion))
        } else if self.view.plan_proposed() {
            Some(("Review & accept proposed plan · Ctrl+L", Command::Plan))
        } else if self.view.audit_retryable() {
            Some(("Retry final audit · F5 / u", Command::TaskActions))
        } else if self
            .view
            .tasks
            .iter()
            .any(|t| matches!(t.status.as_str(), "failed" | "needs_user"))
        {
            Some(("Unblock stopped task · F5 / u", Command::TaskActions))
        } else if !self.view.approvals.is_empty() && self.approval.is_none() {
            Some((
                "Answer pending CLI approval · Ctrl+P",
                Command::PendingApprovals,
            ))
        } else {
            None
        }
    }

    pub(in crate::lean::tui) fn draw_phase_action(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        theme: &Theme,
    ) {
        let Some((label, command)) = self.phase_action() else {
            return;
        };
        let text = format!("  {label} {}  ", if theme.ascii { ">" } else { "▶" });
        let framed = area.height >= 3;
        let width = (unicode_width::UnicodeWidthStr::width(text.as_str()) as u16
            + 2 * u16::from(framed))
        .min(area.width);
        let button = Rect::new(
            area.x + 1.min(area.width - width),
            area.y,
            width,
            area.height,
        );
        self.phase_action_area = Some(button);
        self.phase_action_command = Some(command);
        let mut widget = Paragraph::new(text).style(theme.button());
        if framed {
            widget = widget.block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme.button()),
            );
        }
        frame.render_widget(widget, button);
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
        cursor: (u16, usize),
        theme: &Theme,
    ) {
        let visible_box_lines = usize::from(input.height.saturating_sub(2)).max(1);
        let (cursor_col, cursor_row) = cursor;
        let shown_lines = self.chat_input_window(&input_lines, cursor_row, visible_box_lines);
        let focused = self.focus == Focus::Chat
            && self.editor.is_none()
            && self.picker.is_none()
            && self.help.is_none()
            && self.diff.is_none()
            && (self.details.is_none() || self.graph_expanded.is_some());
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
        if input.height > 0 && input.width >= 28 {
            let hint = truncate(
                &format!("· {} (Ctrl+M) · / commands ", self.view.rigor_label()),
                usize::from(input.width.saturating_sub(11)),
            );
            frame
                .buffer_mut()
                .set_string(input.x + 10, input.y, hint, fill.patch(theme.dim()));
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn hidden_expanded_details_leave_the_chat_composer_active_and_editable() {
        for color in [true, false] {
            let theme = Theme::new(color, false);
            let mut app = App::test_app();
            app.graph_expanded = Some("worker:t1".into());
            app.details = Some(Details::new("worker:t1".into()));
            app.input = InputBuffer::with_text("draft 👩‍💻e\u{301}");
            let mut terminal = Terminal::new(TestBackend::new(60, 16)).unwrap();
            for focus in [Focus::Graph, Focus::Chat] {
                app.focus = focus;
                terminal
                    .draw(|frame| app.draw_chat(frame, frame.area(), &theme))
                    .unwrap();
                let label = &terminal.backend().buffer()[(2, 13)];
                let expected = if focus == Focus::Chat {
                    theme.accent()
                } else {
                    theme.dim()
                };
                assert_eq!(label.fg, expected.fg.unwrap_or(Color::Reset));
                assert_eq!(
                    label.modifier.contains(Modifier::DIM),
                    expected.add_modifier.contains(Modifier::DIM)
                );
                assert_eq!(
                    label.modifier.contains(Modifier::BOLD),
                    focus == Focus::Chat
                );
                if focus == Focus::Chat {
                    let cursor = terminal.get_cursor_position().unwrap();
                    assert_eq!(cursor.y, 14);
                    assert!(cursor.x > 1);
                }
            }
            assert!(app.key(event::KeyEvent::new(
                event::KeyCode::Char('q'),
                event::KeyModifiers::NONE
            )));
            assert_eq!(app.input.text, "draft 👩‍💻e\u{301}q");
            app.help = Some(palette::Help::Chat);
            terminal
                .draw(|frame| app.draw_chat(frame, frame.area(), &theme))
                .unwrap();
            assert_eq!(
                terminal.backend().buffer()[(2, 13)].fg,
                theme.dim().fg.unwrap_or(Color::Reset)
            );
            assert_eq!(app.input.text, "draft 👩‍💻e\u{301}q");
        }
    }
}
