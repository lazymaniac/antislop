use super::*;

pub(in crate::lean::ledger::runtime) struct RequirementsTurnScope<'a> {
    pub conversation: &'a str,
    pub run_id: &'a str,
    pub phase: &'a str,
    pub plan_id: Option<&'a str>,
}

impl Core {
    pub(in crate::lean::ledger::runtime) fn requirements_turn(
        self: &Arc<Self>,
        scope: RequirementsTurnScope<'_>,
        reviewer: bool,
        prompt: String,
        cancel: &AtomicBool,
    ) -> Result<(String, String)> {
        let RequirementsTurnScope {
            conversation,
            run_id,
            phase,
            plan_id,
        } = scope;
        if let Some(plan) = plan_id {
            require(
                self.store.plan_conversation(plan)? == conversation
                    && self.store.plan_run(plan)?.as_deref() == Some(run_id),
                "audit plan does not belong to this conversation and run",
            )?;
        }
        self.publish_requirements_progress(conversation, phase, reviewer)?;
        let role = requirements_role(phase, reviewer);
        let mut setup = self.requirements_turn_setup(conversation, &role, phase, reviewer)?;
        setup.plan_id = plan_id.map(str::to_owned);
        let (source_ids, revision) = self.requirements_turn_metadata(conversation, run_id)?;
        let context = self.ledger_context(conversation, run_id)?;
        let answer =
            self.run_requirements_agent(setup, cancel, &prompt, &context, conversation, &role)?;
        let turn = self
            .store
            .last_turn(conversation, &role)?
            .ok_or_else(|| Error::Invalid("requirements turn missing".into()))?;
        self.bind_requirements_turn(conversation, run_id, phase, revision, &source_ids, &turn.id)?;
        Ok((answer, turn.id))
    }

    fn bind_requirements_turn(
        &self,
        conversation: &str,
        run_id: &str,
        phase: &str,
        revision: i64,
        source_ids: &[String],
        turn_id: &str,
    ) -> Result<()> {
        self.store
            .bind_ledger_turn(conversation, run_id, turn_id, revision, phase, source_ids)
    }

    fn publish_requirements_progress(
        &self,
        conversation: &str,
        phase: &str,
        reviewer: bool,
    ) -> Result<()> {
        let text = match (phase, reviewer) {
            ("extraction", false) => {
                "Preparing requirements from the interview and source documents."
            }
            ("extraction", true) => "Checking requirements independently before planning.",
            ("options", false) => "Comparing implementation alternatives.",
            ("options", true) => "Checking the solution comparison independently.",
            (_, false) => "Auditing the integrated implementation against requirements.",
            (_, true) => "Reviewing the implementation audit independently.",
        };
        self.store.append_event(
            conversation,
            None,
            None,
            "planning_progress",
            &json!({"text": text}),
        )?;
        Ok(())
    }

    fn requirements_turn_setup<'a>(
        &self,
        conversation: &'a str,
        role: &str,
        phase: &str,
        reviewer: bool,
    ) -> Result<TurnSetup<'a>> {
        let role_kind = if reviewer {
            RoleKind::Reviewer
        } else {
            RoleKind::Executor
        };
        let work_kind = if phase == "options" {
            WorkKind::Options
        } else {
            WorkKind::Requirements
        };
        Ok(TurnSetup {
            session_policy: crate::lean::SessionPolicy::Independent,
            conversation,
            plan_id: None,
            role: role.into(),
            profile: self.config_snapshot().resolve(work_kind, role_kind),
            cwd: PathBuf::from(self.store.conversation(conversation)?.project),
            add_dirs: vec![],

            upto_message: 0,
        })
    }

    fn requirements_turn_metadata(
        &self,
        conversation: &str,
        run_id: &str,
    ) -> Result<(Vec<String>, i64)> {
        let sources = self.store.ledger_sources(conversation, run_id)?;
        let ids = sources.into_iter().map(|source| source.id).collect();
        let revision = self.store.run(conversation, run_id)?.spec_revision;
        Ok((ids, revision))
    }

    fn run_requirements_agent(
        &self,
        setup: TurnSetup<'_>,
        cancel: &AtomicBool,
        prompt: &str,
        context: &str,
        conversation: &str,
        role: &str,
    ) -> Result<String> {
        self.turn(setup, cancel, |_, _| format!("{prompt}\n{context}"))?
            .ok_or_else(|| Error::Invalid(self.requirements_failure(conversation, role)))
    }

    fn requirements_failure(&self, conversation: &str, role: &str) -> String {
        let error = self
            .store
            .last_turn(conversation, role)
            .ok()
            .flatten()
            .and_then(|turn| turn.error)
            .unwrap_or_else(|| "no response".into());
        format!("{role} failed: {error}")
    }
}

fn requirements_role(phase: &str, reviewer: bool) -> String {
    format!(
        "requirements-{phase}-{}",
        if reviewer { "reviewer" } else { "analyst" }
    )
}
