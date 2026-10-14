use serde_json::{Value, json};
use std::process::Command;
#[path = "support/native_cli.rs"]
mod native_cli;

fn fixture(wrong_output: bool, touch_untrusted: bool) -> (tempfile::TempDir, Vec<String>) {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("target/debug/antislop");
    std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
    let version = if wrong_output {
        "wrong-version"
    } else {
        "antislop 0.1.0"
    };
    let body = format!(
        r#"
use std::{{env,fs,process}};
fn main() {{
 let args: Vec<_> = env::args_os().skip(1).collect();
 let Ok(args) = args.into_iter().map(|a|a.into_string()).collect::<Result<Vec<_>,_>>() else {{ process::exit(1) }};
 if {touch_untrusted} && args.first().is_some_and(|a|a.starts_with("$(touch")) {{ fs::write("never-execute","").unwrap(); }}
 match args.first().map(String::as_str) {{
  Some("--version") => println!({version:?}),
  None | Some("--help") => println!("antislop help"),
  Some("list") => {{ let _=fs::create_dir("--version"); eprintln!("No such file or directory"); process::exit(1); }},
  _ => {{ eprintln!("unknown command; use --help"); process::exit(1); }}
 }}
}}
"#
    );
    native_cli::build(root.path(), "probe", &body, &binary);
    std::fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname='antislop'\nversion='0.1.0'\n",
    )
    .unwrap();
    let categories = [
        "missing",
        "empty",
        "zero",
        "negative",
        "minimum",
        "maximum",
        "boundary_neighbors",
        "oversized",
        "invalid_encoding",
        "unicode",
        "duplicate",
        "contradictory",
        "wrong_order",
        "partial",
        "untrusted",
        "path_normalization",
        "access_boundaries",
        "eof",
        "stream_error",
    ];
    let declared:Vec<_>=categories.iter().map(|category|json!({"category":category,"applicable":true,"scenario_ids":[match *category {"eof"=>"S-VERSION","stream_error"=>"S-STREAM","wrong_order"|"path_normalization"|"contradictory"=>"S-POSITION",_=>"S-ARGV"}],"expected_result":"fixture contract"})).collect();
    let content=json!({"quality":{"software":{"input_domain":{"inputs":[{"id":"cli-argv","source_ref":"file:src/main.rs","categories":declared}]}}}}).to_string();
    let run = json!({"spec":{"workspace":root.path()},"artifacts":[{"stage":"execution","hash":architect::hash(&content),"content":content}]});
    let run_path = root.path().join("run.json");
    std::fs::write(&run_path, run.to_string()).unwrap();
    let argv_path = root.path().join("argv.json");
    let generated = Command::new("python3")
        .args(["scripts/selfhost_probe.py", "--run-json"])
        .arg(&run_path)
        .arg("--output")
        .arg(&argv_path)
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let argv: Vec<String> = serde_json::from_slice(&std::fs::read(argv_path).unwrap()).unwrap();
    (root, argv)
}

#[test]
fn supplemental_probe_observes_declared_cases_with_literal_content_bound_argv() {
    let (root, argv) = fixture(false, false);
    assert_eq!(&argv[1..3], ["-I", "-c"]);
    assert_eq!(
        argv[4],
        root.path().canonicalize().unwrap().to_str().unwrap()
    );
    assert!(argv[3].contains("binary_sha256") && argv[3].contains("artifact_hash"));
    let output = Command::new(&argv[0]).args(&argv[1..]).output().unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = report["observations"].as_array().unwrap();
    assert!(rows.len() >= 20);
    for id in [
        "argv-empty",
        "argv-invalid-later-encoding",
        "position-state-value",
        "stdin-fd-zero-closed",
        "stdin-unread-bytes",
        "stdout-broken-pipe",
    ] {
        let row = rows.iter().find(|row| row["case_id"] == id).unwrap();
        assert!(row["exit_code"].is_number());
        assert!(row["assertion_failure"].is_null());
        assert_eq!(row["timed_out"], false);
    }
    assert_eq!(
        rows.iter()
            .find(|row| row["case_id"] == "version-normal")
            .unwrap()["stdout"]["utf8"],
        "antislop 0.1.0\n"
    );
    assert_eq!(
        rows.iter()
            .find(|row| row["case_id"] == "position-state-value")
            .unwrap()["state_directory_before"],
        false
    );
    assert_eq!(
        rows.iter()
            .find(|row| row["case_id"] == "position-state-value")
            .unwrap()["state_directory_after"],
        true
    );
    assert!(!root.path().join("never-execute").exists());
    assert_eq!(
        rows.iter()
            .find(|row| row["case_id"] == "argv-untrusted")
            .unwrap()["sentinel_created"],
        false
    );
}

#[test]
fn supplemental_probe_rejects_wrong_version_stdout_instead_of_recording_false_success() {
    let (_root, argv) = fixture(true, false);
    let output = Command::new(&argv[0]).args(&argv[1..]).output().unwrap();
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["failure"]
            .as_str()
            .unwrap()
            .contains("exact contract")
    );
    assert_eq!(
        report["observations"][0]["stdout"]["utf8"],
        "wrong-version\n"
    );
}

#[test]
fn supplemental_probe_rejects_untrusted_argument_effect_in_its_actual_private_cwd() {
    let (_root, argv) = fixture(false, true);
    let output = Command::new(&argv[0]).args(&argv[1..]).output().unwrap();
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["failure"]
            .as_str()
            .unwrap()
            .contains("filesystem effect")
    );
    let last = report["observations"].as_array().unwrap().last().unwrap();
    assert_eq!(last["case_id"], "argv-untrusted");
    assert_eq!(last["sentinel_created"], true);
}
