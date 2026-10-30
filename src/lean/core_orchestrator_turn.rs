use super::*;
mod context;
mod prompt;

impl Core {
    pub(in crate::lean) fn orchestrator_turn(
        self: &Arc<Self>,
        conversation: &str,
        upto: u64,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let context = context::prepare(self, conversation, upto)?;
        let Some(answer) = context::ask(self, &context, cancel)? else {
            return Ok(());
        };
        context::finish(self, &context, &answer, cancel)
    }
}
