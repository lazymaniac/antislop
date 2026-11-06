use super::super::*;
use crate::lean::daemon::ConversationRequest;

impl App {
    pub fn open_diff(&mut self, key: &str) {
        if let Some((_, task)) = key.split_once(':') {
            self.diff = Some(DiffView::new(task.to_owned()));
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

    pub(in crate::lean::tui) fn diff_key(&mut self, key: event::KeyEvent) {
        let Some(diff) = &mut self.diff else {
            return;
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.diff = None,
            KeyCode::Char('?') => self.show_help(Help::Diff),
            KeyCode::Char('c') => {
                if self.graph_expanded.is_some()
                    && self.graph_tab == 3
                    && self
                        .details
                        .as_ref()
                        .is_some_and(|details| details.selected_round.is_some())
                {
                    self.notice =
                        Some("No historical diff is retained for the selected round.".into());
                    return;
                }
                if let Some(target) = diff.target() {
                    let input = self.get_comment_draft(&target);
                    self.editor = Some(Editor { target, input });
                }
            }
            KeyCode::Char('r') => self.fetch_diff(),
            KeyCode::Char('R') => {
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
