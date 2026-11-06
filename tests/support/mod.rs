//! Extend the existing deterministic CLI fixtures with the ledger protocol.
//! Production behavior remains fully enabled; worker communication fixtures still
//! control their own task answers, while extraction gets a separate session.
#[allow(dead_code)]
pub fn install_ledger_protocol(path: &std::path::Path) {
    let source = std::fs::read_to_string(path).unwrap();
    if source.contains("# LEDGER_FIXTURE_PROTOCOL") || !source.contains("prompt = sys.argv[-1]") {
        return;
    }
    let hook = include_str!("ledger_cli.py");
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
    let project = project.canonicalize()?;
    let conversation = core.store.create_conversation(&project.to_string_lossy())?;
    core.store
        .interview_summary(&conversation.id, "Fixture requirements.", 0)?;
    confirm_scope(core, &conversation.id)?;
    Ok(conversation)
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
        _ledger_emit('Variants: direct fixture change; unnecessary abstraction. Criteria: satisfy the fixture contract with minimal changes. Selected: direct fixture change. Implement only the requested file change.')
    if 'You review' in prompt:
        ids = _lr.findall(r'^- ([A-Za-z0-9_.-]+): ', prompt, _lr.M)
        _ledger_emit(_ledger_block('verdict', {'approved': True, 'findings': [], 'coverage': {i: {'status': 'ok', 'note': 'fixture comparison checked'} for i in ids}}))
"#;
    assert!(source.contains("json.dumps = _ledger_dumps"));
    std::fs::write(
        path,
        source.replace(
            "json.dumps = _ledger_dumps",
            &format!("json.dumps = _ledger_dumps\n{hook}"),
        ),
    )
    .unwrap();
}
