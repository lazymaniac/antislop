use super::super::*;
use crate::lean::daemon::ConversationRequest;

impl App {
    pub fn open_diff(&mut self, key: &str) {
        self.default_diff_request = None;
        if !key.contains(':') {
            return;
        }
        if self.graph_expanded.is_none() {
            if self.navigation.screen != Screen::Workbench
                || self
                    .details
                    .as_ref()
                    .is_none_or(|details| details.key != key)
            {
                self.open_details(key.to_owned());
                if self.diff.is_some() && self.navigation.presentation().tab == WorkbenchTab::Diff {
                    return;
                }
            }
            self.result_fetch.cancel_document_fetch();
            let previous = self.navigation.presentation().tab;
            self.navigation.update_presentation(|presentation| {
                presentation.tab = WorkbenchTab::Diff;
                if presentation.terminal_only_default {
                    presentation.terminal_only_default = false;
                    presentation.split = true;
                }
                presentation.focus = NavigationFocus::WorkbenchContent;
            });
            self.navigation.focus = NavigationFocus::WorkbenchContent;
            if let Some(details) = &mut self.details {
                details.prepare_workbench_details(Some(previous), WorkbenchTab::Diff);
            }
        }
        self.load_diff(key);
    }

    pub(in crate::lean::tui) fn load_diff(&mut self, key: &str) {
        if let Some((_, task)) = key.split_once(':') {
            self.diff = Some(
                self.diff_states
                    .get(&(self.conversation.clone(), key.to_owned()))
                    .cloned()
                    .unwrap_or_else(|| DiffView::new(task.to_owned())),
            );
            self.fetch_diff();
        }
    }

    pub(in crate::lean::tui) fn fetch_diff(&mut self) {
        let (Some(diff), Some(plan)) = (&self.diff, &self.view.plan) else {
            return;
        };
        self.diff_request_id += 1;
        let request_id = self.diff_request_id;
        let request = Request::Conversation(ConversationRequest::Diff {
            conversation: self.conversation.clone(),
            plan: plan.clone(),
            id: diff.task.clone(),
        });
        let task = diff.task.clone();
        let plan_id = plan.clone();
        let conversation = self.conversation.clone();
        let generation = self.generation;
        let client = self.client.clone();
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let reply = client.call(&request).map_err(|error| error.to_string());
            let _ = updates.send(Update::Diff {
                conversation,
                plan: Some(plan_id),
                generation,
                request_id,
                task,
                reply,
            });
        });
    }

    pub(in crate::lean::tui) fn historical_diff_selected(&self) -> bool {
        let Some(details) = self.details.as_ref() else {
            return false;
        };
        let key = if let Some(key) = &self.graph_expanded {
            key.as_str()
        } else if self.navigation.screen == Screen::Workbench {
            let WorkbenchTarget::Agent(key) = &self.navigation.target else {
                return false;
            };
            key.as_str()
        } else {
            return false;
        };
        let Some((_, task_id)) = key.split_once(':') else {
            return false;
        };
        details.key == key
            && details.selected_round.is_some()
            && self.diff.as_ref().is_some_and(|diff| diff.task == task_id)
    }

    pub(in crate::lean::tui) fn diff_key(&mut self, key: event::KeyEvent) {
        let historical_diff = self.historical_diff_selected();
        let Some(diff) = &mut self.diff else {
            return;
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.remember_workbench();
                self.diff = None;
                if self.navigation.screen == Screen::Workbench && self.graph_expanded.is_none() {
                    self.navigation.update_presentation(|presentation| {
                        presentation.tab = WorkbenchTab::Summary;
                    });
                    if let Some(details) = &mut self.details {
                        details.prepare_workbench_details(
                            Some(WorkbenchTab::Diff),
                            WorkbenchTab::Summary,
                        );
                    }
                }
            }
            KeyCode::Char('?') => self.show_help(Help::Diff),
            KeyCode::Char('c') => {
                if historical_diff {
                    self.notice =
                        Some("No historical diff is retained for the selected round.".into());
                    return;
                }
                if let Some(target) = diff.target() {
                    let input = self.get_comment_draft(&target);
                    self.editor = Some(Editor { target, input });
                }
            }
            KeyCode::Char('r') if historical_diff => {
                self.notice = Some("No historical diff is retained for the selected round.".into());
            }
            KeyCode::Char('r') => self.fetch_diff(),
            KeyCode::Char('R') => {
                if historical_diff {
                    self.notice =
                        Some("No historical diff is retained for the selected round.".into());
                    return;
                }
                if let Some(plan) = self.view.plan.clone() {
                    let request = Request::Conversation(ConversationRequest::Review {
                        conversation: self.conversation.clone(),
                        plan,
                        id: diff.task.clone(),
                        revision_token: None,
                    });
                    self.request(request, Some("code review started"));
                }
            }
            KeyCode::Char(ch @ ('u' | 'd')) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                diff.half_page(ch == 'd');
            }
            code => diff.key(code),
        }
    }
}
