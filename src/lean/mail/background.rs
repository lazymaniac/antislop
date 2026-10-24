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
            let mut batches = 0;
            loop {
                batches += 1;
                let before = core
                    .ready_mail(&conversation)
                    .ok()
                    .and_then(|targets| targets.iter().map(|target| target.newest).max())
                    .unwrap_or(0);
                if let Err(error) = core.answer_waiting(&conversation, "", &cancel) {
                    core.notice(
                        &conversation,
                        &format!("mail delivery failed; messages remain queued: {error}"),
                    );
                }
                // A sender persists its message before taking this same lock.
                // Either the drain sees it here, or that sender starts a new job.
                let mut jobs = core.mail_jobs.lock().expect("mail jobs mutex");
                let newer = core
                    .ready_mail(&conversation)
                    .ok()
                    .is_some_and(|targets| targets.iter().any(|target| target.newest > before));
                if newer && batches < MAIL_BATCH_LIMIT && !cancel.load(Ordering::SeqCst) {
                    drop(jobs);
                    continue;
                }
                let mut external_arrival = false;
                if newer && batches >= MAIL_BATCH_LIMIT {
                    if let Ok(targets) = core.ready_mail(&conversation) {
                        let mut failures = core.mail_failures.lock().expect("mail failures mutex");
                        for target in targets {
                            let external = core
                                .store
                                .pending_mail(&conversation, &target.role)
                                .ok()
                                .is_some_and(|mail| {
                                    mail.iter().any(|mail| {
                                        mail.id > before
                                            && matches!(mail.sender.as_str(), "user" | RUNTIME)
                                    })
                                });
                            if external {
                                external_arrival = true;
                            } else {
                                failures.insert((conversation.clone(), target.role), target.newest);
                            }
                        }
                    }
                    core.notice(&conversation, "mail response limit reached; remaining agent replies stay queued until a new message or restart");
                }
                jobs.remove(&conversation);
                drop(jobs);
                if external_arrival && !cancel.load(Ordering::SeqCst) {
                    core.answer_in_background(&conversation, "");
                }
                break;
            }
        });
    }
}
