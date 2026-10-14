//! Independent offline benchmark and guardian regressions.
use std::{
    process::Command,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

#[test]
fn independent_benchmark_counterexamples_and_measurement_checks() {
    let output = Command::new("python3")
        .args(["tests/verifier_selfhost_benchmark.py", "-v"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn benchmark_timeout_reaps_specific_child_and_guardian_kills_descendant() {
    let root = tempfile::tempdir().unwrap();
    let generated = Command::new("python3")
        .args([
            "tests/verifier_selfhost_benchmark.py",
            "--emit-hanging-fixture",
        ])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let argv: Vec<String> = serde_json::from_slice(&generated.stdout).unwrap();
    let start = Instant::now();
    let (success, output) =
        architect::workspace::check(root.path(), &argv, Arc::new(AtomicBool::new(false))).unwrap();
    assert!(!success);
    assert!(
        start.elapsed() < Duration::from_secs(8),
        "timeout did not abort promptly"
    );
    let receipt: serde_json::Value = serde_json::from_str(&output).unwrap();
    let report: serde_json::Value =
        serde_json::from_str(receipt["stdout"].as_str().unwrap()).unwrap();
    assert!(
        report["failure"]
            .as_str()
            .unwrap()
            .contains("individual child timeout")
    );
    let timeout = report["observations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["workload"] == "timed-out-child")
        .unwrap();
    assert_eq!(timeout["sample"]["timed_out"], true);
    assert_eq!(timeout["sample"]["exit_code"], -libc::SIGKILL);
    assert!(
        root.path().join("measured-pid").exists(),
        "fixture never started: {report}"
    );
    assert!(
        root.path().join("measured-descendant-pid").exists(),
        "fixture never spawned a descendant: {report}"
    );
    let pid: i32 = std::fs::read_to_string(root.path().join("measured-pid"))
        .unwrap()
        .parse()
        .unwrap();
    // The measured process was already reaped with wait4 before benchmark exit.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    std::thread::sleep(Duration::from_millis(3100));
    assert!(
        !root.path().join("escaped-descendant-effect").exists(),
        "benchmark child escaped its enclosing guardian group"
    );
}
