use super::super::*;
use serde_json::Value;

impl App {
    pub(super) fn apply_events(&mut self, update: Update) {
        let Update::Events {
            conversation,
            generation,
            events,
            idle,
            observer,
        } = update
        else {
            unreachable!("update routed to wrong handler");
        };
        self.apply_delivery_events(&conversation, &events);
        if conversation == self.conversation && generation == self.generation {
            self.observer = observer;
            self.apply_active_event_batch(&events, idle);
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
            composer.delivery = Some(
                match event["payload"]["state"].as_str() {
                    Some("steered") => "steered into the running turn",
                    _ => "delivered",
                }
                .into(),
            );
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

    fn apply_active_event_batch(&mut self, events: &[Value], idle: bool) {
        let manual_plan = self.apply_view_events(events);
        self.view.idle = idle;
        self.refetch_stale_details();
        self.open_pending_plan(manual_plan);
    }

    fn apply_view_events(&mut self, events: &[Value]) -> bool {
        let mut manual_plan = false;
        for event in events {
            manual_plan |=
                event["kind"] == "plan_proposed" && event["payload"]["automatic"] != true;
            self.view.apply(event);
        }
        manual_plan
    }

    fn open_pending_plan(&mut self, saw_manual_plan: bool) {
        if let (Some(plan_view), Some(plan)) = (&mut self.plan_view, &self.view.plan)
            && plan_view.plan_id != *plan
        {
            plan_view.plan_id = plan.clone();
        }
        if saw_manual_plan && self.view.plan_proposed() && !self.view.tasks.is_empty() {
            let plan = self.view.plan.clone().unwrap_or_else(|| "plan".into());
            self.plan_view = Some(PlanView::new(plan));
        }
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
