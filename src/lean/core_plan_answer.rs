use super::*;

/// What an agent that writes plans answered: its words and the plan it
/// submitted, if any.
pub(in crate::lean) struct PlanAnswer<'a> {
    pub(in crate::lean) conversation: &'a str,
    pub(in crate::lean) project: &'a Path,
    pub(in crate::lean) config: &'a Config,
    pub(in crate::lean) text: &'a str,
    pub(in crate::lean) plan: Option<Result<Vec<TaskContract>>>,
    pub(in crate::lean) addressed: Option<&'a [u64]>,
    pub(in crate::lean) is_comment: bool,
    pub(in crate::lean) answered: u64,
    pub(in crate::lean) upto: u64,
}

impl Core {
    /// A submitted plan is proposed; words alone only mark the comments they
    /// answered.
    pub(in crate::lean) fn handle_plan_answer(
        self: &Arc<Self>,
        answer: PlanAnswer<'_>,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let PlanAnswer {
            conversation,
            project,
            config,
            text,
            plan,
            addressed,
            is_comment,
            answered,
            upto,
        } = answer;
        match plan {
            Some(Ok(specs)) => self.propose(
                conversation,
                project,
                text,
                PlanProposal {
                    config,
                    specs: &specs,
                },
                addressed,
                cancel,
            ),
            Some(Err(error)) => self.record_invalid_plan(conversation, &error),
            None => {
                // Answered in words only: the comments stay open for the user.
                if is_comment {
                    self.record_comment_reply(conversation, answered, upto)?;
                }
                Ok(())
            }
        }
    }

    /// `submit_plan` refuses a malformed plan within the turn, so this is only
    /// reached when no plan was submitted or a stored one no longer passes.
    fn record_invalid_plan(&self, conversation: &str, error: &Error) -> Result<()> {
        self.store.append_event(
            conversation,
            None,
            None,
            "plan_invalid",
            &json!({"error":error.to_string(),"retrying":false,"retry":"Send a message to retry; your requirements are preserved."}),
        )?;
        Ok(())
    }

    fn record_comment_reply(&self, conversation: &str, answered: u64, upto: u64) -> Result<()> {
        let replied: Vec<u64> = self
            .store
            .comments(conversation, "open")?
            .into_iter()
            .filter(|c| c.message > answered && c.message <= upto)
            .map(|c| c.id)
            .collect();
        self.store.append_event(
            conversation,
            None,
            None,
            "comment_replied",
            &json!({"comments": replied}),
        )?;
        Ok(())
    }
}
