use super::*;

impl Core {
    /// Labels are local to a plan; an explicit UUID can name an earlier task
    /// in this conversation, but can never cross the conversation boundary.
    pub(in crate::lean) fn mail_address_task(
        &self,
        conversation: &str,
        plan: &[Task],
        address: &str,
    ) -> Result<Option<(Task, String)>> {
        let (id, prefix) = parse_mail_address(address);
        let task = plan
            .iter()
            .find(|task| task.spec["id"] == id || task.id == id)
            .cloned()
            .or(self.store.task_by_id(id)?);
        let Some(task) = task else {
            return Ok(None);
        };
        if self.store.plan_conversation(&task.plan_id)? != conversation {
            return Ok(None);
        }
        let role = format!("{prefix}:{}", task.id);
        Ok(Some((task, role)))
    }

    pub(in crate::lean::mail) fn ready_mail(
        &self,
        conversation: &str,
    ) -> Result<Vec<store::MailTarget>> {
        let mut targets = self.store.pending_mail_targets(conversation)?;
        let failures = self.mail_failures.lock().expect("mail failures mutex");
        targets.retain(|target| self.mail_target_ready(conversation, target, &failures));
        Ok(targets)
    }

    fn mail_target_ready(
        &self,
        conversation: &str,
        target: &store::MailTarget,
        failures: &HashMap<(String, String), u64>,
    ) -> bool {
        self.mail_target_supported(target)
            && !self.direct_task_mail(conversation, target)
            && !self.direct_orchestrator_mail(conversation, target)
            && failures
                .get(&(conversation.to_owned(), target.role.clone()))
                .is_none_or(|failed| target.newest > *failed)
    }

    fn mail_target_supported(&self, target: &store::MailTarget) -> bool {
        if let Some(id) = &target.task_id {
            return self
                .store
                .task_by_id(id)
                .ok()
                .flatten()
                .is_some_and(|task| status::finished(&task.status));
        }
        matches!(
            target.role.as_str(),
            ORCHESTRATOR | COMMENT_ORCHESTRATOR | PLAN_VERIFIER | AUDITOR
        )
    }

    fn direct_task_mail(&self, conversation: &str, target: &store::MailTarget) -> bool {
        target.task_id.is_some()
            && self
                .store
                .pending_mail(conversation, &target.role)
                .is_ok_and(|mail| mail.iter().any(|mail| mail.kind == "agent_work"))
    }

    fn direct_orchestrator_mail(&self, conversation: &str, target: &store::MailTarget) -> bool {
        target.role == ORCHESTRATOR
            && self
                .store
                .pending_mail(conversation, &target.role)
                .is_ok_and(|mail| {
                    mail.iter()
                        .all(|mail| matches!(mail.kind.as_str(), "agent_work" | "copy"))
                })
    }
}
