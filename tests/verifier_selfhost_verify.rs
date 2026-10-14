#[test]
fn independent_selected_review_status_and_evidence_checks_cannot_be_rehashed_away() {
    let output = std::process::Command::new("python3")
        .arg("-B")
        .arg("tests/verifier_selfhost_verify.py")
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
