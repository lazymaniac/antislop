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

/// Scheduler/mail fixtures predate guided sessions and intentionally exercise
/// persisted conversations without workflow state (including automatic plans).
/// New-session approval gates are covered in lean_workflow.
#[allow(dead_code)]
pub fn fixture_conversation(
    core: &antislop::lean::Core,
    project: &std::path::Path,
) -> antislop::Result<antislop::lean::store::Conversation> {
    let project = project.canonicalize()?;
    core.store.create_conversation(&project.to_string_lossy())
}

/// Keep file-writing fixtures from executing their writer branch during
/// the newly inserted read-only options turns. Dedicated options integration
/// tests supply their own comparison and verifier oracle instead.
#[allow(dead_code)]
pub fn install_task_options_protocol(path: &std::path::Path) {
    let source = std::fs::read_to_string(path).unwrap();
    let hook = r#"
if _lr.search(r'^Task: Options for ', prompt, _lr.M):
    if 'You are a worker' in prompt:
        _ledger_emit('Variants: direct fixture change; unnecessary abstraction. Criteria: satisfy the fixture contract with minimal changes. Selected: direct fixture change. Implement only the requested file change.')
    if 'You verify' in prompt:
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
