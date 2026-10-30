use super::*;

impl LedgerView {
    pub(super) fn preview_key(&mut self, key: KeyEvent, ctrl: bool) -> Action {
        if self.pending {
            return Action::None;
        }
        if ctrl
            && key.code == KeyCode::Char('r')
            && let Some(change) = &self.proposed_change
        {
            return Action::Review(change.run_id.clone());
        }
        if self.rejecting {
            return self.rejection_key(key, ctrl);
        }
        self.review_display_key(key, ctrl)
    }

    fn rejection_key(&mut self, key: KeyEvent, ctrl: bool) -> Action {
        match key.code {
            KeyCode::Esc => self.rejecting = false,
            KeyCode::Enter if ctrl && !self.stale => return self.reject_action(),
            KeyCode::Enter => self.rejection.insert_char('\n'),
            KeyCode::Up => self
                .rejection
                .move_up(self.area.width.saturating_sub(2) as usize),
            KeyCode::Down => self
                .rejection
                .move_down(self.area.width.saturating_sub(2) as usize),
            _ => self.rejection.edit_key(key),
        }
        Action::None
    }

    fn reject_action(&mut self) -> Action {
        if self.rejection.text.trim().is_empty() {
            self.error = Some("Rejection requires a comment".into());
            return Action::None;
        }
        let (Some(preview), Some(change)) = (&self.preview, &self.proposed_change) else {
            return Action::None;
        };
        Action::Reject {
            proposal: preview["proposal_id"].as_str().unwrap_or_default().into(),
            hash: preview["diff_hash"].as_str().unwrap_or_default().into(),
            run: change.run_id.clone(),
            comment: self.rejection.text.clone(),
        }
    }

    fn review_display_key(&mut self, key: KeyEvent, ctrl: bool) -> Action {
        match key.code {
            KeyCode::Esc => {
                self.save_rejection_draft();
                self.preview = None;
                self.visible = false;
            }
            KeyCode::Char('r') if !ctrl && !self.stale => self.rejecting = true,
            KeyCode::Enter if !self.stale => return self.apply_action(),
            KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
            KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::PageDown => {
                self.scroll = self
                    .scroll
                    .saturating_add(self.area.height.saturating_sub(8) as usize)
            }
            KeyCode::PageUp => {
                self.scroll = self
                    .scroll
                    .saturating_sub(self.area.height.saturating_sub(8) as usize)
            }
            _ => {}
        }
        Action::None
    }

    fn apply_action(&self) -> Action {
        let (Some(preview), Some(change)) = (&self.preview, &self.proposed_change) else {
            return Action::None;
        };
        Action::Apply {
            proposal: preview["proposal_id"].as_str().unwrap_or_default().into(),
            hash: preview["diff_hash"].as_str().unwrap_or_default().into(),
            change: change.clone(),
        }
    }
}
