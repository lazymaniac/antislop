use super::*;

#[derive(Clone, Copy)]
struct KeyState {
    max_cursor: usize,
    decidable: bool,
    drafts: usize,
    confirming: bool,
}

impl PlanView {
    /// Handle keyboard navigation and actions in the full-screen plan view.
    pub fn key(&mut self, key: KeyEvent, view: &View) -> PlanAction {
        let lines = self.build_lines(view);
        let state = self.prepare_key(view, lines.len());
        if let Some(action) = self.control_action(key, state) {
            return action;
        }
        self.regular_action(key.code, view, &lines, state)
    }

    fn prepare_key(&mut self, view: &View, line_count: usize) -> KeyState {
        let max_cursor = line_count.saturating_sub(1);
        self.cursor = self.cursor.min(max_cursor);
        let mut state = self.action_state(view);
        state.max_cursor = max_cursor;
        state
    }

    fn action_state(&mut self, view: &View) -> KeyState {
        KeyState {
            max_cursor: self.cursor,
            decidable: view.plan_proposed(),
            drafts: view.draft_plan_comments().len(),
            confirming: std::mem::take(&mut self.confirm_accept),
        }
    }

    fn control_action(&mut self, key: KeyEvent, state: KeyState) -> Option<PlanAction> {
        if !key.modifiers.contains(KeyModifiers::CONTROL) {
            return None;
        }
        match key.code {
            KeyCode::Char('d') => {
                self.cursor = (self.cursor + 5).min(state.max_cursor);
                Some(PlanAction::None)
            }
            KeyCode::Char('u') => {
                self.cursor = self.cursor.saturating_sub(5);
                Some(PlanAction::None)
            }
            KeyCode::Char('a') if state.decidable => Some(PlanAction::Accept),
            KeyCode::Char('x' | 'r') if state.decidable => Some(PlanAction::Reject),
            _ => None,
        }
    }

    fn regular_action(
        &mut self,
        code: KeyCode,
        view: &View,
        lines: &[DocLine],
        state: KeyState,
    ) -> PlanAction {
        match code {
            KeyCode::Esc | KeyCode::Char('q') => PlanAction::Close,
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(self.cursor.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_cursor((self.cursor + 1).min(state.max_cursor))
            }
            KeyCode::PageUp => self.move_cursor(self.cursor.saturating_sub(10)),
            KeyCode::PageDown => self.move_cursor((self.cursor + 10).min(state.max_cursor)),
            KeyCode::Home | KeyCode::Char('g') => self.move_cursor(0),
            KeyCode::End | KeyCode::Char('G') => self.move_cursor(state.max_cursor),
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => self.move_button(true),
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => self.move_button(false),
            KeyCode::Char('a' | '1') if state.decidable => PlanAction::Accept,
            KeyCode::Char('x' | '2') if state.decidable => PlanAction::Reject,
            KeyCode::Char('C' | '3') => PlanAction::CommentWholePlan,
            KeyCode::Char('R' | '4') => self.activate(BottomButton::Review, state),
            KeyCode::Char('c') => line_comment(self.cursor, lines),
            KeyCode::Delete | KeyCode::Char('D') => self.delete_comment(view),
            KeyCode::Char(ch @ ('r' | 'o')) => self.toggle_comment(view, ch),
            KeyCode::Enter => self.activate(self.button, state),
            _ => PlanAction::None,
        }
    }

    fn move_cursor(&mut self, cursor: usize) -> PlanAction {
        self.cursor = cursor;
        PlanAction::None
    }

    fn move_button(&mut self, next: bool) -> PlanAction {
        self.button = if next {
            self.button.next()
        } else {
            self.button.prev()
        };
        PlanAction::None
    }

    fn delete_comment(&self, view: &View) -> PlanAction {
        self.line_comments(view, self.cursor + 1)
            .into_iter()
            .rev()
            .find(|comment| comment.status == "draft")
            .or_else(|| {
                self.line_comments(view, self.cursor + 1)
                    .into_iter()
                    .next_back()
            })
            .map(|comment| PlanAction::ToggleResolveComment {
                id: comment.id,
                next_status: "deleted".into(),
            })
            .unwrap_or(PlanAction::None)
    }

    fn toggle_comment(&self, view: &View, key: char) -> PlanAction {
        let comment = self
            .line_comments(view, self.cursor + 1)
            .into_iter()
            .find(|comment| comment.status == "open")
            .or_else(|| self.line_comments(view, self.cursor + 1).into_iter().next())
            .or_else(|| {
                self.cursor
                    .eq(&0)
                    .then(|| whole_plan_comment(self, view))
                    .flatten()
            });
        comment.map_or(PlanAction::None, |comment| {
            let next_status = if key == 'o' {
                "open".to_owned()
            } else if comment.status == "open" {
                "resolved".to_owned()
            } else {
                "open".to_owned()
            };
            PlanAction::ToggleResolveComment {
                id: comment.id,
                next_status,
            }
        })
    }

    /// A button was pressed: Accept asks for a second press first.
    fn activate(&mut self, button: BottomButton, state: KeyState) -> PlanAction {
        match button {
            BottomButton::Review if state.drafts > 0 => PlanAction::SubmitReview,
            BottomButton::Review => PlanAction::None,
            BottomButton::Accept if !state.decidable => PlanAction::None,
            BottomButton::Accept if state.confirming => PlanAction::Accept,
            BottomButton::Accept => {
                self.confirm_accept = true;
                PlanAction::None
            }
            BottomButton::Reject if state.decidable => PlanAction::Reject,
            BottomButton::Reject => PlanAction::None,
            BottomButton::CommentWholePlan => PlanAction::CommentWholePlan,
        }
    }

    /// Handle mouse click at `(col, row)`.
    pub fn mouse_click(&mut self, col: u16, row: u16, view: &View) -> PlanAction {
        let state = self.action_state(view);
        if let Some(action) = self.click_button(col, row, state) {
            return action;
        }
        self.click_document(col, row, view);
        PlanAction::None
    }

    fn click_button(&mut self, col: u16, row: u16, state: KeyState) -> Option<PlanAction> {
        let pos = ratatui::layout::Position::new(col, row);
        let buttons = [
            BottomButton::Accept,
            BottomButton::Reject,
            BottomButton::CommentWholePlan,
            BottomButton::Review,
        ];
        for (rect, button) in self.button_rects.into_iter().zip(buttons) {
            if rect.contains(pos) {
                self.button = button;
                return Some(self.activate(button, state));
            }
        }
        None
    }

    fn click_document(&mut self, col: u16, row: u16, view: &View) {
        let pos = ratatui::layout::Position::new(col, row);
        if !self.doc_area.contains(pos) {
            return;
        }
        let rel_y = usize::from(row.saturating_sub(self.doc_area.y));
        if let Some(Some(idx)) = self.row_docs.get(self.scroll + rel_y) {
            self.cursor = (*idx).min(self.build_lines(view).len().saturating_sub(1));
        }
    }
}

fn line_comment(cursor: usize, lines: &[DocLine]) -> PlanAction {
    let preview = lines
        .get(cursor)
        .map(|line| line.text.clone())
        .unwrap_or_default();
    PlanAction::CommentLine {
        line: cursor + 1,
        text_preview: preview,
    }
}

fn whole_plan_comment<'a>(plan: &PlanView, view: &'a View) -> Option<&'a Comment> {
    plan.whole_plan_comments(view)
        .into_iter()
        .find(|comment| comment.status == "open")
        .or_else(|| plan.whole_plan_comments(view).into_iter().next())
}
