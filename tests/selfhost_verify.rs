#[test]
fn independent_selfhost_acceptance_rejects_forged_or_incomplete_exports() {
    let output = std::process::Command::new("python3")
        .arg("-B")
        .arg("tests/selfhost_verify.py")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
