use super::super::*;
mod actions;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl App {
    pub(super) fn details_key(&mut self, key: event::KeyEvent) {
        if self.details_edit_key(key) || self.details_search_key(key) {
            return;
        }
        self.details_screen_key(key);
    }

    fn details_screen_key(&mut self, key: KeyEvent) {
        let Some(details) = &mut self.details else {
            return;
        };
        let plain = !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        if key.code == KeyCode::Char('g') && plain {
            if std::mem::take(&mut details.pending_g) {
                details.follow = false;
                details.scroll = 0;
                details.check = 0;
                details.comment = 0;
                if details.dashboard_visible() {
                    details.dash_jump(false);
                }
            } else {
                details.pending_g = true;
            }
            return;
        }
        details.pending_g = false;
        if self.details_route_action(key, plain) {
            self.details_fetch_current_task();
            return;
        }
        self.details_fetch_current_task();
    }

    fn details_route_action(&mut self, key: KeyEvent, plain: bool) -> bool {
        self.details_primary_key(key, plain)
            || self.details_dashboard_key(key, plain)
            || self.details_tab_key(key)
            || self.details_content_key(key, plain)
    }

    fn details_fetch_current_task(&mut self) {
        if self
            .details
            .as_ref()
            .is_some_and(|d| d.tab == RESULT || d.tab == CHECKS)
        {
            self.fetch_task();
        }
    }

    fn details_edit_key(&mut self, key: KeyEvent) -> bool {
        let Some(details) = &mut self.details else {
            return false;
        };
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        if !details.editing || (control && key.code == KeyCode::Char('w')) {
            return false;
        }
        if key.code == KeyCode::Esc {
            details.editing = false;
            return true;
        }
        if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::ALT) {
            self.submit_agent_message();
            return true;
        }
        let width = details.input_width[details.active_panel];
        let draft_key = details.composer_key();
        let mut drafts = details.drafts.borrow_mut();
        let input = &mut drafts.entry(draft_key).or_default().input;
        match key.code {
            KeyCode::Up => input.move_up(width),
            KeyCode::Down => input.move_down(width),
            _ => input.edit_key(key),
        }
        true
    }

    fn details_search_key(&mut self, key: KeyEvent) -> bool {
        let Some(details) = &mut self.details else {
            return false;
        };
        if !details.searching {
            return false;
        }
        details.pending_g = false;
        details.follow = false;
        match key.code {
            KeyCode::Esc => {
                details.searching = false;
                details.query.clear();
                details.scroll = 0;
            }
            KeyCode::Enter => details.searching = false,
            KeyCode::Backspace => Self::remove_search_grapheme(details),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                details.query.push(c);
                details.scroll = 0;
            }
            _ => {}
        }
        true
    }

    fn remove_search_grapheme(details: &mut Details) {
        if let Some((index, _)) = unicode_segmentation::UnicodeSegmentation::grapheme_indices(
            details.query.as_str(),
            true,
        )
        .next_back()
        {
            details.query.truncate(index);
        }
        details.scroll = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::*;
    use crate::lean::tui::view::TaskRow;

    fn press(app: &mut App, code: KeyCode) {
        app.details_key(event::KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn detail_search_deletes_one_grapheme_and_keeps_the_query() {
        let mut app = App::test_app();
        let mut details = Details::new("worker:task".into());
        details.searching = true;
        details.query = "👩‍💻x".into();
        app.details = Some(details);

        press(&mut app, KeyCode::Backspace);

        let details = app.details.as_ref().unwrap();
        assert_eq!(details.query, "👩‍💻");
        assert!(details.searching);
    }

    #[test]
    fn dashboard_navigation_and_fold_actions_stay_on_the_selected_card() {
        let mut app = App::test_app();
        let mut details = Details::new("worker:task".into());
        details.show_info = true;
        details.dash_ids = vec!["goal", "checks"];
        app.details = Some(details);

        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Enter);

        let details = app.details.as_ref().unwrap();
        assert_eq!(details.dash_focus, 1);
        assert_eq!(details.fold_of("checks"), dashboard::Fold::Full);
    }

    #[test]
    fn detail_tab_shortcut_uses_the_agent_specific_tab_order() {
        let mut app = App::test_app();
        app.details = Some(Details::new("worker:task".into()));

        press(&mut app, KeyCode::Char('5'));

        assert_eq!(app.details.as_ref().unwrap().tab, details::CHECKS);
    }

    #[test]
    fn round_navigation_fetches_the_selected_round_once() {
        let mut app = App::test_app();
        app.view.plan = Some("plan".into());
        app.view.tasks.push(TaskRow {
            id: "task".into(),
            round: 3,
            ..TaskRow::default()
        });
        let mut details = Details::new("worker:task".into());
        details.tab = details::RESULT;
        details.available_rounds = vec![1, 2, 3];
        details.selected_round = Some(1);
        app.details = Some(details);

        press(&mut app, KeyCode::Right);

        assert_eq!(app.details.as_ref().unwrap().selected_round, Some(2));
        assert_eq!(app.task_request_id, 1);
        assert_eq!(app.details.as_ref().unwrap().fetched_round, Some(2));
    }
}
