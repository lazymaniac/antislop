use super::super::super::*;
use crossterm::event::{KeyCode, KeyEvent};

mod navigation;

impl App {
    pub(super) fn details_primary_key(&mut self, key: KeyEvent, plain: bool) -> bool {
        if self.details_close_or_search(key) {
            return true;
        }
        if self.details_overview_key(key, plain) {
            return true;
        }
        if key.code == KeyCode::Char('?') {
            self.show_help(Help::Details);
            return true;
        }
        if key.code == KeyCode::Char('c') {
            self.details_open_comment();
            return true;
        }
        if self.details_comment_status(key) {
            return true;
        }
        false
    }

    fn details_close_or_search(&mut self, key: KeyEvent) -> bool {
        let Some(details) = &mut self.details else {
            return false;
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                details.save_current_tab();
                self.details_scroll_memory
                    .extend(details.scroll_memory.clone());
                self.details_query_memory
                    .extend(details.query_memory.clone());
                self.details = None;
                true
            }
            KeyCode::Char('/') if !details.overview() || details.dashboard_visible() => {
                details.searching = true;
                details.follow = false;
                details.query.clear();
                details.scroll = 0;
                true
            }
            _ => false,
        }
    }

    fn details_overview_key(&mut self, key: KeyEvent, plain: bool) -> bool {
        if self.details_native_focus_key(key, plain) {
            return true;
        }
        self.details_resume_round(key, plain)
            || self.details_retry_native(key, plain)
            || self.details_start_composer(key, plain)
            || self.details_toggle_info(key, plain)
    }

    fn details_resume_round(&mut self, key: KeyEvent, plain: bool) -> bool {
        let Some(details) = &mut self.details else {
            return false;
        };
        if key.code != KeyCode::Char('o')
            || !details.overview()
            || !plain
            || details.selected_round.is_none()
        {
            return false;
        }
        details.resume_round = true;
        true
    }

    fn details_retry_native(&mut self, key: KeyEvent, plain: bool) -> bool {
        let Some(details) = &self.details else {
            return false;
        };
        if key.code != KeyCode::Char('r') || !details.overview() || !plain {
            return false;
        }
        self.native_retry();
        true
    }

    fn details_start_composer(&mut self, key: KeyEvent, plain: bool) -> bool {
        let Some(details) = &mut self.details else {
            return false;
        };
        if !matches!(key.code, KeyCode::Char('i' | 'a')) || !details.overview() || !plain {
            return false;
        }
        if details.selected_round.is_none()
            && (details.task_id().is_none() || details.task_uuid.is_some())
        {
            details.editing = true;
            let key = details.composer_key();
            details.drafts.borrow_mut().entry(key).or_default();
        }
        true
    }

    fn details_toggle_info(&mut self, key: KeyEvent, plain: bool) -> bool {
        let Some(details) = &mut self.details else {
            return false;
        };
        if key.code != KeyCode::Char('m') || !details.overview() || !plain || details.split {
            return false;
        }
        details.show_info = !details.show_info;
        details.scroll = 0;
        details.follow = !details.show_info;
        true
    }

    fn details_native_focus_key(&mut self, key: KeyEvent, plain: bool) -> bool {
        let Some(details) = &mut self.details else {
            return false;
        };
        if key.code != KeyCode::Char('i') || !details.overview() || !plain {
            return false;
        }
        let ready = details
            .native
            .get(&details.session_keys()[details.active_panel])
            .is_some_and(|s| {
                s.error.is_none()
                    && s.screen
                        .as_ref()
                        .is_some_and(|screen| screen.error.is_none())
            });
        if !ready {
            return false;
        }
        details.native_focus = true;
        details.editing = false;
        true
    }

    fn details_open_comment(&mut self) {
        if let Some(details) = &self.details {
            let target = match (details.selected_check(), details.task_id()) {
                (Some(check), Some(task)) => format!("check:{task}:{check}"),
                _ => details.key.clone(),
            };
            let input = self.get_comment_draft(&target);
            self.editor = Some(Editor { target, input });
        }
    }

    fn details_comment_status(&mut self, key: KeyEvent) -> bool {
        let Some(details) = &self.details else {
            return false;
        };
        let Some(comment) = details.selected_comment(&self.view) else {
            return false;
        };
        let KeyCode::Char(ch @ ('r' | 'o' | 'x')) = key.code else {
            return false;
        };
        let status = match ch {
            'r' => "resolved",
            'o' => "open",
            _ => "deleted",
        };
        self.set_comment(comment, status);
        true
    }
}
