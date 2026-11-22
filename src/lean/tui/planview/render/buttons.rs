use super::super::*;

#[derive(Clone, Copy)]
struct ButtonState {
    drafts: usize,
    decidable: bool,
    acceptable: bool,
}

impl PlanView {
    pub(super) fn draw_buttons(
        &mut self,
        frame: &mut Frame,
        bar_area: Rect,
        view: &View,
        theme: &Theme,
    ) {
        let actions = Rect::new(
            bar_area.x,
            bar_area.y + u16::from(bar_area.height >= 3),
            bar_area.width,
            u16::from(!bar_area.is_empty()),
        );
        let btn_cols = self.button_areas(actions);
        let drafts = view.draft_plan_comments().len();
        let decidable = view.plan_proposed();
        let acceptable = decidable && view.open_plan_comments().is_empty();
        let state = ButtonState {
            drafts,
            decidable,
            acceptable,
        };
        self.draw_button_group(frame, btn_cols, theme, state);
        if bar_area.height > 1 {
            let hints = fit_hint(
                &[
                    "v pane",
                    "↑/↓ select",
                    "c comment line",
                    "Tab action",
                    "Esc back",
                ],
                usize::from(bar_area.width),
                true,
            );
            frame.render_widget(
                Paragraph::new(theme.chrome(&hints)).style(theme.dim()),
                Rect::new(bar_area.x, bar_area.bottom() - 1, bar_area.width, 1),
            );
        }
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
        let (label, short, tone) = self.button_label_tone(button, state);
        let active = self.button == button;
        let style = self.button_style(theme, tone, active);
        frame.render_widget(Block::default().style(style), rect);
        let room = usize::from(rect.width.saturating_sub(2));
        let label = if unicode_width::UnicodeWidthStr::width(label) <= room {
            label
        } else if unicode_width::UnicodeWidthStr::width(short) <= room {
            short
        } else {
            match button {
                BottomButton::Accept => "a",
                BottomButton::Reject => "x",
                BottomButton::CommentWholePlan => "C",
                BottomButton::Review => "R",
            }
        };
        frame.render_widget(
            Paragraph::new(theme.fit(label, room))
                .style(style)
                .alignment(ratatui::layout::Alignment::Center),
            rect.inner(ratatui::layout::Margin::new(u16::from(rect.width >= 2), 0)),
        );
    }

    /// The full label and the short one for a narrow button.
    fn button_label_tone(
        &self,
        button: BottomButton,
        state: ButtonState,
    ) -> (&'static str, &'static str, Tone) {
        match button {
            BottomButton::Accept if state.acceptable => ("a accept", "a accept", Tone::Done),
            BottomButton::Accept => ("a accept", "a accept", Tone::Queued),
            BottomButton::Reject if state.decidable => ("x reject", "x reject", Tone::Failed),
            BottomButton::Reject => ("x reject", "x reject", Tone::Queued),
            BottomButton::CommentWholePlan => ("C comment plan", "C comment", Tone::Active),
            BottomButton::Review if state.drafts > 0 => ("R send review", "R review", Tone::Done),
            BottomButton::Review => ("R no drafts", "R review", Tone::Queued),
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
            theme.selected().patch(style).add_modifier(Modifier::BOLD)
        } else {
            theme.card().patch(style)
        }
    }
}
