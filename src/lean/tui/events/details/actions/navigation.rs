use super::super::super::super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl App {
    pub(in crate::lean::tui::events::details) fn details_dashboard_key(
        &mut self,
        key: KeyEvent,
        plain: bool,
    ) -> bool {
        let Some(details) = &mut self.details else {
            return false;
        };
        if !details.dashboard_visible() {
            return false;
        }
        match key.code {
            KeyCode::Up => details.dash_move(-1),
            KeyCode::Down => details.dash_move(1),
            KeyCode::Char('k') if plain => details.dash_move(-1),
            KeyCode::Char('j') if plain => details.dash_move(1),
            KeyCode::Enter | KeyCode::Char(' ') => details.dash_cycle(),
            KeyCode::Char('e') if plain => details.dash_expand_all(),
            KeyCode::PageDown => details.dash_scroll(10),
            KeyCode::PageUp => details.dash_scroll(-10),
            _ => return false,
        }
        true
    }

    pub(in crate::lean::tui::events::details) fn details_tab_key(&mut self, key: KeyEvent) -> bool {
        let Some(details) = &mut self.details else {
            return false;
        };
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('w') {
            if details.task_id().is_some() {
                let other = if details.tab == details::VERIFIER {
                    0
                } else {
                    details::VERIFIER
                };
                details.switch_tab(other);
            }
            return true;
        }
        match key.code {
            KeyCode::Char(c @ '1'..='7') => {
                if let Some(tab) = details.nth_tab((c as usize) - ('1' as usize)) {
                    details.switch_tab(tab);
                }
            }
            KeyCode::Tab | KeyCode::Char(']') => details.switch_tab(details.neighbour_tab(1)),
            KeyCode::BackTab | KeyCode::Char('[') => details.switch_tab(details.neighbour_tab(-1)),
            _ => return false,
        }
        true
    }

    pub(in crate::lean::tui::events::details) fn details_content_key(
        &mut self,
        key: KeyEvent,
        plain: bool,
    ) -> bool {
        if self.details_check_comment_key(key) {
            return true;
        }
        if self.details_round_key(key) || self.details_control_scroll_key(key) {
            return true;
        }
        self.details_scroll_key(key, plain)
    }

    fn details_check_comment_key(&mut self, key: KeyEvent) -> bool {
        let Some(details) = &mut self.details else {
            return false;
        };
        match key.code {
            KeyCode::Down if details.tab == CHECKS => {
                details.check = (details.check + 1).min(details.check_count().saturating_sub(1));
                details.reveal_check = true;
            }
            KeyCode::Up if details.tab == CHECKS => {
                details.check = details.check.saturating_sub(1);
                details.reveal_check = true;
            }
            KeyCode::Down if details.tab == COMMENTS => details.comment += 1,
            KeyCode::Up if details.tab == COMMENTS => {
                details.comment = details.comment.saturating_sub(1)
            }
            _ => return false,
        }
        true
    }

    fn details_round_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => return false,
            KeyCode::Char('d') => {
                if let Some(details) = &self.details {
                    self.open_diff(&details.key.clone());
                    return true;
                }
            }
            KeyCode::Right | KeyCode::Left => {
                if let Some(details) = &mut self.details {
                    details
                        .change_round(&self.view, if key.code == KeyCode::Right { 1 } else { -1 });
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    fn details_control_scroll_key(&mut self, key: KeyEvent) -> bool {
        let Some(details) = &mut self.details else {
            return false;
        };
        if !key.modifiers.contains(KeyModifiers::CONTROL) {
            return false;
        }
        match key.code {
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                details.scroll = details.scroll.saturating_add(10)
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                details.follow = false;
                details.scroll = details.scroll.saturating_sub(10);
            }
            _ => return false,
        }
        true
    }

    fn details_scroll_key(&mut self, key: KeyEvent, plain: bool) -> bool {
        let Some(details) = &mut self.details else {
            return false;
        };
        match key.code {
            KeyCode::Down => details.scroll = details.scroll.saturating_add(1),
            KeyCode::Up => {
                details.follow = false;
                details.scroll = details.scroll.saturating_sub(1);
            }
            KeyCode::PageDown => details.scroll = details.scroll.saturating_add(20),
            KeyCode::PageUp => {
                details.follow = false;
                details.scroll = details.scroll.saturating_sub(20);
            }
            KeyCode::Home => {
                details.follow = false;
                details.scroll = 0;
            }
            KeyCode::End | KeyCode::Char('G') => return self.details_jump_key(plain),
            _ => return false,
        }
        true
    }

    fn details_jump_key(&mut self, plain: bool) -> bool {
        if !plain {
            return false;
        }
        let Some(details) = &mut self.details else {
            return false;
        };
        if details.dashboard_visible() {
            details.dash_jump(true);
            return true;
        }
        details.follow = details.overview() && !details.show_info;
        details.scroll = usize::MAX;
        if details.tab == CHECKS {
            details.check = details.check_count().saturating_sub(1);
        }
        if details.tab == COMMENTS {
            details.comment = self.view.comments_on(&details.key).len().saturating_sub(1);
        }
        true
    }
}
