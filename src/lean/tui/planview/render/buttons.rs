use super::super::*;

#[derive(Clone, Copy)]
struct ButtonState {
    drafts: usize,
    can_accept: bool,
    decidable: bool,
}

impl PlanView {
    pub(super) fn draw_buttons(
        &mut self,
        frame: &mut Frame,
        bar_area: Rect,
        view: &View,
        theme: &Theme,
        open_comments: &[&super::super::super::view::Comment],
    ) {
        let btn_cols = self.button_areas(bar_area);
        let drafts = view.draft_plan_comments().len();
        let decidable = view.plan_proposed();
        let can_accept = open_comments.is_empty() && decidable;
        let state = ButtonState {
            drafts,
            can_accept,
            decidable,
        };
        self.draw_button_group(frame, btn_cols, theme, state);
    }

    fn draw_button_group(
        &self,
        frame: &mut Frame,
        areas: [Rect; 4],
        theme: &Theme,
        state: ButtonState,
    ) {
        for (index, button) in [
            BottomButton::Accept,
            BottomButton::Reject,
            BottomButton::CommentWholePlan,
            BottomButton::Review,
        ]
        .into_iter()
        .enumerate()
        {
            self.draw_button(frame, areas[index], button, theme, state);
        }
    }

    fn button_areas(&mut self, bar_area: Rect) -> [Rect; 4] {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(25); 4])
            .split(bar_area);
        self.button_rects = [cols[0], cols[1], cols[2], cols[3]];
        self.button_rects
    }

    fn draw_button(
        &self,
        frame: &mut Frame,
        rect: Rect,
        button: BottomButton,
        theme: &Theme,
        state: ButtonState,
    ) {
        let (label, tone) = self.button_label_tone(button, state);
        let active = self.button == button;
        let style = self.button_style(theme, tone, active);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(if active {
                Style::default().fg(crate::lean::tui::theme::ayu::BLUE)
            } else {
                theme.dim()
            });
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        if inner.height > 0 && inner.width > 0 {
            let text = theme.fit(label, inner.width as usize);
            frame.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), inner);
        }
    }

    fn button_label_tone(&self, button: BottomButton, state: ButtonState) -> (&'static str, Tone) {
        match button {
            BottomButton::Accept => self.accept_label_tone(state),
            BottomButton::Reject => self.reject_label_tone(state.decidable),
            BottomButton::CommentWholePlan => (" [ Comment plan (C) ] ", Tone::Active),
            BottomButton::Review => {
                if state.drafts > 0 {
                    (" [ Review: send (R) ] ", Tone::Done)
                } else {
                    (" [ Review: no drafts ] ", Tone::Queued)
                }
            }
        }
    }

    fn accept_label_tone(&self, state: ButtonState) -> (&'static str, Tone) {
        let label = if !state.decidable {
            " [ Accept: decided ] "
        } else if state.drafts > 0 {
            " [ Accept: review first ] "
        } else if !state.can_accept {
            " [ Accept: resolve first ] "
        } else if self.confirm_accept {
            " [ Press again: accept ] "
        } else {
            " [ Accept (a) ] "
        };
        let tone = if state.can_accept {
            Tone::Done
        } else if state.decidable {
            Tone::Waiting
        } else {
            Tone::Queued
        };
        (label, tone)
    }

    fn reject_label_tone(&self, decidable: bool) -> (&'static str, Tone) {
        if decidable {
            (" [ Reject (x) ] ", Tone::Failed)
        } else {
            (" [ Reject: decided ] ", Tone::Queued)
        }
    }

    fn button_style(&self, theme: &Theme, tone: Tone, active: bool) -> Style {
        let style = match tone {
            Tone::Done => theme.status("approved"),
            Tone::Failed => theme.error(),
            Tone::Waiting => theme.warning(),
            Tone::Active => theme.accent(),
            Tone::Queued => theme.dim(),
        };
        if active {
            style.add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            style
        }
    }
}
