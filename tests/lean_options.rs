mod common;
mod support;

use antislop::{
    config::Config,
    lean::{
        Core,
        tasks::{self, TaskContract},
    },
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    process::Command,
    time::{Duration, Instant},
};

fn writer(id: &str) -> Value {
    json!({"id":id,"title":id,"acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Implement selected format","change_kind": "code", "kind":"implementation","reviewer":{"summary": "selected format"}})
}

fn run(
    plan: Value,
    team: tasks::Team,
    report: &str,
    implementation: &str,
    claim: &str,
) -> (Vec<antislop::lean::store::Task>, Vec<Value>, Vec<Value>) {
    run_with_requirement(plan, team, report, implementation, claim, "A")
}

fn run_with_requirement(
    plan: Value,
    team: tasks::Team,
    report: &str,
    implementation: &str,
    claim: &str,
    required_format: &str,
) -> (Vec<antislop::lean::store::Task>, Vec<Value>, Vec<Value>) {
    run_scenario(
        plan,
        team,
        report,
        implementation,
        claim,
        required_format,
        "",
    )
}

fn run_scenario(
    plan: Value,
    team: tasks::Team,
    report: &str,
    implementation: &str,
    claim: &str,
    required_format: &str,
    scenario: &str,
) -> (Vec<antislop::lean::store::Task>, Vec<Value>, Vec<Value>) {
    run_scenario_with_team(
        plan,
        team,
        report,
        implementation,
        claim,
        required_format,
        scenario,
    )
}

