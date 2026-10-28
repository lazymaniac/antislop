mod common;
mod support;

use antislop::{
    config::Config,
    lean::{
        Core,
        tasks::{self, TaskContract},
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
    run_scenario(
        plan,
        gates,
        report,
        implementation,
        claim,
        required_format,
        "",
    )
}

fn run_scenario(
    plan: Value,
    gates: bool,
    report: &str,
    implementation: &str,
    claim: &str,
    required_format: &str,
    scenario: &str,
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
        json!({"plan":plan,"report":report,"implementation":implementation,"claim":claim,"scenario":scenario})
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
    let conversation = support::fixture_conversation(&core, &project).unwrap();
    let idle = || {
        let deadline = Instant::now() + Duration::from_secs(120);
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
    if scenario.starts_with("mail-") {
        let deadline = Instant::now() + Duration::from_secs(120);
        while !dir.path().join("mail-ready").exists() {
            assert!(
                Instant::now() < deadline,
                "worker never reached mail barrier"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let task = core.task(&plan[0].plan_id, "w").unwrap();
        let role = format!("worker:{}", task.id);
        core.store
            .add_mail(
                &conversation.id,
                &role,
                &antislop::lean::store::Mail {
                    id: 0,
                    sender: "user".into(),
                    recipient: "w".into(),
                    kind: "message".into(),
                    text: "MAIL-CYCLE-SENTINEL".into(),
                    created_ms: 0,
                    delivered_turn: None,
                    delivered_ms: None,
                    state: "queued".into(),
                },
            )
            .unwrap();
        std::fs::write(dir.path().join("mail-release"), "go").unwrap();
    }
    idle();
    if scenario.starts_with("resume-") {
        let stopped = core.task(&plan[0].plan_id, "w").unwrap();
        assert_eq!(stopped.status, "waiting", "{stopped:?}");
        core.resume_plan(&conversation.id, &stopped.plan_id)
            .unwrap();
        idle();
    }
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
        let specs: Vec<TaskContract> = stored
            .iter()
            .map(|t| serde_json::from_value(t.spec.clone()).unwrap())
            .collect();
        assert_eq!(tasks::with_options(specs.clone()), specs);
        let input: Vec<TaskContract> = vec![serde_json::from_value(writer("w")).unwrap()];
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
            assert_selected_input(worker);
            let verifier = calls
                .iter()
                .find(|c| c["worker"] == false && c["task"] == spec.title)
                .unwrap()["prompt"]
                .as_str()
                .unwrap();
            assert_selected_input(verifier);
        }
        let prompt = calls.iter().find(|c| c["task"] == "w-options").unwrap()["prompt"]
            .as_str()
            .unwrap();
        for text in [
            "2–4",
            "Exclude variants violating requirements",
            "End with select",
        ] {
            assert!(prompt.contains(text), "{text}");
        }
    }
}

#[test]
fn shared_analysis_never_substitutes_for_dedicated_options() {
    let mut w = writer("w");
    w["depends_on"] = json!(["shared"]);
    let mut x = writer("x");
    x["depends_on"] = json!(["shared", "w"]);
    let (stored, calls, _) = run(
        json!({"tasks":[{"id":"shared","title":"shared","goal":"Compare formats","kind":"options","writes":false,"verifier":{"focus":"selection"}},w,x]}),
        false,
        REPORT,
        "A",
        "",
    );
    assert_eq!(stored.len(), 5);
    for id in ["w", "x"] {
        let local = stored.iter().find(|t| t.spec["options_for"] == id).unwrap();
        let target = stored.iter().find(|t| t.spec["id"] == id).unwrap();
        assert!(
            target.spec["depends_on"]
                .as_array()
                .unwrap()
                .contains(&local.spec["id"])
        );
        let prompt = calls
            .iter()
            .find(|c| c["worker"] == true && c["task"] == id)
            .unwrap()["prompt"]
            .as_str()
            .unwrap();
        assert_selected_input(prompt);
        assert!(!prompt.contains(REPORT));
    }
}

#[test]
fn only_verifier_verdict_can_approve_worker_claims_and_departures() {
    let departure = r#"Departure: {"variant":"B","justification":"A violates the format required by the repository contract","evidence":"required-format.txt"}"#;
    for (claim, required_format, approved) in [
        (departure, "B", false),
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
            "Ask the orchestrator",
            "changed selection requires fresh options",
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
            "Independently compare the implementation",
            "reviewed options",
            "the worker's declaration is not proof",
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
        let spec: TaskContract = serde_json::from_value(stored[0].spec.clone()).unwrap();
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

fn assert_core_delivery(scenario: &str) {
    let (stored, calls, _) = run_scenario(
        json!({"tasks":[writer("w")]}),
        false,
        REPORT,
        "selected",
        "",
        "A",
        scenario,
    );
    let task = stored.iter().find(|t| t.spec["id"] == "w").unwrap();
    assert_eq!(task.status, "approved", "{scenario}: {task:?}");
    let worker_calls: Vec<_> = calls
        .iter()
        .filter(|c| c["worker"] == true && c["task"] == "w")
        .collect();
    let verifier_calls: Vec<_> = calls
        .iter()
        .filter(|c| c["worker"] == false && c["task"] == "w")
        .collect();
    let continued = if scenario == "resume-verifier" {
        &verifier_calls
    } else {
        &worker_calls
    };
    assert_eq!(continued.len(), 2, "{scenario}: expected two CLI processes");
    assert_ne!(continued[0]["pid"], continued[1]["pid"]);
    assert!(
        !continued[0]["args"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "--conversation")
    );
    if scenario.starts_with("mail-") {
        assert_eq!(task.round, 1, "mail is handled within the same Core round");
    } else {
        assert_eq!(
            task.round, 2,
            "resume/rejection must advance the Core round"
        );
    }
    if !scenario.starts_with("mail-") {
        assert!(
            continued[1]["prompt"]
                .as_str()
                .unwrap()
                .contains("Round: 2")
        );
        assert!(
            continued[1]["prompt"]
                .as_str()
                .unwrap()
                .contains("Round handoff")
        );
    }
    if scenario == "resume-verifier" {
        assert_eq!(
            worker_calls.len(),
            2,
            "resume_plan reruns the worker before a fresh verifier round"
        );
    }
    let second = continued[1];
    let prompt = second["prompt"].as_str().unwrap();
    assert_selected_input(prompt);
    let args = second["args"].as_array().unwrap();
    assert_eq!(
        args.iter().any(|v| v == "--conversation"),
        scenario == "mail-existing",
        "{scenario}"
    );
    if scenario == "resume-verifier" {
        assert!(prompt.contains("You verify another agent's work."));
        assert!(prompt.contains("The worker's report:"));
        assert!(prompt.contains("Ask the orchestrator"));
    } else {
        assert!(prompt.contains("selected_approach"));
        assert!(prompt.contains("Never silently switch variants"));
        let branch = match scenario {
            "resume-worker" => "Your work on this task was interrupted",
            "reject-once" => "Your verifier found problems:",
            "mail-new" => "You are a worker on a team",
            "mail-existing" => "Apply the scoped user instruction.",
            _ => unreachable!(),
        };
        assert!(
            prompt.contains(branch),
            "{scenario}: wrong Core branch: {prompt}"
        );
    }
    if scenario.starts_with("mail-") {
        assert!(prompt.contains("MAIL-CYCLE-SENTINEL"));
    }
    if scenario == "reject-once" {
        assert_eq!(verifier_calls.len(), 2);
        let followup = verifier_calls[1]["prompt"].as_str().unwrap();
        assert!(followup.contains("You verify another agent's work."));
        assert!(followup.contains("The worker's report:"));
        assert_selected_input(followup);
        assert!(followup.contains("Ask the orchestrator"));
        let bytes: Vec<_> = calls
            .iter()
            .filter_map(|c| c["written"].as_str().map(|_| c["actual"].clone()))
            .collect();
        assert_eq!(bytes, vec![json!("B"), json!("A")]);
    }
}

#[test]
fn core_delivery_starts_fresh_worker_session_after_interruption() {
    assert_core_delivery("resume-worker");
}

#[test]
fn core_delivery_starts_fresh_verifier_session_after_interruption() {
    assert_core_delivery("resume-verifier");
}

#[test]
fn core_delivery_follows_rejection_in_both_roles() {
    assert_core_delivery("reject-once");
}

fn assert_selected_input(prompt: &str) {
    let inputs: Vec<Value> = prompt
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|value| value["variant"].is_object())
        .collect();
    assert_eq!(
        inputs.len(),
        1,
        "expected one local selection, not dependency reports"
    );
    assert!(inputs[0]["variant"]["id"].is_string());
    assert_eq!(
        inputs[0]["rationale"],
        "The focused approach satisfies this local contract"
    );
}

#[test]
fn an_independently_reviewed_local_choice_can_select_b() {
    let (tasks, _, _) = run_with_requirement(
        json!({"tasks":[writer("w")]}),
        false,
        &REPORT.replace("Selected: A", "Selected: B"),
        "B",
        "",
        "B",
    );
    assert_eq!(
        tasks.iter().find(|t| t.spec["id"] == "w").unwrap().status,
        "approved"
    );
}
