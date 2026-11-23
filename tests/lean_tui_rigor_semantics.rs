//! End-to-end rigor selection through the TUI and the real daemon/Core.
mod common;
mod support;

use antislop::{
    config::Config,
    lean::{
        Core,
        daemon::{Client, ControlRequest, HistoryRequest, Request},
        tasks::Team,
        tui::{App, Command, InputBuffer, Update},
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::{Child, Command as ProcessCommand, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Daemon {
    child: Child,
    client: Client,
}

impl Daemon {
    fn start(project: &Path, config: &Path, state: &Path, registry: &Path) -> Self {
        common::assert_test_socket(state, Path::new("."));
        let child = ProcessCommand::new(env!("CARGO_BIN_EXE_antislop"))
            .args(["__core"])
            .arg(project)
            .arg(config)
            .arg(state)
            .env("ANTISLOP_CONFIG_DIR", registry)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let daemon = Self {
            child,
            client: Client::at(state),
        };
        wait_until("daemon identity", || {
            daemon
                .client
                .call(&Request::Control(ControlRequest::Identity))
                .is_ok()
        });
        daemon
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self
            .client
            .call(&Request::Control(ControlRequest::ForceShutdown));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(20));
    }
}

fn setup(team: Team, root: &Path) -> (String, Daemon, PathBuf, PathBuf, PathBuf) {
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    common::initialize_git_project(&project);
    std::fs::write(project.join("required-format.txt"), "A\n").unwrap();
    let fixture_commit = ProcessCommand::new("git")
        .args([
            "-C",
            project.to_str().unwrap(),
            "add",
            "required-format.txt",
        ])
        .output()
        .unwrap();
    assert!(
        fixture_commit.status.success(),
        "git add failed: {fixture_commit:?}"
    );
    let fixture_commit = ProcessCommand::new("git")
        .args([
            "-C",
            project.to_str().unwrap(),
            "-c",
            "user.name=Antislop Test",
            "-c",
            "user.email=antislop-test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "add deterministic provider fixture input",
        ])
        .output()
        .unwrap();
    assert!(
        fixture_commit.status.success(),
        "fixture commit failed: {fixture_commit:?}"
    );
    common::runtime_environment(&project);

    let provider_root = root.join("providers");
    std::fs::create_dir_all(&provider_root).unwrap();
    let request = match team {
        Team::CodeReview => {
            "Review commit 383364e4f3c0983f0c387ce39e13093f0fb6be16 for correctness."
        }
        Team::Research => "Research how the project currently handles rigor selection.",
        Team::Document => "Write a short guide explaining the current rigor choices.",
        _ => "Implement a contained behavior change for rigor selection.",
    };
    let plan = plan_for(team);
    let primary_title = match team {
        Team::CodeReview => "Inspect rigor flow",
        Team::Research => "Research rigor flow",
        Team::Document => "Document rigor flow",
        _ => "Change rigor flow",
    };
    let settings = serde_json::json!({
        "plan": plan,
        "report": "A: direct; B: indexed. Criteria: correctness and complexity. B adds needless state. Selected evidence: direct preserves the existing contract.\nSelected: A",
        "implementation": "A",
        "claim": "The requested result is present.",
        "scenario": "",
        "barrier_id": "w",
        "barrier_title": primary_title
    });
    std::fs::write(provider_root.join("settings.json"), settings.to_string()).unwrap();
    let codex = provider_root.join("codex");
    let agy = provider_root.join("agy");
    let fixture_cli = include_str!("common/options_cli.py")
        .replace(
            "worker = 'You are a worker' in prompt or previous.startswith('worker:')",
            "options_role = 'This is an options task:' in prompt\n    worker = 'You are a worker' in prompt or previous.startswith('worker:') or options_role",
        )
        .replace(
            "if task.endswith('-options') or task == 'shared':",
            "if task.endswith('-options') or task == 'shared' or options_role:",
        )
        .replace(
            "        elif ('Deliver the analysis in your report;' in prompt or 'Do not edit tracked files.' in prompt\n              or any(t['id'] == task and t['change_kind'] == 'none' for t in settings['plan']['tasks'])):",
            "        elif any(t['change_kind'] == 'artifact' and t['goal'] in prompt for t in settings['plan']['tasks']):\n            for authored in settings['plan']['tasks']:\n                if authored['change_kind'] == 'artifact':\n                    for output in authored.get('outputs', []):\n                        os.makedirs(os.path.dirname(output) or '.', exist_ok=True)\n                        open(output, 'w').write('Fixture artifact for rigor semantics.\\n')\n            answer = 'APPROVED. The declared artifact is present.'\n        elif ('Deliver the analysis in your report;' in prompt or 'Do not edit tracked files.' in prompt\n              or any(t['id'] == task and t['change_kind'] == 'none' for t in settings['plan']['tasks'])):",
        )
        .replace(
            "task = task.group(1) if task else previous.split(':', 1)[-1]",
            "task = task.group(1) if task else previous.split(':', 1)[-1]\n    if options_role:\n        task = 'w-options'\n    if task in (settings.get('barrier_id'), settings.get('barrier_title')):\n        open(os.path.join(root, 'writer-ready'), 'w').close()\n        import time\n        deadline = time.monotonic() + 120\n        while not os.path.exists(os.path.join(root, 'writer-release')):\n            if time.monotonic() > deadline:\n                raise RuntimeError('writer barrier was not released')\n            time.sleep(0.01)",
        );
    let fixture_cli = fixture_cli.replace(
        "answer = 'Inspected files read-only.'",
        "answer = ('N/A: inspected required-format.txt and the fixture imports; no dependency changes are needed.' if task == 'feature-dependencies' else 'Inspected files read-only.')",
    );
    assert!(fixture_cli.contains("Fixture artifact for rigor semantics."));
    for executable in [&codex, &agy] {
        common::install_probe_cli(executable, &fixture_cli);
    }
    let config_text = include_str!("../examples/antislop.toml")
        .replace("binary = \"codex\"", &format!("binary = {:?}", codex))
        .replace("binary = \"agy\"", &format!("binary = {:?}", agy))
        .replace("local_commits = true", "local_commits = false");
    let config_path = root.join("antislop.toml");
    std::fs::write(&config_path, &config_text).unwrap();
    let workspace = std::env::current_dir().unwrap();
    let state = project.strip_prefix(&workspace).unwrap().join(".antislop");
    let config = Config::from_toml(&config_text).unwrap();
    let core = Core::open(&project.join(".antislop"), config).unwrap();
    let conversation =
        support::fixture_conversation_with_requirements(&core, &project, request).unwrap();
    drop(core);
    let daemon = Daemon::start(&project, &config_path, &state, &root.join("registry"));
    (
        conversation.id,
        daemon,
        PathBuf::from(request),
        project.join(".antislop/core.db"),
        provider_root,
    )
}

fn stored_tasks(database: &Path, conversation: &str) -> Vec<(String, String, Value, String)> {
    let connection = rusqlite::Connection::open(database).unwrap();
    let mut statement = connection
        .prepare("SELECT id,plan_id,spec,status FROM tasks WHERE conversation_id=?1 ORDER BY created_ms,rowid")
        .unwrap();
    statement
        .query_map([conversation], |row| {
            let spec: String = row.get(2)?;
            Ok((
                row.get(0)?,
                row.get(1)?,
                serde_json::from_str(&spec).unwrap(),
                row.get(3)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn stored_events(database: &Path, conversation: &str) -> Vec<Value> {
    let connection = rusqlite::Connection::open(database).unwrap();
    let mut statement = connection
        .prepare(
            "SELECT seq,plan_id,kind,payload FROM events WHERE conversation_id=?1 ORDER BY seq",
        )
        .unwrap();
    statement
        .query_map([conversation], |row| {
            let payload: String = row.get(3)?;
            Ok(serde_json::json!({
                "seq":row.get::<_,i64>(0)?, "plan":row.get::<_,Option<String>>(1)?,
                "kind":row.get::<_,String>(2)?, "payload":serde_json::from_str::<Value>(&payload).unwrap()
            }))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn daemon_idle(client: &Client, conversation: &str) -> bool {
    client
        .call(&Request::History(HistoryRequest::Events {
            conversation: conversation.to_owned(),
            after: u64::MAX,
            wait_ms: 0,
        }))
        .is_ok_and(|reply| reply["idle"] == true)
}

fn plan_for(team: Team) -> Value {
    let (kind, change_kind, title, goal) = match team {
        Team::CodeReview => (
            "analysis",
            "none",
            "Inspect rigor flow",
            "Review the selected commit for rigor selection defects.",
        ),
        Team::Research => (
            "research",
            "none",
            "Research rigor flow",
            "Collect evidence about rigor selection.",
        ),
        Team::Document => (
            "writing",
            "artifact",
            "Document rigor flow",
            "Write a guide to rigor selection.",
        ),
        _ => (
            "implementation",
            "code",
            "Change rigor flow",
            "Implement the requested rigor selection behavior.",
        ),
    };
    let mut plan = serde_json::json!({"tasks":[{
        "id":"w", "title":title, "goal":goal,
        "acceptance_criteria":[{"id":"C1","text":goal}],
        "kind":kind, "change_kind":change_kind,
        "outputs": if team == Team::Document { serde_json::json!(["RIGOR.md"]) } else { serde_json::json!([]) },
        "reviewer":{"summary":"Verify the task result against its acceptance criterion."}
    }]});
    if team == Team::Feature {
        let tasks = plan["tasks"].as_array_mut().unwrap();
        tasks[0]["depends_on"] = serde_json::json!(["feature-architecture", "feature-test-design"]);
        for (id, kind, inputs) in [
            ("feature-research", "research", vec![]),
            ("feature-analysis", "analysis", vec![]),
            (
                "feature-dependencies",
                "dependencies",
                vec!["feature-research"],
            ),
            (
                "feature-architecture",
                "architecture",
                vec!["feature-analysis", "feature-dependencies"],
            ),
            (
                "feature-test-design",
                "test_design",
                vec!["feature-analysis"],
            ),
            (
                "feature-delivery",
                "delivery",
                vec!["integration-gate", "w"],
            ),
        ] {
            tasks.push(serde_json::json!({"id":id,"kind":kind,"title":id,"goal":format!("Assess {kind} for the fixture feature; report evidence or justified N/A"),"change_kind":"none","depends_on":inputs,"acceptance_criteria":[{"id":"C1","text":"Report the scoped assessment or justified N/A"}]}));
        }
    }
    plan
}

// Contract oracle kept separate from Team::gates()/Team::options() so the
// assertions can detect drift in the implementation being exercised.
fn expected_gates(team: Team) -> &'static [&'static str] {
    match team {
        Team::SmallChange => &["static_analysis", "testing"],
        Team::Feature => &[
            "refactoring",
            "static_analysis",
            "testing",
            "security",
            "performance",
            "integration",
        ],
        Team::Refactoring => &["refactoring", "static_analysis", "testing", "integration"],
        Team::SecurityFix => &["static_analysis", "testing", "security", "integration"],
        Team::CodeReview | Team::NoQualityGates | Team::Research | Team::Document => &[],
    }
}

fn expected_options(team: Team) -> bool {
    matches!(team, Team::SmallChange | Team::Feature | Team::Refactoring)
}

fn replay_events(
    app: &mut App,
    client: &Client,
    conversation: &str,
    after: &mut u64,
) -> Vec<Value> {
    let reply = client
        .call(&Request::History(HistoryRequest::Events {
            conversation: conversation.to_owned(),
            after: *after,
            wait_ms: 0,
        }))
        .unwrap();
    let events = reply["events"].as_array().cloned().unwrap_or_default();
    if let Some(seq) = events.last().and_then(|event| event["seq"].as_u64()) {
        *after = seq;
    }
    app.apply_update(Update::Events {
        conversation: conversation.to_owned(),
        generation: app.generation,
        events: events.clone(),
        idle: reply["idle"] == true,
        observer: false,
    });
    events
}

fn replay_all_events(app: &mut App, client: &Client, conversation: &str) {
    let mut after = 0;
    loop {
        if replay_events(app, client, conversation, &mut after).is_empty() {
            break;
        }
    }
}

fn select_from_composer_or_palette(app: &mut App, team: Team, slash: bool) {
    if slash {
        app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        app.key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    } else {
        app.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    }
    for ch in team.name().chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
}

fn drain_updates(app: &mut App, updates: &antislop::lean::tui::UpdateReceiver) {
    while let Ok(update) = updates.try_recv() {
        app.apply_update(update);
    }
}

#[test]
fn all_eight_rigors_drive_real_core_plans_and_direct_execution() {
    let cases = [
        Team::SmallChange,
        Team::Feature,
        Team::Refactoring,
        Team::SecurityFix,
        Team::CodeReview,
        Team::NoQualityGates,
        Team::Research,
        Team::Document,
    ];

    for (index, team) in cases.into_iter().enumerate() {
        let root = common::tempdir();
        let (conversation, daemon, request, database, provider_root) = setup(team, root.path());
        let (mut app, updates) = App::test_app_with_client(daemon.client.clone());
        app.conversation = conversation.clone();

        select_from_composer_or_palette(&mut app, team, index % 2 == 1);
        loop {
            match updates.recv_timeout(Duration::from_secs(20)).unwrap() {
                Update::Notice(text) if text == "workflow rigor updated" => {
                    app.apply_update(Update::Notice(text));
                    break;
                }
                Update::Notice(text) => panic!("team selection failed in UI: {text}"),
                update => app.apply_update(update),
            }
        }

        let mut after = 0;
        let selected = loop {
            let events = replay_events(&mut app, &daemon.client, &conversation, &mut after);
            if events.iter().any(|event| {
                event["kind"] == "team_selected" && event["payload"]["team"] == team.name()
            }) {
                break true;
            }
            thread::sleep(Duration::from_millis(20));
        };
        assert!(selected);
        assert_eq!(app.view.rigor_label(), team.name());

        app.input = InputBuffer::with_text(request.to_string_lossy());
        app.submit();
        wait_until("composer send result", || {
            drain_updates(&mut app, &updates);
            !app.sending
        });
        let plan_event = loop {
            let _ = replay_events(&mut app, &daemon.client, &conversation, &mut after);
            if let Some(event) = stored_events(&database, &conversation)
                .into_iter()
                .rev()
                .find(|event| {
                    event["kind"] == "plan_proposed"
                        && event["payload"]["review"]["approved"] == true
                })
            {
                break event;
            }
            thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(plan_event["payload"]["review"]["approved"], true);
        let (mut acceptance_app, _) = App::test_app_with_client(daemon.client.clone());
        acceptance_app.conversation = conversation.clone();
        replay_all_events(&mut acceptance_app, &daemon.client, &conversation);
        assert!(acceptance_app.view.plan_proposed());
        let plan_id = acceptance_app
            .view
            .plan
            .clone()
            .expect("replayed UI shows the reviewed plan");
        assert!(
            stored_events(&database, &conversation).iter().any(|event| {
                event["kind"] == "plan_proposed"
                    && event["payload"]["plan"] == plan_id
                    && event["payload"]["review"]["approved"] == true
            }),
            "the plan shown by the UI has an approved Core review: {plan_id}"
        );
        acceptance_app.run_workflow(Command::AcceptPlan);

        wait_until("primary task handshake", || {
            provider_root.join("writer-ready").exists()
        });
        let at_barrier: Vec<_> = stored_tasks(&database, &conversation)
            .into_iter()
            .filter(|task| task.1 == plan_id)
            .collect();
        assert!(
            at_barrier
                .iter()
                .any(|task| task.2["id"] == "w" && task.3 == "working"),
            "the selected plan's primary task reached its provider barrier for {}: {at_barrier:?}",
            team.name()
        );
        if expected_options(team) {
            assert!(
                at_barrier
                    .iter()
                    .any(|task| task.2["options_for"] == "w" && task.3 == "approved"),
                "the options prerequisite must be approved before the writer starts for {}: {at_barrier:?}",
                team.name()
            );
        }
        for kind in expected_gates(team) {
            assert!(
                at_barrier
                    .iter()
                    .any(|task| task.2["kind"] == *kind && task.3 == "queued"),
                "quality gates stay queued while the producer is held for {} ({kind}): {at_barrier:?}",
                team.name()
            );
        }
        if team == Team::NoQualityGates {
            assert!(
                at_barrier.iter().any(|task| {
                    task.2["kind"] == "integration"
                        && task.2["quality_gate"] != true
                        && task.2["depends_on"]
                            .as_array()
                            .is_some_and(|deps| deps.contains(&Value::String("w".into())))
                        && task.3 == "queued"
                }),
                "the no-quality-gates final integration waits for the writer but is not a quality gate: {at_barrier:?}"
            );
        }
        std::fs::write(provider_root.join("writer-release"), "release").unwrap();

        let completion_deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let current = stored_tasks(&database, &conversation);
            let plan_tasks: Vec<_> = current.iter().filter(|task| task.1 == plan_id).collect();
            if !plan_tasks.is_empty()
                && plan_tasks.iter().all(|task| task.3 == "approved")
                && daemon_idle(&daemon.client, &conversation)
            {
                break;
            }
            assert!(
                Instant::now() < completion_deadline,
                "timed out waiting for team plan completion for {}: tasks={plan_tasks:?}, events={:?}",
                team.name(),
                stored_events(&database, &conversation)
                    .into_iter()
                    .filter(|event| event["plan"] == plan_id)
                    .collect::<Vec<_>>()
            );
            thread::sleep(Duration::from_millis(20));
        }
        let tasks: Vec<_> = stored_tasks(&database, &conversation)
            .into_iter()
            .filter(|task| task.1 == plan_id)
            .collect();
        assert!(
            !tasks.is_empty(),
            "Core must materialize tasks for {}",
            team.name()
        );
        let finalized_events = stored_events(&database, &conversation);
        assert!(
            tasks
                .iter()
                .filter(|task| task.2["kind"] != "integration")
                .all(|task| {
                    finalized_events.iter().any(|event| {
                        event["kind"] == "turn_started"
                            && event["plan"] == plan_id
                            && event["payload"]["role"] == format!("reviewer:{}", task.0)
                    }) && finalized_events.iter().any(|event| {
                        event["kind"] == "turn_finished"
                            && event["plan"] == plan_id
                            && event["payload"]["role"] == format!("reviewer:{}", task.0)
                            && event["payload"]["status"] == "done"
                    })
                }),
            "each finalized {} task has a completed reviewer turn",
            team.name()
        );
        let has_options = tasks.iter().any(|task| !task.2["options_for"].is_null());
        let has_quality_gates = tasks.iter().any(|task| task.2["quality_gate"] == true);
        assert_eq!(
            has_options,
            expected_options(team),
            "options policy for {}",
            team.name()
        );
        match team {
            Team::SmallChange => {
                assert!(has_options);
                assert!(tasks.iter().any(|task| task.2["kind"] == "static_analysis"));
                assert!(tasks.iter().any(|task| task.2["kind"] == "testing"));
            }
            Team::Feature => {
                assert!(has_options);
                assert!(has_quality_gates);
                for kind in [
                    "research",
                    "analysis",
                    "dependencies",
                    "architecture",
                    "test_design",
                    "delivery",
                ] {
                    assert!(
                        tasks.iter().any(|task| task.2["kind"] == kind),
                        "missing feature {kind}"
                    );
                }
                assert!(tasks.iter().all(|task| !matches!(
                    task.2["kind"].as_str(),
                    Some("writing" | "requirements")
                )));
                let connection = rusqlite::Connection::open(&database).unwrap();
                let report: String = connection.query_row(
                    "SELECT report FROM tasks WHERE plan_id=?1 AND json_extract(spec,'$.id')='feature-dependencies'",
                    [&plan_id], |row| row.get(0),
                ).unwrap();
                let report = antislop::lean::content::Content::new(&database)
                    .decode(&report)
                    .unwrap();
                assert!(report.contains("N/A:"), "{report}");
                assert!(tasks.iter().any(|task| task.2["kind"] == "security"));
                assert!(tasks.iter().any(|task| task.2["kind"] == "integration"));
            }
            Team::Refactoring => {
                assert!(has_options);
                assert!(tasks.iter().any(|task| task.2["kind"] == "refactoring"));
                assert!(tasks.iter().any(|task| task.2["kind"] == "integration"));
            }
            Team::SecurityFix => {
                assert!(!has_options);
                assert!(tasks.iter().any(|task| task.2["kind"] == "security"));
                assert!(tasks.iter().any(|task| task.2["kind"] == "integration"));
            }
            Team::CodeReview => {
                assert!(!has_options && !has_quality_gates);
                assert!(tasks.iter().all(|task| task.2["change_kind"] == "none"));
                assert!(tasks.iter().all(|task| {
                    task.2["reviewer"]["summary"]
                        .as_str()
                        .is_some_and(|summary| summary.starts_with("Code review:"))
                }));
            }
            Team::NoQualityGates => {
                assert!(!has_options && !has_quality_gates);
                assert!(tasks.iter().any(|task| task.2["kind"] == "integration"));
                assert!(tasks.iter().all(|task| task.2["options_for"].is_null()));
            }
            Team::Research | Team::Document => {
                assert!(!has_options && !has_quality_gates);
                assert!(tasks.iter().all(|task| task.2["change_kind"] != "code"));
            }
        }
        let completed_gates: Vec<_> = tasks
            .iter()
            .filter(|task| task.2["quality_gate"] == true)
            .filter(|task| task.3 == "approved")
            .collect();
        let expected: BTreeSet<_> = expected_gates(team).iter().copied().collect();
        let actual: BTreeSet<_> = tasks
            .iter()
            .filter(|task| task.2["quality_gate"] == true)
            .filter_map(|task| task.2["kind"].as_str())
            .collect();
        assert_eq!(actual, expected, "finalized gates for {}", team.name());
        let plan_audits: Vec<_> = stored_events(&database, &conversation)
            .into_iter()
            .filter(|event| event["kind"] == "audit" && event["plan"] == plan_id)
            .collect();
        if team == Team::NoQualityGates {
            assert!(
                plan_audits.is_empty(),
                "no-quality-gates skips the plan audit"
            );
        } else {
            assert_eq!(
                plan_audits.len(),
                1,
                "the selected team gets its expected final audit"
            );
            if team == Team::CodeReview {
                assert!(plan_audits[0]["payload"]["report"].is_string());
            }
        }
        if expected.is_empty() {
            assert!(completed_gates.is_empty());
        } else {
            assert_eq!(
                completed_gates.len(),
                tasks
                    .iter()
                    .filter(|task| task.2["quality_gate"] == true)
                    .count()
            );
        }
        assert!(!tasks.is_empty());
        let (mut replayed_app, _) = App::test_app_with_client(daemon.client.clone());
        replayed_app.conversation = conversation.clone();
        replay_all_events(&mut replayed_app, &daemon.client, &conversation);
        assert_eq!(replayed_app.view.rigor_label(), team.name());
        assert_eq!(
            replayed_app.view.tasks.len(),
            tasks.len(),
            "replayed UI task list reflects Core's finalized {} plan",
            team.name()
        );
        assert!(
            replayed_app.view.plan.is_some(),
            "the replayed UI keeps a visible plan for {}",
            team.name()
        );

        // /direct must take the actual Send -> submit_addressed -> Core path;
        // it creates one implementation task and bypasses the selected team's
        // options, generated quality gates and plan review committee.
        app.input = InputBuffer::with_text("/direct Make a direct, contained change.");
        app.submit();
        wait_until("direct composer send result", || {
            drain_updates(&mut app, &updates);
            !app.sending
        });
        wait_until("direct plan materialization", || {
            stored_events(&database, &conversation).iter().any(|event| {
                event["kind"] == "plan_proposed"
                    && event["payload"]["automatic"] == true
                    && event["payload"]["plan"] != plan_id
            })
        });
        let direct_plan_id = stored_events(&database, &conversation)
            .into_iter()
            .rev()
            .find(|event| event["kind"] == "plan_proposed" && event["payload"]["automatic"] == true)
            .unwrap()["payload"]["plan"]
            .as_str()
            .unwrap()
            .to_owned();
        let direct: Vec<_> = stored_tasks(&database, &conversation)
            .into_iter()
            .filter(|task| task.1 == direct_plan_id)
            .collect();
        assert_eq!(
            direct.len(),
            1,
            "direct run is one task for {}",
            team.name()
        );
        assert_eq!(direct[0].2["kind"], "implementation");
        assert_ne!(direct[0].2["quality_gate"], true);
        assert!(direct[0].2["options_for"].is_null());
        let direct_review = stored_events(&database, &conversation);
        assert!(
            direct_review
                .iter()
                .any(|event| event["kind"] == "plan_proposed"
                    && event["payload"]["automatic"] == true
                    && event["payload"]["plan"] == direct_plan_id)
        );
        assert_eq!(
            direct_review
                .iter()
                .filter(|event| {
                    event["kind"] == "plan_review" && event["plan"] == direct_plan_id
                })
                .count(),
            1,
            "/direct records its single synthetic plan acceptance"
        );
        assert!(
            !direct_review.iter().any(|event| {
                event["kind"] == "turn_started"
                    && event["plan"] == direct_plan_id
                    && event["payload"]["role"] == "plan-reviewer"
            }),
            "/direct bypasses the plan review committee"
        );
        wait_until("direct execution completion", || {
            daemon_idle(&daemon.client, &conversation)
                && stored_tasks(&database, &conversation)
                    .iter()
                    .any(|task| task.1 == direct_plan_id && task.3 == "approved")
        });
        let direct_task = stored_tasks(&database, &conversation)
            .into_iter()
            .find(|task| task.1 == direct_plan_id)
            .unwrap();
        assert!(direct_task.2["reviewer"]["summary"].is_string());
        assert!(
            stored_events(&database, &conversation).iter().any(|event| {
                event["kind"] == "turn_finished"
                    && event["plan"] == direct_plan_id
                    && event["payload"]["role"] == format!("reviewer:{}", direct_task.0)
                    && event["payload"]["status"] == "done"
            }),
            "/direct keeps its task reviewer even though committee review is bypassed"
        );
        replay_all_events(&mut app, &daemon.client, &conversation);
        assert_eq!(app.view.rigor_label(), team.name());
        assert!(
            app.view.cards.iter().any(|card| matches!(
                card,
                antislop::lean::tui::TranscriptCard::Plan { plan, tasks: 1, .. }
                    if plan == &direct_plan_id
            )),
            "replayed /direct plan card remains visible in the TUI"
        );
        drop(daemon);
    }
}
