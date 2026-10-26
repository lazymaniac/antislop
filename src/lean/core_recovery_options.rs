use super::*;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct RecoveryLimits {
    pub retries: u32,
    pub clarifications: usize,
    pub input_error: Option<String>,
}

impl Core {
    pub(super) fn recovery_limits(
        &self,
        conversation: &str,
        tasks: &[Task],
    ) -> HashMap<String, RecoveryLimits> {
        let unblocked = self.unblocked.lock().expect("unblocked mutex");
        tasks
            .iter()
            .filter(|task| {
                matches!(
                    task.status.as_str(),
                    status::NEEDS_USER | status::WAITING | status::FAILED
                )
            })
            .map(|task| {
                let retries = unblocked
                    .get(&(conversation.to_owned(), task.id.clone()))
                    .copied()
                    .unwrap_or(0);
                let clarifications =
                    self.store
                        .clarification_count(&task.id)
                        .unwrap_or_else(|error| {
                            self.notice(
                                conversation,
                                &format!(
                                    "cannot read clarification budget for {}: {error}",
                                    task.id
                                ),
                            );
                            blocker::CLARIFICATIONS
                        });
                (
                    task.id.clone(),
                    RecoveryLimits {
                        retries,
                        clarifications,
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
    autonomous: bool,
    limits: &RecoveryLimits,
) -> Vec<(&'static str, Result<()>)> {
    if let Some(error) = &limits.input_error {
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
    let retry = || {
        require(
            !autonomous || limits.retries < 3,
            "automatic retry budget exhausted",
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
            structural().and_then(|()| {
                require(!spec.writes, "only a read-only task can be clarified")?;
                require(
                    limits.clarifications < blocker::CLARIFICATIONS,
                    "task was already clarified twice; the user decides",
                )
            }),
        ),
        ("retry_with_guidance", retry()),
        ("extend_rounds", retry()),
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
