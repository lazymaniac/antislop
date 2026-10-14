//! Native local CLI fixture: startup must not depend on a framework interpreter
//! or execution of newly written shebang scripts. Deadlines/oracles stay intact.
use std::path::Path;
pub fn build(root: &Path, name: &str, program: &str, binary: &Path) {
    let source = root.join(format!("fixture-{name}.rs"));
    std::fs::write(&source, program).unwrap();
    let output = std::process::Command::new("rustc")
        .args([
            "--edition=2024",
            "--deny",
            "warnings",
            "--crate-name",
            "native_cli_fixture",
        ])
        .arg(&source)
        .arg("-o")
        .arg(binary)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "native fixture compile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
