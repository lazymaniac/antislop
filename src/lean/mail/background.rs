use super::*;

impl Core {
    pub(in crate::lean) fn answer_in_background(
        self: &Arc<Self>,
        conversation: &str,
        _plan_id: &str,
    ) {
        let mut jobs = self.mail_jobs.lock().expect("mail jobs mutex");
        if jobs.contains_key(conversation)
            || self
                .ready_mail(conversation)
                .is_ok_and(|mail| mail.is_empty())
        {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        jobs.insert(conversation.to_owned(), cancel.clone());
        drop(jobs);
        let core = Arc::clone(self);
        let conversation = conversation.to_owned();
        std::thread::spawn(move || {
            core.drain_mail_batches(&conversation, &cancel);
        });
    }

    fn drain_mail_batches(self: &Arc<Self>, conversation: &str, cancel: &AtomicBool) {
        for batch in 1..=MAIL_BATCH_LIMIT {
            let before = self.latest_mail(conversation);
            if let Err(error) = self.answer_waiting(conversation, "", cancel) {
                self.notice(
                    conversation,
                    &format!("mail delivery failed; messages remain queued: {error}"),
                );
            }
            let mut jobs = self.mail_jobs.lock().expect("mail jobs mutex");
            let newer = self.has_newer_mail(conversation, before);
            if newer && batch < MAIL_BATCH_LIMIT && !cancel.load(Ordering::SeqCst) {
                drop(jobs);
                continue;
            }
            let external =
                newer && batch == MAIL_BATCH_LIMIT && self.defer_agent_mail(conversation, before);
            if newer && batch == MAIL_BATCH_LIMIT {
                self.notice(conversation, "mail response limit reached; remaining agent replies stay queued until a new message or restart");
            }
            jobs.remove(conversation);
            drop(jobs);
            if external && !cancel.load(Ordering::SeqCst) {
                self.answer_in_background(conversation, "");
            }
            return;
        }
    }

    fn latest_mail(&self, conversation: &str) -> u64 {
        self.ready_mail(conversation)
            .ok()
            .and_then(|targets| targets.iter().map(|target| target.newest).max())
            .unwrap_or(0)
    }

    fn has_newer_mail(&self, conversation: &str, after: u64) -> bool {
        self.ready_mail(conversation)
            .ok()
            .is_some_and(|targets| targets.iter().any(|target| target.newest > after))
    }

    fn defer_agent_mail(&self, conversation: &str, after: u64) -> bool {
        let Ok(targets) = self.ready_mail(conversation) else {
            return false;
        };
        let mut external_arrival = false;
        let mut failures = self.mail_failures.lock().expect("mail failures mutex");
        for target in targets {
            let external = self
                .store
                .pending_mail(conversation, &target.role)
                .ok()
                .is_some_and(|mail| {
                    mail.iter().any(|mail| {
                        mail.id > after && matches!(mail.sender.as_str(), "user" | RUNTIME)
                    })
                });
            if external {
                external_arrival = true;
            } else {
                failures.insert((conversation.to_owned(), target.role), target.newest);
            }
        }
        external_arrival
    }
}
