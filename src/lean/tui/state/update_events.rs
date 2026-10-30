use super::super::*;
use serde_json::Value;

impl App {
    pub(super) fn apply_events(&mut self, update: Update) {
        let Update::Events {
            conversation,
            generation,
            events,
            idle,
            autonomous,
        } = update
        else {
            unreachable!("update routed to wrong handler");
        };
        self.apply_delivery_events(&conversation, &events);
        if conversation == self.conversation && generation == self.generation {
            self.apply_active_event_batch(&events, idle, autonomous);
        }
    }

    fn apply_delivery_events(&mut self, conversation: &str, events: &[Value]) {
        for event in events {
            match event["kind"].as_str() {
                Some("mail_delivered") => self.apply_mail_delivered(conversation, event),
                Some("mail_delivery_failed") => {
                    self.apply_mail_delivery_failed(conversation, event)
                }
                _ => {}
            }
        }
    }

    fn apply_mail_delivered(&mut self, conversation: &str, event: &Value) {
        for ((owner, _), composer) in self.agent_drafts.borrow_mut().iter_mut() {
            if owner != conversation || !delivery_matches(composer, event) {
                continue;
            }
            composer.delivery = Some("delivered".into());
            composer.error = None;
            composer.delivery_failure = None;
            composer.delivery_receipts.extend(
                event["payload"]["ids"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_u64),
            );
        }
    }

    fn apply_mail_delivery_failed(&mut self, conversation: &str, event: &Value) {
        for ((owner, _), composer) in self.agent_drafts.borrow_mut().iter_mut() {
            if owner != conversation || !failure_matches(composer, event) {
                continue;
            }
            composer.error = Some(format!(
                "Delivery failed; queued: {}",
                event["payload"]["reason"]
                    .as_str()
                    .unwrap_or("unknown error")
            ));
            save_delivery_failure(composer, event);
        }
    }

    fn apply_active_event_batch(&mut self, events: &[Value], idle: bool, autonomous: bool) {
        let (manual_plan, requirements_review) = self.apply_view_events(events);
        self.update_ledger_approval(requirements_review);
        self.view.idle = idle;
        self.view.autonomous = Some(autonomous);
        self.refresh_visible_ledger(events);
        self.refetch_stale_details();
        self.open_pending_plan(manual_plan);
        self.mark_stale_confirmation();
    }

    fn apply_view_events(&mut self, events: &[Value]) -> (bool, bool) {
        let mut manual_plan = false;
        let mut requirements_review = false;
        for event in events {
            manual_plan |=
                event["kind"] == "plan_proposed" && event["payload"]["automatic"] != true;
            requirements_review |= event["kind"] == "ledger_approval_required";
            self.view.apply(event);
        }
        (manual_plan, requirements_review)
    }

    fn update_ledger_approval(&mut self, required: bool) {
        if !required {
            return;
        }
        let Some(run) = self.view.requirements_approval_run.clone() else {
            return;
        };
        let view = self.ledger_view.get_or_insert_with(ledger::LedgerView::new);
        if view.approval_event == self.view.requirements_approval_event {
            return;
        }
        view.approval_event = self.view.requirements_approval_event;
        view.review_run = Some(run);
        if ledger_read_is_stale(view) {
            view.request_id = view.request_id.wrapping_add(1);
            view.pending = false;
        }
    }

    fn refresh_visible_ledger(&mut self, events: &[Value]) {
        if !events.iter().any(is_ledger_refresh_event) {
            return;
        }
        let Some(ledger) = self.ledger_view.as_mut().filter(|view| view.visible) else {
            return;
        };
        if ledger_scope_changed(ledger, events) {
            ledger.scope_epoch = ledger.scope_epoch.wrapping_add(1);
            ledger.stale = ledger.preview.is_some();
            ledger.error =
                Some("Scope changed; draft preserved. Ctrl+R reloads the proposal.".into());
        }
        if ledger.editing.is_none() && ledger.preview.is_none() {
            self.ledger_action(ledger::Action::Fetch);
        }
    }

    fn open_pending_plan(&mut self, saw_manual_plan: bool) {
        if saw_manual_plan
            && self.view.plan_proposed()
            && !self.view.tasks.is_empty()
            && self.confirmation.is_none()
        {
            let plan = self.view.plan.clone().unwrap_or_else(|| "plan".into());
            self.plan_view = Some(PlanView::new(plan));
        }
    }

    fn mark_stale_confirmation(&mut self) {
        let stale = self
            .confirmation
            .as_ref()
            .is_some_and(|card| self.is_confirmation_stale(card));
        if !stale {
            return;
        }
        let Some(mut card) = self.confirmation.take() else {
            return;
        };
        card.status = ConfirmationStatus::Stale;
        for existing in self.view.cards.iter_mut().rev() {
            if let TranscriptCard::Confirmation(existing) = existing
                && existing.id == card.id
            {
                existing.status = ConfirmationStatus::Stale;
                break;
            }
        }
        self.notice = Some("✗ stale_scope: scope changed".into());
    }
}

fn delivery_matches(composer: &details::Composer, event: &Value) -> bool {
    let ids_match = event["payload"]["ids"].as_array().is_some_and(|ids| {
        ids.iter()
            .any(|id| id.as_u64() == composer.mail && composer.mail.is_some())
    });
    let address_match = composer.sending
        && composer.address.is_some()
        && composer.address.as_deref() == event["payload"]["recipient"].as_str();
    ids_match || address_match
}

fn failure_matches(composer: &details::Composer, event: &Value) -> bool {
    composer.address.as_deref() == event["payload"]["recipient"].as_str()
        && event["payload"]["ids"].as_array().is_none_or(|ids| {
            composer.mail.is_none()
                || composer.sending
                || ids.iter().any(|id| id.as_u64() == composer.mail)
        })
}

fn save_delivery_failure(composer: &mut details::Composer, event: &Value) {
    let Some(ids) = event["payload"]["ids"].as_array() else {
        return;
    };
    let error = composer.error.clone().expect("delivery error");
    composer.delivery_failure = Some((ids.iter().filter_map(Value::as_u64).collect(), error));
}

fn ledger_read_is_stale(view: &ledger::LedgerView) -> bool {
    view.pending
        && (view.pending_read
            || !matches!(
                view.pending_kind.as_str(),
                "applied" | "rejected" | "preview" | "created"
            ))
}

fn is_ledger_refresh_event(event: &Value) -> bool {
    matches!(
        event["kind"].as_str(),
        Some(
            "ledger_changed" | "ledger_evidence" | "ledger_review" | "ledger_source_added" | "task"
        )
    )
}

fn ledger_scope_changed(view: &ledger::LedgerView, events: &[Value]) -> bool {
    let review_run = view.pending_run.as_deref().or_else(|| {
        view.proposed_change
            .as_ref()
            .map(|change| change.run_id.as_str())
    });
    events.iter().any(|event| {
        event["kind"] == "ledger_changed"
            && review_run.is_none_or(|run| event["payload"]["run"].as_str() == Some(run))
    })
}
