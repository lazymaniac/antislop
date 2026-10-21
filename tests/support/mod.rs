//! Extend the existing deterministic CLI fixtures with the ledger protocol.
//! Production behavior remains fully enabled; legacy worker/mail tests still
//! control their own task answers, while extraction gets a separate session.
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
