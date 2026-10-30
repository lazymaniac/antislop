use super::super::*;

impl App {
    pub fn request_confirmation(&mut self, mut card: ConfirmationCard) {
        if card.scope_hash.is_empty() {
            card.scope_hash = self.confirmation_scope_hash(&card);
        }
        self.confirmation = Some(card.clone());
        self.view.cards.push(TranscriptCard::Confirmation(card));
    }

    fn confirmation_scope_hash(&self, card: &ConfirmationCard) -> String {
        match card.action {
            ConfirmationAction::AcceptPlan | ConfirmationAction::ResumePlan => self
                .view
                .plan_scope_hash
                .clone()
                .or_else(|| card.plan_id.clone())
                .unwrap_or_default(),
            ConfirmationAction::ReviewDiff => card
                .task_id
                .as_ref()
                .and_then(|id| self.view.tasks.iter().find(|task| &task.id == id))
                .and_then(|task| task.scope_hash.clone())
                .unwrap_or_default(),
            ConfirmationAction::CancelWork => self
                .view
                .conversation_scope_hash
                .clone()
                .unwrap_or_default(),
        }
    }

    pub fn is_confirmation_stale(&self, card: &ConfirmationCard) -> bool {
        match card.action {
            ConfirmationAction::AcceptPlan | ConfirmationAction::ResumePlan => {
                self.plan_confirmation_stale(card)
            }
            ConfirmationAction::ReviewDiff => self.diff_confirmation_stale(card),
            ConfirmationAction::CancelWork => {
                scope_changed(&self.view.conversation_scope_hash, &card.scope_hash)
            }
        }
    }

    fn plan_confirmation_stale(&self, card: &ConfirmationCard) -> bool {
        let plan_changed = card
            .plan_id
            .as_ref()
            .is_some_and(|plan| self.view.plan.as_ref() != Some(plan));
        plan_changed || scope_changed(&self.view.plan_scope_hash, &card.scope_hash)
    }

    fn diff_confirmation_stale(&self, card: &ConfirmationCard) -> bool {
        if card
            .plan_id
            .as_ref()
            .is_some_and(|plan| self.view.plan.as_ref() != Some(plan))
        {
            return true;
        }
        let current = card
            .task_id
            .as_ref()
            .and_then(|id| self.view.tasks.iter().find(|task| &task.id == id))
            .and_then(|task| task.scope_hash.as_ref());
        scope_changed(&current.cloned(), &card.scope_hash)
    }

    pub fn confirm_action(&mut self) {
        if let Some(mut card) = self.confirmation.take() {
            if self.is_confirmation_stale(&card) {
                self.finish_confirmation(&mut card, ConfirmationStatus::Stale);
                self.notice = Some("✗ stale_scope: scope changed, action rejected".into());
                return;
            }
            self.finish_confirmation(&mut card, ConfirmationStatus::Confirmed);
            self.send_confirmation(card);
        }
    }

    fn finish_confirmation(&mut self, card: &mut ConfirmationCard, status: ConfirmationStatus) {
        card.status = status;
        if let Some(TranscriptCard::Confirmation(existing)) =
            self.view.cards.iter_mut().rev().find(
                |item| matches!(item, TranscriptCard::Confirmation(found) if found.id == card.id),
            )
        {
            existing.status = status;
        }
    }

    fn send_confirmation(&self, card: ConfirmationCard) {
        let conversation = self.conversation.clone();
        let scope_hash = Some(card.scope_hash.clone());
        let request = confirmation_request(card, conversation, scope_hash);
        if let Some((request, notice)) = request {
            self.request(request, Some(notice));
        }
    }

    pub fn reject_confirmation(&mut self) {
        if let Some(mut card) = self.confirmation.take() {
            card.status = ConfirmationStatus::Rejected;
            for c in self.view.cards.iter_mut().rev() {
                if let TranscriptCard::Confirmation(existing) = c
                    && existing.id == card.id
                {
                    existing.status = ConfirmationStatus::Rejected;
                    break;
                }
            }
            self.notice = Some("Action rejected".into());
        }
    }

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
        let request = Request::Diff {
            conversation: self.conversation.clone(),
            plan: plan.clone(),
            id: diff.task.clone(),
        };
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
                if let Some(target) = diff.target() {
                    let input = self.get_comment_draft(&target);
                    self.editor = Some(Editor { target, input });
                }
            }
            KeyCode::Char('r') => self.fetch_diff(),
            KeyCode::Char('R') => {
                if let Some(plan) = self.view.plan.clone() {
                    let request = Request::Review {
                        conversation: self.conversation.clone(),
                        plan,
                        id: diff.task.clone(),
                    };
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

fn confirmation_request(
    card: ConfirmationCard,
    conversation: String,
    scope_hash: Option<String>,
) -> Option<(Request, &'static str)> {
    match card.action {
        ConfirmationAction::AcceptPlan => accept_request(card.plan_id, conversation, scope_hash),
        ConfirmationAction::ResumePlan => resume_request(card.plan_id, conversation, scope_hash),
        ConfirmationAction::ReviewDiff => {
            review_request(card.plan_id, card.task_id, conversation, scope_hash)
        }
        ConfirmationAction::CancelWork => Some((
            Request::CancelScoped {
                conversation,
                scope_hash,
            },
            "cancelled",
        )),
    }
}

fn accept_request(
    plan: Option<String>,
    conversation: String,
    scope_hash: Option<String>,
) -> Option<(Request, &'static str)> {
    plan.map(|plan| {
        (
            Request::AcceptScoped {
                conversation,
                plan,
                scope_hash,
            },
            "plan accepted",
        )
    })
}

fn resume_request(
    plan: Option<String>,
    conversation: String,
    scope_hash: Option<String>,
) -> Option<(Request, &'static str)> {
    plan.map(|plan| {
        (
            Request::ResumeScoped {
                conversation,
                plan,
                scope_hash,
            },
            "plan resumed",
        )
    })
}

fn review_request(
    plan: Option<String>,
    task: Option<String>,
    conversation: String,
    scope_hash: Option<String>,
) -> Option<(Request, &'static str)> {
    plan.zip(task).map(|(plan, id)| {
        (
            Request::ReviewScoped {
                conversation,
                plan,
                id,
                scope_hash,
            },
            "changes approved",
        )
    })
}

fn scope_changed(current: &Option<String>, expected: &str) -> bool {
    current
        .as_ref()
        .is_some_and(|current| !expected.is_empty() && current != expected)
}
