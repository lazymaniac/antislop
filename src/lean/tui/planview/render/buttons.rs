use super::super::*;

#[derive(Clone, Copy)]
struct ButtonState {
    drafts: usize,
    decidable: bool,
}

impl PlanView {
    pub(super) fn draw_buttons(
        &mut self,
        frame: &mut Frame,
        bar_area: Rect,
        view: &View,
        theme: &Theme,
    ) {
        let btn_cols = self.button_areas(bar_area);
        let drafts = view.draft_plan_comments().len();
        let decidable = view.plan_proposed();
        let state = ButtonState { drafts, decidable };
        self.draw_button_group(frame, btn_cols, theme, state);
    }

    fn draw_button_group(
        &self,
        frame: &mut Frame,
        areas: [Rect; 3],
        theme: &Theme,
        state: ButtonState,
    ) {
        for (index, button) in [
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

    fn button_areas(&mut self, bar_area: Rect) -> [Rect; 3] {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(33); 3])
            .split(bar_area);
        self.button_rects = [cols[0], cols[1], cols[2]];
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
