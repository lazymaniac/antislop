use super::super::super::*;
use crossterm::event::{KeyCode, KeyEvent};

mod navigation;

impl App {
    pub(super) fn details_primary_key(&mut self, key: KeyEvent, plain: bool) -> bool {
        if key.code == KeyCode::F(6) {
            self.run_workflow(Command::PauseOrContinue);
            return true;
        }
        if key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('x')
        {
            self.run_workflow(Command::Cancel);
            return true;
        }
        if self.details_close_or_search(key) {
            return true;
        }
        if key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL)
        {
            match key.code {
                KeyCode::Char('l') => {
                    self.run_workflow(Command::Plan);
                    return true;
                }
                KeyCode::Char('p') => {
                    self.picker = Some(Picker::new(Kind::Commands));
                    return true;
                }
                _ => {}
            }
        }
        if self.details_overview_key(key, plain) {
            return true;
        }
        if plain && (key.code == KeyCode::F(5) || key.code == KeyCode::Char('u')) {
            let target = self.details.as_ref().map(|details| details.key.clone());
            if let Some(target) = target {
                if target == "conversation" {
                    self.open_unblock(Some(crate::lean::tui::ORCHESTRATOR.into()));
                } else if let Some((_, task)) = target
                    .split_once(':')
                    .filter(|(role, _)| matches!(*role, "worker" | "reviewer"))
                {
                    self.open_unblock(Some(task.into()));
                } else {
                    self.open_unblock(Some(target));
                }
            }
            return true;
        }
        if plain
            && let KeyCode::Char(action @ '1'..='3') = key.code
            && self
                .details
                .as_ref()
                .is_some_and(|details| details.key == "conversation")
            && self.select_session_action((action as usize) - ('1' as usize))
        {
            return true;
        }
        if plain
            && key.code == KeyCode::Char('p')
            && self
                .details
                .as_ref()
                .is_some_and(|d| d.tab == details::PLAN)
        {
            self.run_workflow(Command::Plan);
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
                let tab = self.navigation.presentation().tab;
                details.save_current_tab();
                details.workbench_memory.insert(
                    (details.key.clone(), tab),
                    (details.scroll, details.query.clone()),
                );
                for ((key, tab), (scroll, query)) in &details.workbench_memory {
                    if key == &details.key {
                        // Keep Workbench slots outside the legacy Details tab range (0..7).
                        let slot = usize::MAX - *tab as usize;
                        self.details_scroll_memory
                            .insert((key.clone(), slot), *scroll);
                        self.details_query_memory
                            .insert((key.clone(), slot), query.clone());
                    }
                }
                self.details_scroll_memory
                    .extend(details.scroll_memory.clone());
                self.details_query_memory
                    .extend(details.query_memory.clone());
                self.remember_workbench();
                self.details = None;
                self.navigation.screen = Screen::Graph;
                self.set_focus(Focus::Graph);
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
        // The conversation has no session to swap its information for.
        let session = details.key != "conversation";
        if key.code != KeyCode::Char('m')
            || !details.overview()
            || !plain
            || details.split
            || !session
        {
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

#[cfg(test)]
mod workbench_memory_tests {
    use super::*;
    use crate::lean::tui::WorkbenchTab;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn closing_details_persists_each_workbench_tab_state() {
        let mut app = App::test_app();
        app.details = Some(Details::new("worker:t1".into()));
        let details = app.details.as_mut().unwrap();
        details.workbench_memory.insert(
            ("worker:t1".into(), WorkbenchTab::Summary),
            (3, "summary query".into()),
        );
        details.workbench_memory.insert(
            ("worker:t1".into(), WorkbenchTab::Conversation),
            (8, "conversation query".into()),
        );
        details.scroll = 13;
        details.query = "diff query".into();
        app.navigation
            .update_presentation(|presentation| presentation.tab = WorkbenchTab::Diff);

        assert!(app.details_close_or_search(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(app.details.is_none());
        for (tab, expected) in [
            (WorkbenchTab::Diff, (13, "diff query")),
            (WorkbenchTab::Summary, (3, "summary query")),
            (WorkbenchTab::Conversation, (8, "conversation query")),
        ] {
            let slot = usize::MAX - tab as usize;
            assert_eq!(
                app.details_scroll_memory.get(&("worker:t1".into(), slot)),
                Some(&expected.0)
            );
            assert_eq!(
                app.details_query_memory
                    .get(&("worker:t1".into(), slot))
                    .map(String::as_str),
                Some(expected.1)
            );
        }
    }
}
