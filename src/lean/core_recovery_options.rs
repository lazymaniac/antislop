use super::*;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct RecoveryState {
    pub input_error: Option<String>,
}

impl Core {
    pub(super) fn recovery_states(&self, tasks: &[Task]) -> HashMap<String, RecoveryState> {
        tasks
            .iter()
            .map(|task| {
                (
                    task.id.clone(),
                    RecoveryState {
                        input_error: self
                            .store
                            .assert_task_inputs(task)
                            .err()
                            .map(|error| error.to_string()),
                    },
                )
            })
            .collect()
    }
}

pub(super) fn recovery_options(
    task: &Task,
    plan: &[(TaskSpec, String)],
    state: &RecoveryState,
) -> Vec<(&'static str, Result<()>)> {
    if let Some(error) = &state.input_error {
        return [
            "add_task",
            "split_task",
            "clarify_task",
            "retry_with_guidance",
            "extend_rounds",
        ]
        .into_iter()
        .map(|action| (action, Err(Error::Invalid(error.clone()))))
        .collect();
    }
    let parsed = serde_json::from_value::<TaskSpec>(task.spec.clone());
    let spec = match parsed {
        Ok(spec) => spec,
        Err(error) => return vec![("recovery", Err(Error::Invalid(error.to_string())))],
    };
    let structural = || {
        require(
            task.status == status::NEEDS_USER,
            "requires needs_user status",
        )?;
        let block = task
            .report
            .as_deref()
            .and_then(blocker::parse)
            .ok_or_else(|| Error::Invalid("requires a structured blocker".into()))?;
        require(
            !block.for_user(),
            "only the user can lift a permission blocker",
        )
    };
    vec![
        (
            "add_task",
            structural().and_then(|()| blocker::check_addition(&spec, plan)),
        ),
        (
            "split_task",
            structural().and_then(|()| {
                require(
                    !spec.writes,
                    "splitting a writer with partial changes is left to the user",
                )?;
                crate::lean::split::check_parent(&spec, plan)
            }),
        ),
        (
            "clarify_task",
            structural()
                .and_then(|()| require(!spec.writes, "only a read-only task can be clarified")),
        ),
        ("retry_with_guidance", Ok(())),
        ("extend_rounds", Ok(())),
    ]
}

pub(super) fn options_text(options: &[(&str, Result<()>)]) -> String {
    options
        .iter()
        .map(|(action, result)| match result {
            Ok(()) => format!(
                "\n  {action}: available (proposal and live state are validated before execution)"
            ),
            Err(error) => format!("\n  {action}: unavailable — {error}"),
        })
        .collect()
}
