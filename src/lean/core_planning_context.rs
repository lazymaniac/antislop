//! The requirements ledger as planning context, shared by every agent that writes a plan.
use super::*;

pub(in crate::lean) fn append_ledger(context: Option<&str>, prompt: &mut String) {
    let Some(context) = context else { return };
    prompt.push_str(context);
    prompt.push_str(LEDGER_INSTRUCTIONS);
}

pub(in crate::lean) fn planning_ledger_context(
    core: &Core,
    conversation: &str,
) -> Result<Option<String>> {
    load_planning_context(core.store.planning_run(conversation)?, |run| {
        core.ledger_context(conversation, &run.id)
    })
}

fn load_planning_context<T>(
    run: Option<T>,
    render: impl FnOnce(T) -> Result<String>,
) -> Result<Option<String>> {
    run.map(render).transpose()
}

const LEDGER_INSTRUCTIONS: &str = "\nThe runtime binds this plan to the active requirements run. In each task's `covers`, name the acceptance criterion ids it delivers; a supporting task that delivers no criterion names the requirement id it serves instead. Every task covers something. Cover every mandatory criterion across the tasks; supporting tasks need not cover every criterion themselves. Requirements extraction and corrections are performed by the runtime requirements analyst, including after requirements review rejection; do not extract or edit requirements yourself. Resolve task planning findings in the plan. Do not ask the user to copy run or option identifiers.\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_context_storage_error_is_not_replaced_with_empty_text() {
        let run = Some(());
        let result =
            load_planning_context(run, |_| Err(Error::Invalid("ledger unavailable".into())));

        assert!(matches!(
            result,
            Err(Error::Invalid(message)) if message == "ledger unavailable"
        ));
    }
}
