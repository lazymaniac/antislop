use serde_json::{Value, json};
use std::{process::Command, sync::Mutex};
#[path = "support/native_cli.rs"]
mod native_cli;

// Timing and child-startup fixtures must not compete with another benchmark in
// this test process. Keep their real child deadlines and cleanup checks intact.
static BENCHMARK_FIXTURE: Mutex<()> = Mutex::new(());

fn fixture(mode: &str) -> (tempfile::TempDir, Vec<String>) {
    let root = tempfile::tempdir().unwrap();
    let mut binaries = Vec::new();
    for name in ["baseline", "current"] {
        let source = root.path().join(name);
        std::fs::create_dir_all(source.join("src")).unwrap();
        std::fs::create_dir_all(source.join("target/debug")).unwrap();
        std::fs::write(source.join("Cargo.toml"), "[package]\nversion='0.1.0'\n").unwrap();
        std::fs::write(
            source.join("src/main.rs"),
            if name == "baseline" {
                "// before version\n"
            } else {
                "// after version\n"
            },
        )
        .unwrap();
        let binary = source.join("target/debug/antislop");
        let behavior = if name == "current" { mode } else { "good" };
        let program = format!(
            r#"
use std::{{env,fs,thread,time::Duration,process::{{self,Command}}}};
fn main() {{
 let args:Vec<_>=env::args().skip(1).collect();
 if args.first().is_some_and(|a|a=="__fixture_child") {{
  thread::sleep(Duration::from_secs(3)); fs::write(&args[1],"orphan").unwrap(); return;
 }}
 let mode={behavior:?};
 if mode=="hang" {{
  Command::new(env::current_exe().unwrap()).args(["__fixture_child",{late:?}]).spawn().unwrap();
  fs::write({started:?},"yes").unwrap(); thread::sleep(Duration::from_secs(20));
 }}
 match args.first().map(String::as_str) {{
  Some("--help") => println!("{{}}",if mode=="mismatch" {{ "changed help" }} else {{ "antislop help" }}),
  Some("--version") => println!("antislop 0.1.0"),
  _ => {{eprintln!("unknown command; use --help");process::exit(1);}}
 }}
}}
"#,
            late = root.path().join("late-effect").to_str().unwrap(),
            started = root.path().join("child-started").to_str().unwrap()
        );
        native_cli::build(root.path(), name, &program, &binary);
        binaries.push(binary);
    }
    let baseline_source = root.path().join("baseline");
    let hashes: serde_json::Map<String, Value> = ["Cargo.toml", "src/main.rs"]
        .into_iter()
        .map(|path| {
            (
                path.into(),
                json!(architect::hash(
                    std::fs::read(baseline_source.join(path)).unwrap()
                )),
            )
        })
        .collect();
    let manifest = root.path().join("baseline-manifest.json");
    std::fs::write(&manifest, serde_json::to_vec(&hashes).unwrap()).unwrap();
    let argv_file = root.path().join("argv.json");
    let generated = Command::new("python3")
        .arg("scripts/selfhost_benchmark.py")
        .arg("--baseline")
        .arg(&binaries[0])
        .arg("--current")
        .arg(&binaries[1])
        .arg("--baseline-manifest")
        .arg(&manifest)
        .arg("--output")
        .arg(&argv_file)
        .arg("--timeout")
        .arg(if mode == "hang" { "1" } else { "2" })
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let argv = serde_json::from_slice(&std::fs::read(argv_file).unwrap()).unwrap();
    (root, argv)
}

#[test]
fn benchmark_records_paired_raw_cpu_rss_tails_and_distinct_version_scaling() {
    let _measurement = BENCHMARK_FIXTURE.lock().unwrap_or_else(|e| e.into_inner());
    let (_root, argv) = fixture("good");
    assert_eq!(&argv[1..3], ["-I", "-c"]);
    let output = Command::new(&argv[0]).args(&argv[1..]).output().unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = report["measurements"].as_array().unwrap();
    assert_eq!(rows.len(), 5);
    for row in rows {
        let samples = row["samples"].as_array().unwrap();
        assert_eq!(samples.len(), 30);
        assert_eq!(
            samples
                .iter()
                .filter(|sample| sample[0] == "baseline")
                .count(),
            15
        );
        assert_eq!(
            samples
                .iter()
                .filter(|sample| sample[0] == "current")
                .count(),
            15
        );
        for sample in samples {
            assert_eq!(
                sample.as_array().unwrap().len(),
                row["sample_columns"].as_array().unwrap().len()
            );
        }
        for name in ["baseline", "current"] {
            let wall = &row[name]["wall_ns"];
            assert!(wall["p50"].as_f64().unwrap() > 0.0);
            assert!(wall["p99"].as_f64().unwrap() >= wall["p95"].as_f64().unwrap());
            assert!(row[name]["peak_rss_bytes"]["p50"].as_f64().unwrap() > 0.0);
        }
    }
    let versions = report["version_current_only"].as_array().unwrap();
    assert_eq!(versions.len(), 5);
    for row in versions {
        assert_eq!(row["comparable_baseline"], false);
        assert_eq!(row["samples"].as_array().unwrap().len(), 30);
        assert_eq!(row["verified_output"]["stdout"]["bytes"], 15);
    }
    assert_eq!(report["unknown_argv_scaling"].as_array().unwrap().len(), 4);
    assert!(report["plan"]["baseline_manifest_base64"].is_string());
    eprintln!("Dummy benchmark report bytes: {}", output.stdout.len());
}

#[test]
fn benchmark_rejects_equivalence_failure_and_post_approval_binary_mutation() {
    let _measurement = BENCHMARK_FIXTURE.lock().unwrap_or_else(|e| e.into_inner());
    let (_root, argv) = fixture("mismatch");
    let output = Command::new(&argv[0]).args(&argv[1..]).output().unwrap();
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["failure"]
            .as_str()
            .unwrap()
            .contains("differ from baseline")
    );
    let (root, argv) = fixture("good");
    std::fs::write(
        root.path().join("current/target/debug/antislop"),
        "replaced binary",
    )
    .unwrap();
    let output = Command::new(&argv[0]).args(&argv[1..]).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("current binary changed"));
}

#[test]
fn timed_out_benchmark_reaps_child_and_supervised_runner_cleans_descendants() {
    let _measurement = BENCHMARK_FIXTURE.lock().unwrap_or_else(|e| e.into_inner());
    let (root, argv) = fixture("hang");
    let (success, output) = architect::workspace::check(
        root.path(),
        &argv,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .unwrap();
    assert!(!success);
    assert!(root.path().join("child-started").exists());
    let receipt: Value = serde_json::from_str(&output).unwrap();
    let report: Value = serde_json::from_str(receipt["stdout"].as_str().unwrap()).unwrap();
    assert!(
        report["failure"]
            .as_str()
            .unwrap()
            .contains("individual child timeout")
    );
    std::thread::sleep(std::time::Duration::from_millis(3100));
    assert!(!root.path().join("late-effect").exists());
}