fn run_scenario_with_team(
    plan: Value,
    team: tasks::Team,
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
    common::runtime_environment(&project);
    let authored_ids: HashSet<String> = plan["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|task| task["id"].as_str().map(str::to_owned))
        .collect();
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
        .replace("local_commits = true", "local_commits = false");
    let core = Core::open(
        &project.join(".antislop"),
        Config::from_toml(&config).unwrap(),
    )
    .unwrap();
    let conversation = support::fixture_conversation(&core, &project).unwrap();
    {
        // Integration tests exercise the Core scheduler directly, so seed the
        // same persisted user choice that the daemon's select_team action writes.
        Connection::open(project.join(".antislop/core.db"))
            .unwrap()
            .execute(
                "INSERT INTO teams(conversation_id,team,reason,version,updated_ms,user_selected,origin) VALUES(?1,?2,?3,?4,0,1,'user')",
                rusqlite::params![conversation.id, team.name(), "Selected in integration fixture", tasks::TEAM_VERSION],
            )
            .unwrap();
    }
    let mut accepted_plans = HashSet::new();
    let mut idle = || {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            for plan in core
                .store
                .events_since(&conversation.id, 0, 10_000)
                .unwrap()
                .into_iter()
                .filter(|event| {
                    event.kind == "plan_proposed" && event.payload["review"]["approved"] == true
                })
                .filter_map(|event| event.payload["plan"].as_str().map(str::to_owned))
            {
                if accepted_plans.insert(plan.clone()) {
                    core.resume_plan(&conversation.id, &plan).unwrap();
                }
            }
            if core.is_idle(&conversation.id) {
                break;
            }
            if implementation == "B" && scenario.is_empty() {
                // This oracle always rejects B. Verify repeated independent
                // rejection, then exercise explicit cancellation instead of
                // waiting for a removed runtime round limit.
                let rejected = std::fs::read_to_string(dir.path().join("calls.jsonl"))
                    .unwrap_or_default()
                    .lines()
                    .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                    .filter(|call| call["reviewed"] == "w" && call["approved"] == false)
                    .count();
                if rejected >= 2 {
                    core.cancel(&conversation.id);
                }
            }
            assert!(Instant::now() < deadline, "runtime timeout");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    core.send(&conversation.id, "Implement selected format")
        .unwrap();
    if !scenario.starts_with("mail-") {
        idle();
    }
    if scenario.starts_with("mail-") {
        let deadline = Instant::now() + Duration::from_secs(120);
        while !dir.path().join("mail-ready").exists() {
            assert!(
                Instant::now() < deadline,
                "worker never reached mail barrier"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let plan = core.store.latest_plan(&conversation.id).unwrap();
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
    let plan = core.store.latest_plan(&conversation.id).unwrap();
    assert!(
        !plan.is_empty(),
        "planning produced no tasks: {:?}",
        core.store.events_since(&conversation.id, 0, 100).unwrap()
    );
    if scenario.starts_with("resume-") {
        let stopped = core.task(&plan[0].plan_id, "w").unwrap();
        assert_eq!(stopped.status, "waiting", "{stopped:?}");
        core.resume_plan(&conversation.id, &stopped.plan_id)
            .unwrap();
        idle();
    }
    let tasks = core.store.latest_plan(&conversation.id).unwrap();
    let branch = support::session_branch(&core, &conversation.id);
    let integration_worktree = support::session_worktree_if_present(&project, &branch);

    for task in tasks.iter().filter(|task| {
        authored_ids.contains(task.spec["id"].as_str().unwrap_or_default())
            && task.spec["change_kind"]
                .as_str()
                .is_some_and(|kind| kind != "none")
    }) {
        let relative = format!("result-{}.txt", task.spec["id"].as_str().unwrap());
        let file = integration_worktree
            .as_ref()
            .map(|path| path.join(&relative));
        let delivered = match file.as_ref().map(std::fs::read_to_string).transpose() {
            Ok(Some(text)) => Some(text),
            Ok(None) => None,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("could not inspect integrated {relative}: {error}"),
        }
        .or_else(|| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&project)
                .args(["show", &format!("{branch}:{relative}")])
                .output()
                .unwrap();
            output
                .status
                .success()
                .then(|| String::from_utf8(output.stdout).unwrap())
        });
        if task.status == "approved" {
            let expected = if implementation == "selected" {
                "A"
            } else {
                implementation
            };
            assert_eq!(
                delivered.unwrap_or_else(|| {
                    panic!(
                        "missing integrated authored output {} for {task:?}",
                        relative
                    )
                }),
                expected
            );
        } else {
            assert!(delivered.is_none(), "rejected implementation must not land");
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
    for team in [tasks::Team::SmallChange, tasks::Team::Refactoring] {
        let (stored, calls, events) =
            run(json!({"tasks":[writer("w")]}), team, REPORT, "selected", "");
        let specs: Vec<TaskContract> = stored
            .iter()
            .map(|t| serde_json::from_value(t.spec.clone()).unwrap())
            .collect();
        assert_eq!(tasks::with_options(specs.clone()), specs);
        let input: Vec<TaskContract> = vec![serde_json::from_value(writer("w")).unwrap()];
        let normalize = |input| tasks::with_options(tasks::with_team_gates(input, team.gates()));
        assert_eq!(normalize(input.clone()), normalize(input));
        assert!(
            specs
                .iter()
                .filter(|t| t.change_kind.has_delivery())
                .count()
                > 1
        );
        for spec in specs.iter().filter(|t| t.change_kind.has_delivery()) {
            let writer_row = stored.iter().find(|t| t.spec["id"] == spec.id).unwrap();
            if spec.quality_gate {
                assert!(
                    specs
                        .iter()
                        .all(|option| option.options_for.as_deref() != Some(&spec.id))
                );
            } else {
                let option = specs
                    .iter()
                    .find(|option| option.options_for.as_deref() == Some(&spec.id))
                    .unwrap();
                assert!(spec.depends_on.contains(&option.id));
                assert!(!option.change_kind.has_delivery());
                let option_row = stored
                    .iter()
                    .find(|task| task.spec["id"] == option.id)
                    .unwrap();
                let done = events
                    .iter()
                    .position(|event| {
                        event["task"] == option_row.id && event["status"] == "approved"
                    })
                    .expect("options must be approved");
                let choice = &events[done]["feedback"]["selection"];
                assert!(
                    choice["approach"]
                        .as_str()
                        .is_some_and(|text| !text.is_empty())
                );
                assert_eq!(
                    choice["rationale"],
                    "The focused approach satisfies this local contract"
                );
                let mut view = antislop::lean::tui::view::View::default();
                view.apply(
                    &json!({"seq":1,"kind":"plan_proposed","payload":{"plan":"fixture",
                    "tasks":stored.iter().map(|task| task.spec.clone()).collect::<Vec<_>>()}}),
                );
                view.apply(&json!({"seq":2,"kind":"task","payload":events[done]}));
                assert!(view.cards.iter().any(|card| matches!(card,
                    antislop::lean::tui::TranscriptCard::System { title,text,.. }
                    if title.starts_with("Selected approach:") && text == choice["user_summary"].as_str().unwrap()
                        && !text.contains("The focused approach satisfies this local contract"))));
                let start = events
                    .iter()
                    .position(|event| {
                        event["task"] == writer_row.id && event["status"] == "working"
                    })
                    .unwrap();
                assert!(done < start);
            }
            assert_eq!(writer_row.status, "approved");
            // The generated gates are absent from the planner input. Check their
            // own file bytes and reviewer observations, not just prompt wording.
            let written = calls.iter().find(|c| c["written"] == spec.id).unwrap();
            assert_eq!(written["actual"], "A");
            let reviewed = calls.iter().find(|c| c["reviewed"] == spec.title).unwrap();
            assert_eq!(reviewed["files"][format!("result-{}.txt", spec.id)], "A");
            assert_eq!(reviewed["approved"], true);
            let worker = calls
                .iter()
                .find(|c| c["worker"] == true && c["task"] == spec.id)
                .unwrap()["prompt"]
                .as_str()
                .unwrap();
            if spec.quality_gate {
                assert!(!worker.contains("selected_approach"));
            } else {
                assert_selected_input(worker);
            }
            let reviewer = calls
                .iter()
                .find(|c| c["worker"] == false && c["task"] == spec.title)
                .unwrap()["prompt"]
                .as_str()
                .unwrap();
            if spec.quality_gate {
                assert!(!reviewer.contains("selected_approach"));
            } else {
                assert_selected_input(reviewer);
            }
        }
        let prompt = calls.iter().find(|c| c["task"] == "w-options").unwrap()["prompt"]
            .as_str()
            .unwrap();
        for text in [
            "explain if only one is feasible",
            "exclude variants that violate requirements",
            "Submit select",
        ] {
            assert!(prompt.contains(text), "{text}");
        }
    }
}

#[test]
fn options_worker_review_and_repair_rounds_share_one_checkout_in_role_order() {
    let (stored, calls, _) = run_scenario(
        json!({"tasks":[writer("w")]}),
        tasks::Team::SmallChange,
        REPORT,
        "A",
        "",
        "A",
        "shared-worktree-repair",
    );
    let options = stored
        .iter()
        .find(|task| task.spec["kind"] == "options")
        .unwrap();
    let writer_task = stored.iter().find(|task| task.spec["id"] == "w").unwrap();
    assert_eq!(options.status, "approved");
    assert_eq!(writer_task.status, "approved");

    let role_calls: Vec<_> = calls
        .iter()
        .filter(|call| {
            (call["worker"].as_bool() == Some(true)
                && (call["task"] == "w" || call["task"] == "w-options"))
                || call["reviewed"]
                    .as_str()
                    .is_some_and(|task| task == "w" || task == "Options for w")
        })
        .collect();
    let worker_calls: Vec<_> = role_calls
        .iter()
        .enumerate()
        .filter(|(_, call)| call["worker"] == true)
        .collect();
    assert_eq!(worker_calls.len(), 3);
    assert_eq!(worker_calls[0].1["task"], "w-options");
    assert_eq!(worker_calls[1].1["task"], "w");
    assert_eq!(worker_calls[2].1["task"], "w");
    let option_review = role_calls
        .iter()
        .position(|call| call["reviewed"] == options.spec["title"])
        .unwrap();
    let writer_reviews: Vec<_> = role_calls
        .iter()
        .enumerate()
        .filter(|(_, call)| call["reviewed"] == "w")
        .collect();
    assert!(!writer_reviews.is_empty());
    assert!(worker_calls[0].0 < option_review);
    assert!(option_review < worker_calls[1].0);
    assert!(writer_reviews[0].0 > worker_calls[1].0);
    assert!(writer_reviews[0].0 < worker_calls[2].0);
    assert!(writer_reviews[1].0 > worker_calls[2].0);
    assert!(writer_reviews[0].1["approved"] == false);
    assert!(writer_reviews.last().unwrap().1["approved"] == true);

    let checkout = role_calls[0]["cwd"].as_str().unwrap();
    assert!(checkout.contains("worktrees"));
    assert!(
        role_calls.iter().all(|call| call["cwd"] == checkout),
        "role cwd values: {:?}",
        role_calls
            .iter()
            .map(|call| (
                call.get("worker"),
                call.get("task"),
                call.get("reviewed"),
                call.get("cwd")
            ))
            .collect::<Vec<_>>()
    );
    assert_eq!(writer_reviews[0].1["files"]["result-w.txt"], "B");
    assert_eq!(
        writer_reviews.last().unwrap().1["files"]["result-w.txt"],
        "A"
    );
}

#[test]
fn shared_analysis_never_substitutes_for_dedicated_options() {
    let mut w = writer("w");
    w["depends_on"] = json!(["shared"]);
    let mut x = writer("x");
    x["depends_on"] = json!(["shared", "w"]);
    let (stored, calls, _) = run(
        json!({"tasks":[{"id":"shared","title":"shared","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Compare formats","change_kind": "none", "kind":"options","reviewer":{"summary": "selection"}},w,x]}),
        tasks::Team::SmallChange,
        REPORT,
        "A",
        "",
    );
    assert_eq!(stored.len(), 7);
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
fn only_reviewer_verdict_can_approve_worker_claims_and_departures() {
    let departure = r#"Departure: {"variant":"B","justification":"A violates the format required by the repository contract","evidence":"required-format.txt"}"#;
    for (claim, required_format, approved) in [
        (departure, "B", false),
        // Same claim, different actual evidence: the reviewer must reject it.
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
            tasks::Team::SmallChange,
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
        let checks: Vec<_> = calls.iter().filter(|c| c["reviewed"] == "w").collect();
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
        let reviewer = calls
            .iter()
            .find(|c| c["worker"] == false && c["task"] == "w")
            .unwrap()["prompt"]
            .as_str()
            .unwrap();
        assert!(reviewer.contains(claim));
        for text in [
            "Independently compare the implementation",
            "reviewed options",
            "the worker's declaration is not proof",
            "APPROVED. I complied",
        ] {
            assert!(reviewer.contains(text), "{text}");
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
    for team in [tasks::Team::SmallChange, tasks::Team::Refactoring] {
        let input = json!({"id":"inspect", "title":"Inspect files", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Read the repository contract", "change_kind": "none", "kind":"analysis",   "reviewer":{"summary": "read-only findings"}});
        let (stored, calls, events) = run(json!({"tasks":[input.clone()]}), team, REPORT, "A", "");
        assert_eq!(
            stored.len(),
            1,
            "read-only plans must not acquire options or writing gates"
        );
        let spec: TaskContract = serde_json::from_value(stored[0].spec.clone()).unwrap();
        assert_eq!(spec.id, "inspect");
        assert!(!spec.change_kind.has_delivery());
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
        assert!(worker["prompt"].as_str().unwrap().contains(
            "Deliver the analysis in your report; inspect files and run checks as needed."
        ));
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

#[test]
fn security_fix_needs_no_options_selection() {
    let (stored, calls, _) = run_scenario_with_team(
        json!({"tasks":[writer("w")]}),
        tasks::Team::SecurityFix,
        REPORT,
        "A",
        "",
        "A",
        "",
    );

    assert_eq!(stored.len(), 5);
    let writer_task = stored.iter().find(|task| task.spec["id"] == "w").unwrap();
    assert_eq!(writer_task.status, "approved");
    assert!(
        writer_task.spec["depends_on"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(stored.iter().all(|task| task.spec["options_for"].is_null()));

    let worker = calls
        .iter()
        .find(|call| call["worker"] == true && call["task"] == "w")
        .unwrap()["prompt"]
        .as_str()
        .unwrap();
    assert!(!worker.contains("selected_approach"));
    assert!(!worker.contains("Never silently switch variants"));
}

#[test]
fn small_change_runs_options_and_its_static_analysis_and_testing_gates_in_core_order() {
    let (stored, calls, events) = run_scenario_with_team(
        json!({"tasks":[writer("w")]}),
        tasks::Team::SmallChange,
        REPORT,
        "A",
        "",
        "A",
        "",
    );

    let specs: Vec<TaskContract> = stored
        .iter()
        .map(|task| serde_json::from_value(task.spec.clone()).unwrap())
        .collect();
    let by_id: std::collections::BTreeMap<_, _> =
        specs.iter().map(|spec| (spec.id.as_str(), spec)).collect();
    assert_eq!(
        by_id.keys().copied().collect::<Vec<_>>(),
        ["review-testing", "static-analyzer", "w", "w-options"]
    );
    assert_eq!(by_id["w"].kind, antislop::config::WorkKind::Implementation);
    assert_eq!(by_id["w-options"].kind, antislop::config::WorkKind::Options);
    assert_eq!(by_id["w-options"].options_for.as_deref(), Some("w"));
    assert_eq!(by_id["w"].depends_on, ["w-options"]);
    assert_eq!(
        by_id["static-analyzer"].kind,
        antislop::config::WorkKind::StaticAnalysis
    );
    assert_eq!(by_id["static-analyzer"].depends_on, ["w"]);
    assert_eq!(
        by_id["review-testing"].kind,
        antislop::config::WorkKind::Testing
    );
    assert_eq!(by_id["review-testing"].depends_on, ["w"]);
    assert!(by_id["static-analyzer"].quality_gate);
    assert!(by_id["review-testing"].quality_gate);
    assert!(stored.iter().all(|task| task.status == "approved"));

    assert_eq!(
        calls.iter().filter(|call| call["worker"] == true).count(),
        4,
        "the writer, its option and both quality gates must execute"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| { specs.iter().any(|spec| call["reviewed"] == spec.title) })
            .count(),
        specs.len(),
        "each generated role must receive its independent reviewer"
    );
    for spec in &specs {
        let worker_position = calls
            .iter()
            .position(|call| call["worker"] == true && call["task"] == spec.id)
            .unwrap_or_else(|| panic!("missing worker call for {}", spec.id));
        let reviewer_position = calls
            .iter()
            .position(|call| call["reviewed"] == spec.title)
            .unwrap_or_else(|| panic!("missing reviewer call for {}", spec.id));
        assert!(
            worker_position < reviewer_position,
            "{} must finish execution before its independent review",
            spec.id
        );
        let worker = calls.get(worker_position).unwrap();
        let reviewer = calls.get(reviewer_position).unwrap();
        assert_eq!(reviewer["approved"], true, "{}", spec.id);
        if spec.kind == antislop::config::WorkKind::Options {
            assert!(worker["prompt"].as_str().unwrap().contains("Submit select"));
            continue;
        }
        if spec.changes_code() {
            assert_eq!(
                reviewer["files"][format!("result-{}.txt", spec.id)],
                "A",
                "reviewer must inspect the actual probe output for {}",
                spec.id
            );
        }
    }

    let event_position = |runtime_id: &str, status: &str| {
        let task = stored
            .iter()
            .find(|task| task.spec["id"] == runtime_id)
            .unwrap();
        events
            .iter()
            .position(|event| event["task"] == task.id && event["status"] == status)
            .unwrap_or_else(|| panic!("missing {status} event for {runtime_id}"))
    };
    assert!(event_position("w-options", "approved") < event_position("w", "working"));
    for gate in ["static-analyzer", "review-testing"] {
        assert!(
            event_position("w", "approved") < event_position(gate, "working"),
            "{gate} must wait for the implementation"
        );
    }
}

#[test]
fn small_change_policy_boundaries_match_core_scheduled_graph() {
    let writer_plan = json!({"tasks":[writer("w")]});
    let cases = [
        (
            tasks::Team::SmallChange,
            vec!["w", "w-options", "review-testing", "static-analyzer"],
        ),
        (
            tasks::Team::SecurityFix,
            vec![
                "w",
                "review-testing",
                "static-analyzer",
                "security-auditor",
                "integration-gate",
            ],
        ),
    ];
    for (team, expected) in cases {
        let scenario = team.name();
        let (stored, calls, events) =
            run_scenario_with_team(writer_plan.clone(), team, REPORT, "A", "", "A", scenario);
        let mut actual: Vec<_> = stored
            .iter()
            .map(|task| task.spec["id"].as_str().unwrap())
            .collect();
        actual.sort_unstable();
        let mut expected = expected;
        expected.sort_unstable();
        assert_eq!(actual, expected, "{scenario}");
        assert!(
            stored.iter().all(|task| task.status == "approved"),
            "{scenario}"
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| {
                    stored
                        .iter()
                        .any(|task| call["reviewed"] == task.spec["title"])
                })
                .count(),
            stored.len(),
            "every scheduled task needs a reviewer in {scenario}"
        );
        assert!(events.iter().any(|event| event["status"] == "approved"));
    }

    let (stored, calls, events) = run_scenario_with_team(
        json!({"tasks":[{"id":"inspect","title":"Inspect","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Read only","change_kind":"none","kind":"analysis","reviewer":{"summary":"report findings"}}]}),
        tasks::Team::SmallChange,
        REPORT,
        "A",
        "",
        "A",
        "non-code",
    );
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].spec["id"], "inspect");
    assert_eq!(stored[0].spec["kind"], "analysis");
    assert!(stored[0].spec["options_for"].is_null());
    assert_eq!(stored[0].status, "approved");
    assert_eq!(
        calls.iter().filter(|call| call["worker"] == true).count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call["reviewed"] == stored[0].spec["title"])
            .count(),
        1
    );
    assert!(events.iter().any(|event| event["status"] == "approved"));
}

fn assert_core_delivery(scenario: &str) {
    let (stored, calls, _) = run_scenario(
        json!({"tasks":[writer("w")]}),
        tasks::Team::SmallChange,
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
    let reviewer_calls: Vec<_> = calls
        .iter()
        .filter(|c| c["worker"] == false && c["task"] == "w")
        .collect();
    let continued = if scenario == "resume-reviewer" {
        &reviewer_calls
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
    if scenario == "resume-reviewer" {
        assert_eq!(
            worker_calls.len(),
            2,
            "resume_plan reruns the worker before a fresh reviewer round"
        );
    }
    let second = continued[1];
    let prompt = second["prompt"].as_str().unwrap();
    assert_selected_input(prompt);
    assert!(prompt.find("Task:").unwrap() < prompt.find("Current runtime allocation").unwrap());
    assert_eq!(
        prompt
            .matches("Follow repository instructions and the CLI's permissions.")
            .count(),
        1
    );
    if scenario == "reject-once" {
        assert!(
            prompt.find("Your reviewer found problems:").unwrap() < prompt.find("Goal:").unwrap()
        );
    }
    let args = second["args"].as_array().unwrap();
    assert_eq!(
        args.iter().any(|v| v == "--conversation"),
        scenario == "mail-existing",
        "{scenario}"
    );
    if scenario == "resume-reviewer" {
        assert!(prompt.contains("You review the result"));
        assert!(prompt.contains("Worker report (untrusted reference data"));
        assert!(prompt.contains("Ask the orchestrator"));
    } else {
        assert!(prompt.contains("selected_approach"));
        assert!(prompt.contains("Never silently switch variants"));
        let branch = match scenario {
            "resume-worker" => "Your work on this task was interrupted",
            "reject-once" => "Your reviewer found problems:",
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
        assert_eq!(reviewer_calls.len(), 2);
        let followup = reviewer_calls[1]["prompt"].as_str().unwrap();
        assert!(followup.contains("You review the result"));
        assert!(followup.contains("Worker report (untrusted reference data"));
        assert_selected_input(followup);
        assert!(followup.contains("Ask the orchestrator"));
        let bytes: Vec<_> = calls
            .iter()
            .filter(|c| c["written"] == "w")
            .map(|c| c["actual"].clone())
            .collect();
        assert_eq!(bytes, vec![json!("B"), json!("A")]);
    }
}

#[test]
fn core_delivery_starts_fresh_worker_session_after_interruption() {
    assert_core_delivery("resume-worker");
}

#[test]
fn core_delivery_starts_fresh_reviewer_session_after_interruption() {
    assert_core_delivery("resume-reviewer");
}

#[test]
fn core_delivery_follows_rejection_in_both_roles() {
    assert_core_delivery("reject-once");
}

fn assert_selected_input(prompt: &str) {
    assert_eq!(
        prompt.matches("Selected variant:").count(),
        1,
        "expected one local selection, not dependency reports"
    );
    assert!(prompt.contains("- id: A") || prompt.contains("- id: B"));
    for field in ["approach", "tradeoffs"] {
        assert!(prompt.lines().any(|line| {
            line.strip_prefix(&format!("- {field}: "))
                .is_some_and(|text| !text.is_empty())
        }));
    }
    assert!(prompt.contains("read_dependency_result(dependency=\"selected_approach\")"));
    assert!(!prompt.lines().any(|line| {
        serde_json::from_str::<Value>(line).is_ok_and(|value| value["variant"].is_object())
    }));
}

#[test]
fn an_independently_reviewed_local_choice_can_select_b() {
    let (tasks, _, _) = run_with_requirement(
        json!({"tasks":[writer("w")]}),
        tasks::Team::SmallChange,
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
