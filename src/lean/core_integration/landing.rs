use super::super::core_feedback_helpers::land;
use super::super::core_path_helpers::worktree_branch;
use super::super::*;

pub(super) struct Landing {
    pub(super) status: &'static str,
    pub(super) text: String,
}

pub(super) fn record(
    core: &Core,
    conversation: &str,
    task: &mut Task,
    spec: &TaskContract,
    outcome: &git::Integration,
    files: &[String],
) -> Result<Landing> {
    let landing = record_outcome(core, task, outcome, files)?;
    append_event(core, conversation, task, spec, &landing);
    Ok(landing)
}

fn record_outcome(
    core: &Core,
    task: &mut Task,
    outcome: &git::Integration,
    files: &[String],
) -> Result<Landing> {
    Ok(match outcome {
        git::Integration::Committed(commit) => approved(
            task,
            Outcome::Committed,
            format!("committed as {commit}"),
            Some(commit),
            files,
        ),
        git::Integration::Applied => approved(
            task,
            Outcome::Applied,
            "applied to the working tree, not committed".to_owned(),
            None,
            files,
        ),
        git::Integration::Conflict(files) | git::Integration::UserEdits(files) => {
            unresolved(core, task, outcome, files)?
        }
    })
}

fn approved(
    task: &mut Task,
    outcome: Outcome,
    text: String,
    commit: Option<&str>,
    files: &[String],
) -> Landing {
    land(task, outcome, "run", &text, commit, None, files);
    Landing {
        status: status::APPROVED,
        text,
    }
}

fn append_event(
    core: &Core,
    conversation: &str,
    task: &Task,
    spec: &TaskContract,
    landing: &Landing,
) {
    let _ = core.store.append_event(
        conversation,
        None,
        Some(&task.plan_id),
        "integration",
        &json!({
            "id": spec.id,
            "title": spec.title,
            "status": landing.status,
            "detail": tasks::bound(&landing.text, tasks::DETAIL_LIMIT)
        }),
    );
}

fn unresolved(
    core: &Core,
    task: &mut Task,
    outcome: &git::Integration,
    files: &[String],
) -> Result<Landing> {
    let (kind, reason) = match outcome {
        git::Integration::Conflict(_) => (Outcome::Conflict, "conflicts with the current branch"),
        git::Integration::UserEdits(_) => (Outcome::UserEdits, "you have uncommitted edits"),
        _ => unreachable!("only unresolved integration outcomes reach this helper"),
    };
    let branch = worktree_branch(&core.state, task)?;
    let text = format!(
        "{reason} in {}; the change is on branch {branch}",
        files.join(", ")
    );
    land(task, kind, "run", &text, None, Some(&branch), files);
    Ok(Landing {
        status: status::NEEDS_USER,
        text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core() -> (tempfile::TempDir, Arc<Core>) {
        std::fs::create_dir_all(".antislop/tmp").unwrap();
        let project = tempfile::tempdir_in(".antislop/tmp").unwrap();
        let state = project.path().join(".antislop");
        let config = Config::from_toml(include_str!("../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&state, config).unwrap();
        (project, core)
    }

    fn task() -> Task {
        serde_json::from_value(json!({
            "id": "writer",
            "plan_id": "plan",
            "spec": {},
            "status": "running",
            "round": 2,
            "report": null,
            "verdict": null,
            "base": null,
            "result": null
        }))
        .unwrap()
    }

    #[test]
    fn committed_landing_records_commit_and_approves_task() {
        let (_project, core) = core();
        let mut task = task();
        let landing = record_outcome(
            &core,
            &mut task,
            &git::Integration::Committed("abc123".to_owned()),
            &["src/lib.rs".to_owned()],
        )
        .unwrap();

        assert_eq!(landing.status, status::APPROVED);
        assert_eq!(landing.text, "committed as abc123");
        assert_eq!(
            task.feedback.as_ref().unwrap()["integration"]["outcome"],
            "committed"
        );
        assert_eq!(
            task.feedback.as_ref().unwrap()["integration"]["commit"],
            "abc123"
        );
    }

    #[test]
    fn applied_landing_records_uncommitted_result_and_approves_task() {
        let (_project, core) = core();
        let mut task = task();
        let landing = record_outcome(&core, &mut task, &git::Integration::Applied, &[]).unwrap();

        assert_eq!(landing.status, status::APPROVED);
        assert_eq!(landing.text, "applied to the working tree, not committed");
        assert_eq!(
            task.feedback.as_ref().unwrap()["integration"]["outcome"],
            "applied"
        );
        assert!(task.feedback.as_ref().unwrap()["integration"]["commit"].is_null());
    }
}
