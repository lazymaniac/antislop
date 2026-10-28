use super::*;

impl Core {
    /// An independent reviewer's verdict (a verifier profile, its own
    /// session per role), or why there is none. `prompt` gets whether this is
    /// the session's first turn and the user's messages it has not seen.
    pub(in crate::lean) fn review(
        self: &Arc<Self>,
        conversation: &str,
        role: &str,
        plan_id: Option<&str>,
        project: &Path,
        cancel: &AtomicBool,
        prompt: impl FnOnce(bool, &str) -> String,
    ) -> Result<std::result::Result<Verdict, String>> {
        let config = self.config_snapshot();
        let answered = self.store.last_answered(conversation, role)?;
        let all = self.store.messages_by(conversation, "user", 0)?;
        let upto = all.last().map_or(0, |message| message.seq);
        let setup = review_setup(conversation, role, plan_id, project, &config, upto);
        let mut answer = self.turn(setup, cancel, |first| {
            let initial = first || answered == 0;
            prompt(
                initial,
                &review_request(&all, if initial { 0 } else { answered }),
            )
        })?;
        // A review that states its decision in prose only is not rejected for
        // it: the reviewer is asked once to put the same decision in the block.
        if answer
            .as_deref()
            .is_some_and(|text| tasks::structured_verdict(text).is_none())
            && !cancel.load(Ordering::SeqCst)
        {
            let setup = review_setup(conversation, role, plan_id, project, &config, upto);
            if let Some(repaired) = self.turn(setup, cancel, |_| VERDICT_REPAIR.to_owned())? {
                answer = Some(repaired);
            }
        }
        self.review_outcome(conversation, role, answer)
    }

    /// Runs one agent turn and records it. `Ok(None)` means the turn failed
    /// or was cancelled; that is recorded on the turn and blocks nothing.
    pub(in crate::lean) fn turn(
        &self,
        setup: TurnSetup<'_>,
        cancel: &AtomicBool,
        prompt: impl FnOnce(bool) -> String,
    ) -> Result<Option<String>> {
        self.turn_inner(setup, cancel, prompt)
    }

    fn review_outcome(
        self: &Arc<Self>,
        conversation: &str,
        role: &str,
        answer: Option<String>,
    ) -> Result<std::result::Result<Verdict, String>> {
        Ok(match answer {
            Some(answer) => Ok(tasks::parse_verdict(&answer)),
            None => Err(self
                .store
                .last_turn(conversation, role)?
                .and_then(|turn| turn.error)
                .unwrap_or_else(|| "no reason recorded".into())),
        })
    }
}

const VERDICT_REPAIR: &str = "Your last answer did not end with the required verdict block, so it cannot be recorded. Reply with your same decision and findings, ending with ```verdict\n{\"approved\":true|false,\"findings\":[{\"id\":\"stable-id\",\"severity\":\"high|medium|low\",\"issue\":\"...\",\"where\":\"...\"}]}\n```. Do not repeat your analysis.";

fn review_request(messages: &[Message], after: u64) -> String {
    messages
        .iter()
        .filter(|message| message.seq > after)
        .map(|message| message.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn review_setup<'a>(
    conversation: &'a str,
    role: &str,
    plan_id: Option<&str>,
    project: &Path,
    config: &Config,
    upto: u64,
) -> TurnSetup<'a> {
    TurnSetup {
        session_policy: SessionPolicy::Role,
        conversation,
        plan_id: plan_id.map(str::to_string),
        role: role.into(),
        profile: config.resolve(WorkKind::Orchestration, RoleKind::Verifier),
        cwd: project.to_path_buf(),
        add_dirs: vec![],
        access: agent::Access::ReadOnly,
        upto_message: upto,
    }
}
