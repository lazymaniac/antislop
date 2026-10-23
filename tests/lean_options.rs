mod common;

use antislop::{
    config::Config,
    lean::{
        Core,
        tasks::{self, TaskSpec},
    },
};
use serde_json::{Value, json};
use std::{
    process::Command,
    time::{Duration, Instant},
};

fn writer(id: &str) -> Value {
    json!({"id":id,"title":id,"goal":"Implement selected format","kind":"implementation","writes":true,"scope":["."],"verifier":{"focus":"selected format"}})
}

fn run(
    plan: Value,
    gates: bool,
    report: &str,
    implementation: &str,
    claim: &str,
) -> (Vec<antislop::lean::store::Task>, Vec<Value>, Vec<Value>) {
    run_with_requirement(plan, gates, report, implementation, claim, "A")
}

fn run_with_requirement(
    plan: Value,
    gates: bool,
    report: &str,
    implementation: &str,
    claim: &str,
    required_format: &str,
) -> (Vec<antislop::lean::store::Task>, Vec<Value>, Vec<Value>) {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("required-format.txt"), required_format).unwrap();
    for args in [
        vec!["init", "--quiet", "-b", "main"],
        vec!["add", "required-format.txt"],
        vec![
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "initial",
        ],
    ] {
        let out = Command::new("git")
            .arg("-C")
            .arg(&project)
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    }
    common::trust(&project);
    std::fs::write(
        dir.path().join("settings.json"),
        json!({"plan":plan,"report":report,"implementation":implementation,"claim":claim})
            .to_string(),
    )
    .unwrap();
    let codex = dir.path().join("codex");
    let agy = dir.path().join("agy");
    for path in [&codex, &agy] {
        common::install_probe_cli(path, include_str!("common/options_cli.py"));
    }
    let config = include_str!("../examples/antislop.toml")
        .replace("binary = \"codex\"", &format!("binary = {:?}", codex))
        .replace("binary = \"agy\"", &format!("binary = {:?}", agy))
        .replace(
            "required_gates = true",
            &format!("required_gates = {gates}"),
        )
        .replace("local_commits = true", "local_commits = false");
    let core = Core::open(
        &project.join(".antislop"),
        Config::from_toml(&config).unwrap(),
    )
    .unwrap();
    let conversation = core
        .store
        .create_conversation(project.to_str().unwrap())
        .unwrap();
    let idle = || {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !core.is_idle(&conversation.id) {
            assert!(Instant::now() < deadline, "runtime timeout");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    core.send(&conversation.id, "Implement selected format")
        .unwrap();
    idle();
    let plan = core.store.latest_plan(&conversation.id).unwrap();
    assert!(!plan.is_empty());
    core.accept_plan(&conversation.id, &plan[0].plan_id)
        .unwrap();
    idle();
    let tasks = core.store.latest_plan(&conversation.id).unwrap();
    for task in tasks.iter().filter(|task| task.spec["writes"] == true) {
        let file = project.join(format!("result-{}.txt", task.spec["id"].as_str().unwrap()));
        if task.status == "approved" {
            let expected = if implementation == "selected" {
                "A"
            } else {
                implementation
            };
            assert_eq!(std::fs::read_to_string(file).unwrap(), expected);
        } else {
            assert!(!file.exists(), "rejected implementation must not land");
        }
    }
    let calls = std::fs::read_to_string(dir.path().join("calls.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let events = core
        .store
        .events_since(&conversation.id, 0, 10000)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == "task")
        .map(|e| e.payload)
        .collect();
    (tasks, calls, events)
}

const REPORT: &str = "A: direct; B: index. Criteria: correctness, complexity. B adds needless state. Selected evidence: direct covers required format.\nSelected: A";

#[test]
fn orchestrator_plan_is_completed_before_each_writer_runs() {
    for gates in [false, true] {
        let (stored, calls, events) = run(
            json!({"tasks":[writer("w")]}),
            gates,
            REPORT,
            "selected",
            "",
        );
        let specs: Vec<TaskSpec> = stored
            .iter()
            .map(|t| serde_json::from_value(t.spec.clone()).unwrap())
            .collect();
        assert_eq!(tasks::with_options(specs.clone()), specs);
        let input: Vec<TaskSpec> = vec![serde_json::from_value(writer("w")).unwrap()];
        let normalize = |input| {
            tasks::with_options(if gates {
                tasks::with_required_agents(input)
            } else {
                input
            })
        };
        assert_eq!(normalize(input.clone()), normalize(input));
        assert_eq!(specs.iter().filter(|t| t.writes).count() > 1, gates);
        for spec in specs.iter().filter(|t| t.writes) {
            let option = specs
                .iter()
                .find(|o| {
                    spec.depends_on.contains(&o.id) && o.kind == antislop::config::WorkKind::Options
                })
                .unwrap();
            assert!(!option.writes);
            let option_row = stored.iter().find(|t| t.spec["id"] == option.id).unwrap();
            let writer_row = stored.iter().find(|t| t.spec["id"] == spec.id).unwrap();
            let done = events
                .iter()
                .position(|e| e["task"] == option_row.id && e["status"] == "approved")
                .unwrap_or_else(|| {
                    panic!(
                        "options not approved: {:?}",
                        stored
                            .iter()
                            .map(|t| (&t.spec["id"], &t.status, &t.report))
                            .collect::<Vec<_>>()
                    )
                });
            let start = events
                .iter()
                .position(|e| e["task"] == writer_row.id && e["status"] == "working")
                .unwrap();
            assert!(done < start);
            assert_eq!(writer_row.status, "approved");
            // The generated gates are absent from the planner input. Check their
            // own file bytes and verifier observations, not just prompt wording.
            let written = calls.iter().find(|c| c["written"] == spec.id).unwrap();
            assert_eq!(written["actual"], "A");
            let verified = calls.iter().find(|c| c["verified"] == spec.title).unwrap();
            assert_eq!(verified["files"][format!("result-{}.txt", spec.id)], "A");
            assert_eq!(verified["approved"], true);
            let worker = calls
                .iter()
                .find(|c| c["worker"] == true && c["task"] == spec.id)
                .unwrap()["prompt"]
                .as_str()
                .unwrap();
            assert!(worker.contains(REPORT));
            let verifier = calls
                .iter()
                .find(|c| c["worker"] == false && c["task"] == spec.title)
                .unwrap()["prompt"]
                .as_str()
                .unwrap();
            assert!(verifier.contains(REPORT));
        }
        let prompt = calls.iter().find(|c| c["task"] == "w-options").unwrap()["prompt"]
            .as_str()
            .unwrap();
        for text in [
            "2–4",
            "same explicit criteria",
            "Exclude variants violating requirements",
            "Selected: <id>",
        ] {
            assert!(prompt.contains(text), "{text}");
        }
    }
}

#[test]
fn shared_options_are_not_duplicated_and_bind_every_writer_even_with_ambiguous_reports() {
    for report in [
        REPORT,
        "A and B are plausible; no selection.",
        "For w: Selected: A\nFor x: Selected: B",
    ] {
        let mut w = writer("w");
        w["depends_on"] = json!(["shared"]);
        let mut x = writer("x");
        x["depends_on"] = json!(["shared", "w"]);
        let (stored, calls, _) = run(
            json!({"tasks":[{"id":"shared","title":"shared","goal":"Compare formats","kind":"options","writes":false,"verifier":{"focus":"selection"}},w,x]}),
            false,
            report,
            "A",
            "",
        );
        assert_eq!(stored.len(), 3);
        for id in ["w", "x"] {
            let prompt = calls
                .iter()
                .find(|c| c["worker"] == true && c["task"] == id)
                .unwrap()["prompt"]
                .as_str()
                .unwrap();
            assert!(prompt.contains(report));
            assert!(prompt.contains("options report is binding"));
            assert!(prompt.contains(
                "If a shared options report does not cover this task, report that gap explicitly"
            ));
        }
        // Approach A forwards reports verbatim: ambiguous/multiple decisions are
        // evidence gaps for the agents, not a structured runtime selection gate.
    }
}

#[test]
fn only_verifier_verdict_can_approve_worker_claims_and_departures() {
    let departure = r#"Departure: {"variant":"B","justification":"A violates the format required by the repository contract","evidence":"required-format.txt"}"#;
    for (claim, required_format, approved) in [
        (departure, "B", true),
        // Same claim, different actual evidence: the verifier must reject it.
        (departure, "A", false),
        (
            r#"Departure: {"variant":"B","justification":"","evidence":"required-format.txt"}"#,
            "B",
            false,
        ),
        (
            r#"Departure: {"variant":"B","justification":"A cannot satisfy the contract","evidence":"missing.txt"}"#,
            "B",
            false,
        ),
        ("I prefer B", "A", false),
        ("", "A", false),
    ] {
        let (stored, calls, events) = run_with_requirement(
            json!({"tasks":[writer("w")]}),
            false,
            REPORT,
            "B",
            claim,
            required_format,
        );
        let task = stored.iter().find(|t| t.spec["id"] == "w").unwrap();
        assert_eq!(task.status == "approved", approved, "{task:?}");
        assert_eq!(
            calls.iter().find(|c| c["written"] == "w").unwrap()["actual"],
            "B"
        );
        let checks: Vec<_> = calls.iter().filter(|c| c["verified"] == "w").collect();
        assert!(!checks.is_empty());
        for check in checks {
            assert_eq!(check["files"]["result-w.txt"], "B");
            assert_eq!(check["required"], required_format);
            assert_eq!(check["approved"], approved);
        }

        let worker = calls
            .iter()
            .find(|c| c["worker"] == true && c["task"] == "w")
            .unwrap()["prompt"]
            .as_str()
            .unwrap();
        for text in [
            "explicitly report and justify a departure with evidence",
            "including any discovered requirement violation",
            "Never silently switch variants",
        ] {
            assert!(worker.contains(text), "{text}");
        }
        assert!(task.verdict.is_some());
        assert_eq!(task.verdict.as_ref().unwrap()["approved"], approved);
        let verifier = calls
            .iter()
            .find(|c| c["worker"] == false && c["task"] == "w")
            .unwrap()["prompt"]
            .as_str()
            .unwrap();
        assert!(verifier.contains(claim));
        for text in [
            "independently compare the actual implementation",
            "Check the evidence and justification",
            "do not accept the worker's declaration as proof",
            "APPROVED. I complied",
        ] {
            assert!(verifier.contains(text), "{text}");
        }
        if !approved {
            assert!(
                !events
                    .iter()
                    .any(|e| e["task"] == task.id && e["status"] == "approved")
            );
        }
    }
}

#[test]
fn read_only_plan_runs_without_generated_options_or_quality_writers() {
    for gates in [false, true] {
        let input = json!({"id":"inspect", "title":"Inspect files", "goal":"Read the repository contract", "kind":"analysis", "writes":false, "scope":["."], "verifier":{"focus":"read-only findings"}});
        let (stored, calls, events) = run(json!({"tasks":[input.clone()]}), gates, REPORT, "A", "");
        assert_eq!(
            stored.len(),
            1,
            "read-only plans must not acquire options or writing gates"
        );
        let spec: TaskSpec = serde_json::from_value(stored[0].spec.clone()).unwrap();
        assert_eq!(spec.id, "inspect");
        assert!(!spec.writes);
        assert!(spec.depends_on.is_empty());
        assert_eq!(stored[0].status, "approved");
        assert!(stored[0].base.is_none());
        assert!(stored[0].result.is_none());
        assert!(
            stored[0]
                .report
                .as_deref()
                .unwrap()
                .contains("Inspected files read-only.")
        );
        let worker = calls.iter().find(|call| call["worker"] == true).unwrap();
        assert_eq!(worker["task"], "inspect");
        assert!(
            worker["prompt"]
                .as_str()
                .unwrap()
                .contains("Work read-only; do not modify anything.")
        );
        assert_eq!(
            calls.iter().filter(|call| call["worker"] == true).count(),
            1
        );
        assert!(
            events
                .iter()
                .any(|event| event["task"] == stored[0].id && event["status"] == "working")
        );
        assert!(
            events
                .iter()
                .any(|event| event["task"] == stored[0].id && event["status"] == "approved")
        );
    }
}
