//! Extend the existing deterministic CLI fixtures with the ledger protocol.
//! Production behavior remains fully enabled; legacy worker/mail tests still
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
pub fn legacy_conversation(
    core: &antislop::lean::Core,
    project: &std::path::Path,
) -> antislop::Result<antislop::lean::store::Conversation> {
    let project = project.canonicalize()?;
    core.store.create_conversation(&project.to_string_lossy())
}
