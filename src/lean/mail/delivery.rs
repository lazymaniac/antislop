use super::*;
use crate::lean::{mail::recipient::Recipient, store::MailTarget};
use std::path::Path;

impl Core {
    pub(in crate::lean) fn answer_waiting(
        self: &Arc<Self>,
        conversation: &str,
        _plan_id: &str,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let drain = self.turn_lock(conversation, "__mail_drain");
        let Some(_drain) = lock_cancellable(&drain, cancel) else {
            return Ok(());
        };
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        for target in self.ready_mail(conversation)? {
            self.deliver_target(conversation, &project, target, cancel)?;
        }
        Ok(())
    }

    fn deliver_target(
        self: &Arc<Self>,
        conversation: &str,
        project: &Path,
        target: MailTarget,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let (task, recipient) = self.delivery_recipient(&target)?;
        for _ in 0..MAIL_TURNS {
            if cancel.load(Ordering::SeqCst)
                || self.pending_mail_empty(conversation, &target.role)?
            {
                break;
            }
            if !self.deliver_turns(
                conversation,
                project,
                &target,
                &recipient,
                task.as_ref(),
                cancel,
            )? {
                self.remember_failed_target(conversation, &target);
                break;
            }
        }
        Ok(())
    }

    fn remember_failed_target(&self, conversation: &str, target: &MailTarget) {
        self.mail_failures
            .lock()
            .expect("mail failures mutex")
            .insert(
                (conversation.to_owned(), target.role.clone()),
                target.newest,
            );
    }

    fn delivery_recipient(&self, target: &MailTarget) -> Result<(Option<Task>, Recipient)> {
        let task = self.mail_task(target.task_id.as_deref())?;
        let recipient = Recipient::new(&target.role, task.as_ref())?;
        Ok((task, recipient))
    }

    fn mail_task(&self, id: Option<&str>) -> Result<Option<Task>> {
        id.map(|id| self.store.task_by_id(id))
            .transpose()
            .map(Option::flatten)
    }

    fn pending_mail_empty(&self, conversation: &str, role: &str) -> Result<bool> {
        Ok(self.store.pending_mail(conversation, role)?.is_empty())
    }

    fn deliver_turns(
        self: &Arc<Self>,
        conversation: &str,
        project: &Path,
        target: &MailTarget,
        recipient: &Recipient,
        task: Option<&Task>,
        cancel: &AtomicBool,
    ) -> Result<bool> {
        for attempt in 0..DELIVERY_ATTEMPTS {
            if self.try_delivery(conversation, project, target, recipient, task, cancel)? {
                return Ok(true);
            }
            if cancel.load(Ordering::SeqCst)
                || self.pending_mail_empty(conversation, &target.role)?
            {
                return Ok(false);
            }
            if !self.retry_delivery(conversation, target, attempt, cancel)? {
                return Ok(false);
            }
        }
        Ok(false)
    }

    fn retry_delivery(
        self: &Arc<Self>,
        conversation: &str,
        target: &MailTarget,
        attempt: u32,
        cancel: &AtomicBool,
    ) -> Result<bool> {
        let error = self
            .store
            .last_turn(conversation, &target.role)?
            .and_then(|turn| turn.error)
            .unwrap_or_else(|| "recipient turn ended without acknowledgement".into());
        let retry = attempt + 1 < DELIVERY_ATTEMPTS && retryable(&error);
        self.store.append_event(conversation, None, target.plan_id.as_deref(),
            if retry { "mail_retry" } else { "mail_delivery_failed" },
            &json!({"recipient": target.role, "attempt": attempt + 1, "reason": error, "state": "queued"}))?;
        Ok(retry && retry_delay(attempt, cancel))
    }

    fn try_delivery(
        self: &Arc<Self>,
        conversation: &str,
        project: &Path,
        target: &MailTarget,
        recipient: &Recipient,
        task: Option<&Task>,
        cancel: &AtomicBool,
    ) -> Result<bool> {
        let setup = self.mail_setup(conversation, project, target, recipient, task);
        let direct = self
            .store
            .pending_mail(conversation, &target.role)?
            .iter()
            .any(|mail| mail.kind == "agent_work");
        let context = self.direct_mail_context(conversation, project, &target.role, direct)?;
        let answer = self.turn_inner(setup, cancel, |first| {
            if direct && !context.is_empty() { return context.clone(); }
            format!("Mailbox recipient: {}. This is a receipt-only turn; do not execute work, approve tasks or propose plans.\n{}",
                target.role, tasks::answer_prompt(&recipient.title, first))
        }, true)?;
        let Some(answer) = answer else {
            return Ok(false);
        };
        self.record_delivery(conversation, target, recipient, &answer)?;
        Ok(true)
    }

    fn record_delivery(
        self: &Arc<Self>,
        conversation: &str,
        target: &MailTarget,
        recipient: &Recipient,
        answer: &str,
    ) -> Result<()> {
        self.post(
            conversation,
            target.plan_id.as_deref(),
            &recipient.sender,
            answer,
        )?;
        self.mail_failures
            .lock()
            .expect("mail failures mutex")
            .remove(&(conversation.to_owned(), target.role.clone()));
        Ok(())
    }

    fn mail_setup<'a>(
        &self,
        conversation: &'a str,
        project: &Path,
        target: &MailTarget,
        recipient: &Recipient,
        task: Option<&Task>,
    ) -> TurnSetup<'a> {
        TurnSetup {
            session_policy: task.filter(|_| task_role(&target.role).is_some()).map_or(
                SessionPolicy::Role,
                |task| SessionPolicy::TaskRound {
                    task: task.id.clone(),
                    round: task.round,
                },
            ),
            conversation,
            plan_id: target.plan_id.clone(),
            role: target.role.clone(),
            profile: self
                .config_snapshot()
                .resolve(recipient.kind, recipient.role_kind),
            cwd: project.to_path_buf(),
            add_dirs: vec![],
            access: agent::Access::ReadOnly,
            upto_message: 0,
        }
    }

    fn direct_mail_context(
        &self,
        conversation: &str,
        project: &Path,
        role: &str,
        direct: bool,
    ) -> Result<String> {
        if !direct || !matches!(role, AUDITOR | PLAN_VERIFIER) {
            return Ok(String::new());
        }
        if role == PLAN_VERIFIER {
            return self.plan_verifier_context(conversation, project);
        }
        self.auditor_context(conversation, project)
    }

    fn plan_verifier_context(&self, conversation: &str, project: &Path) -> Result<String> {
        let plan = self.store.latest_plan(conversation)?;
        let specs = plan
            .iter()
            .map(|task| serde_json::from_value(task.spec.clone()))
            .collect::<std::result::Result<Vec<TaskSpec>, _>>()?;
        Ok(tasks::plan_review_prompt(
            &project.to_string_lossy(),
            "Review the current plan again, following the user's direct instructions.",
            &specs,
        ))
    }

    fn auditor_context(&self, conversation: &str, project: &Path) -> Result<String> {
        let plan = self.store.latest_plan(conversation)?;
        let results = plan
            .iter()
            .map(|task| {
                format!(
                    "{} · {}\n{}",
                    task.spec["id"],
                    task.status,
                    task.report.as_deref().unwrap_or("(no report)")
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        Ok(tasks::audit_prompt(
            &project.to_string_lossy(),
            "Audit the current result again, following the user's direct instructions.",
            &results,
        ))
    }
}
