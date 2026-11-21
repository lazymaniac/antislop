use super::super::*;
use crate::lean::tui::layout::lines::{compute_visual_lines, visual_text_lines_from_layout};

/// Match the core's initial `@recipient` and `/direct` routing rules.
fn chat_header(text: &str, rigor: &str) -> String {
    if text
        .trim()
        .strip_prefix("/direct")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
    {
        return format!("Direct execution · {rigor}");
    }
    let Some(rest) = text.strip_prefix('@') else {
        return format!("To · orchestrator · {rigor}");
    };
    let Some((recipient, _)) = rest.split_once(char::is_whitespace) else {
        return "Address · incomplete".into();
    };
    if recipient.is_empty() {
        return "Address · empty recipient".into();
    }
    format!("To · {recipient} · {rigor}")
}

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
        let panel = super::composer::centered_area(area);
        let input_width = super::composer::panel_wrap_width(panel);
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
        let input = super::composer::centered_area(input);
        let editor = super::composer::panel_editor_area(input);
        let visible_box_lines = usize::from(editor.height).max(1);
        let (cursor_col, cursor_row) = cursor;
        let shown_lines = self.chat_input_window(&input_lines, cursor_row, visible_box_lines);
        let focused = self.focus == Focus::Chat
            && self.editor.is_none()
            && self.picker.is_none()
            && self.help.is_none()
            && self.diff.is_none()
            && (self.details.is_none() || self.graph_expanded.is_some());
        let attachments = self
            .image_drafts
            .get(&self.conversation)
            .map(|images| {
                images
                    .iter()
                    .filter_map(|path| {
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let attachment_status = if attachments.is_empty() {
            "Attachments · drop or paste image".to_owned()
        } else {
            format!("Attachments · {attachments}")
        };
        let status = format!("Enter send · Alt+Enter newline · {attachment_status}");
        super::composer::draw_panel(
            frame,
            input,
            super::composer::Panel {
                title: &chat_header(&self.input.text, self.view.rigor_label()),
                status: &status,
                lines: shown_lines,
                focused,
                cursor: Some((cursor_col, cursor_row, self.input_shown)),
            },
            theme,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn chat_header_matches_the_destination_parsed_by_submit() {
        assert_eq!(
            chat_header("hello", "feature"),
            "To · orchestrator · feature"
        );
        assert_eq!(chat_header("@all", "feature"), "Address · incomplete");
        assert_eq!(
            chat_header("@all notify everyone", "feature"),
            "To · all · feature"
        );
        assert_eq!(
            chat_header("@worker:task-1 please help", "feature"),
            "To · worker:task-1 · feature"
        );
        assert_eq!(
            chat_header("/direct implement this", "feature"),
            "Direct execution · feature"
        );
        assert_eq!(
            chat_header("/directive text", "feature"),
            "To · orchestrator · feature"
        );
    }

    #[test]
    fn composer_panel_stays_centered_and_bounded_on_wide_workbenches() {
        assert_eq!(
            super::super::composer::centered_area(Rect::new(0, 0, 120, 12)),
            Rect::new(18, 0, 84, 12)
        );
        assert_eq!(
            super::super::composer::centered_area(Rect::new(3, 2, 11, 5)),
            Rect::new(3, 2, 11, 5)
        );
    }

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
                let (_, _, areas) = app.chat_layout(Rect::new(0, 0, 60, 16));
                let input = super::super::composer::centered_area(areas[4]);
                let label = &terminal.backend().buffer()[(input.x + 1, input.y)];
                let expected_fill = if focus == Focus::Chat {
                    theme.raised()
                } else {
                    theme.card()
                };
                let expected = theme.dim();
                assert_eq!(label.bg, expected_fill.bg.unwrap_or(Color::Reset));
                assert_eq!(label.fg, expected.fg.unwrap_or(Color::Reset));
                assert_eq!(
                    label.modifier.contains(Modifier::DIM),
                    expected.add_modifier.contains(Modifier::DIM)
                );
                assert!(!label.modifier.contains(Modifier::BOLD));
                if focus == Focus::Chat {
                    let cursor = terminal.get_cursor_position().unwrap();
                    assert_eq!(cursor.y, input.y + 1);
                    assert!(cursor.x > input.x);
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
            let (_, _, areas) = app.chat_layout(Rect::new(0, 0, 60, 16));
            let input = super::super::composer::centered_area(areas[4]);
            assert_eq!(
                terminal.backend().buffer()[(input.x + 1, input.y)].fg,
                theme.dim().fg.unwrap_or(Color::Reset)
            );
            assert_eq!(app.input.text, "draft 👩‍💻e\u{301}q");
        }
    }
}
