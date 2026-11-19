//! Deterministic task protocol for CLI fixtures. Production planning and review stay enabled.
#[allow(dead_code)]
pub fn install_task_protocol(path: &std::path::Path) {
    let source = std::fs::read_to_string(path).unwrap();
    if source.contains("# TASK_FIXTURE_PROTOCOL") || !source.contains("prompt = sys.argv[-1]") {
        return;
    }
    let hook = include_str!("task_cli.py");
    let source = source.replacen(
        "prompt = sys.argv[-1]",
        &format!("prompt = sys.argv[-1]\n{hook}"),
        1,
    );
    std::fs::write(path, source).unwrap();
}

/// A conversation whose interview is already settled: the next message is the
/// request, and the planner drafts the plan for it. The interview itself is
/// covered in lean_workflow.
#[allow(dead_code)]
pub fn fixture_conversation(
    core: &antislop::lean::Core,
    project: &std::path::Path,
) -> antislop::Result<antislop::lean::store::Conversation> {
    fixture_conversation_with_requirements(core, project, "Fixture requirements.")
}

/// Create a settled conversation whose confirmed scope matches the next
/// request sent by a test.
#[allow(dead_code)]
pub fn fixture_conversation_with_requirements(
    core: &antislop::lean::Core,
    project: &std::path::Path,
    requirements: &str,
) -> antislop::Result<antislop::lean::store::Conversation> {
    let project = project.canonicalize()?;
    let is_git_root = std::process::Command::new("git")
        .args([
            "-C",
            project.to_str().unwrap_or_default(),
            "rev-parse",
            "--show-toplevel",
        ])
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).trim() == project.to_string_lossy()
                && std::process::Command::new("git")
                    .args([
                        "-C",
                        project.to_str().unwrap_or_default(),
                        "show-ref",
                        "--verify",
                        "--quiet",
                        "refs/heads/main",
                    ])
                    .status()
                    .is_ok_and(|status| status.success())
        });
    let conversation = if is_git_root {
        core.start(&project)?
    } else {
        core.store.create_conversation(&project.to_string_lossy())?
    };
    core.store
        .interview_summary(&conversation.id, requirements, 0)?;
    confirm_scope(core, &conversation.id)?;
    Ok(conversation)
}

/// Locate the linked worktree for a conversation's durable integration branch.
#[allow(dead_code)]
pub fn session_worktree(
    core: &antislop::lean::Core,
    project: &std::path::Path,
    conversation: &str,
) -> std::path::PathBuf {
    let branch = session_branch(core, conversation);
    session_worktree_if_present(project, &branch)
        .unwrap_or_else(|| panic!("no linked worktree for session branch {branch}"))
}

#[allow(dead_code)]
pub fn session_branch(core: &antislop::lean::Core, conversation: &str) -> String {
    core.store
        .events_since(conversation, 0, 100_000)
        .unwrap()
        .into_iter()
        .rev()
        .find(|event| event.kind == "session_started")
        .and_then(|event| event.payload["branch"].as_str().map(str::to_owned))
        .expect("session_started event includes the session branch")
}

#[allow(dead_code)]
pub fn session_worktree_if_present(
    project: &std::path::Path,
    branch: &str,
) -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("git")
        .args([
            "-C",
            project.to_str().unwrap(),
            "worktree",
            "list",
            "--porcelain",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let mut path = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(value) = line.strip_prefix("worktree ") {
            path = Some(std::path::PathBuf::from(value));
        } else if line == format!("branch refs/heads/{branch}") {
            return Some(path.expect("worktree record has a path"));
        }
    }
    None
}

/// Confirm the current requirements summary through the same structured
/// answer recorded by the user interface.
#[allow(dead_code)]
pub fn confirm_scope(core: &antislop::lean::Core, conversation: &str) -> antislop::Result<()> {
    confirm_scope_store(&core.store, conversation)
}

#[allow(dead_code)]
pub fn confirm_scope_store(
    store: &antislop::lean::store::Store,
    conversation: &str,
) -> antislop::Result<()> {
    store.finish_interview(conversation)?;
    let event = store
        .events_since(conversation, 0, 10_000)?
        .into_iter()
        .rev()
        .find(|event| event.kind == "scope_confirmation_requested")
        .ok_or_else(|| antislop::Error::Invalid("scope confirmation was not requested".into()))?;
    let question: antislop::lean::interview::Question =
        serde_json::from_value(event.payload["question"].clone())?;
    store.add_message(
        conversation,
        "user",
        &format!("Interview answer ({})\nAnswer: Confirm scope", question.id),
    )?;
    if !store.finish_interview(conversation)? {
        return Err(antislop::Error::Invalid(
            "confirmed scope did not enter planning".into(),
        ));
    }
    Ok(())
}

/// Keep file-writing fixtures from executing their writer branch during
/// the newly inserted read-only options turns. Dedicated options integration
/// tests supply their own comparison and reviewer oracle instead.
#[allow(dead_code)]
pub fn install_task_options_protocol(path: &std::path::Path) {
    let source = std::fs::read_to_string(path).unwrap();
    let hook = r#"
if _lr.search(r'^Task: Options for ', prompt, _lr.M):
    if 'You are a worker' in prompt:
        _fixture_emit('Variants: direct fixture change; unnecessary abstraction. Criteria: satisfy the fixture contract with minimal changes. Selected: direct fixture change. Implement only the requested file change.')
    if 'You review' in prompt:
        ids = _lr.findall(r'^- ([A-Za-z0-9_.-]+): ', prompt, _lr.M)
        _fixture_emit(_fixture_block('verdict', {'approved': True, 'findings': [], 'coverage': {i: {'status': 'ok', 'note': 'fixture comparison checked'} for i in ids}}))
"#;
    assert!(source.contains("json.dumps = _fixture_dumps"));
    std::fs::write(
        path,
        source.replace(
            "json.dumps = _fixture_dumps",
            &format!("json.dumps = _fixture_dumps\n{hook}"),
        ),
    )
    .unwrap();
}
