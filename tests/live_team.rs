//! Explicit authenticated acceptance experiment. Never runs in the normal suite.
use architect::{
    Result,
    domain::{Run, Status},
    ipc,
    runtime::intake,
    tui::{
        app::{App, Effect, View},
        backend::{Action, Backend, IpcBackend},
    },
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

const PROMPT: &str = "Fix magnitude(value: i32) -> i64 in lib.rs so it returns the nonnegative magnitude for every i32, including i32::MIN; preserve zero and positive behavior and the public signature. Change only lib.rs. Do not change tests.rs, check.py or architect.toml. This disposable repository contains the complete task. Use a managed worktree, supervised approvals, the configured Codex orchestrator and Antigravity executors/verifiers; choose the predefined roles and model/effort routes yourself within those provider limits. Use python3 check.py for original-state investigation and final checks; the original failing tests are the bug reproduction, not an environment failure. The checker also compiles with warnings denied. No external packages, commits, push, publication or external messages are needed or authorized. Read all fixture inputs; ask only if a material product decision remains unresolved.";
struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn git(root: &Path, args: &[&str]) -> Result<()> {
    let o = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=Antislop Fixture",
            "-c",
            "user.email=fixture@localhost",
            "-c",
            "commit.gpgSign=false",
        ])
        .args(args)
        .output()?;
    architect::require(o.status.success(), "fixture Git initialization failed")
}
fn fixture(root: &Path) -> Result<PathBuf> {
    fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    let project = root.join("project");
    fs::create_dir(&project)?;
    fs::write(
        project.join("lib.rs"),
        "pub fn magnitude(value: i32) -> i64 { i64::from(value) }\n",
    )?;
    fs::write(
        project.join("tests.rs"),
        "#[path=\"lib.rs\"] mod subject;\n#[test] fn positive(){assert_eq!(subject::magnitude(7),7)}\n#[test] fn zero(){assert_eq!(subject::magnitude(0),0)}\n#[test] fn negative(){assert_eq!(subject::magnitude(-7),7)}\n#[test] fn minimum(){assert_eq!(subject::magnitude(i32::MIN),2147483648)}\n#[test] fn maximum(){assert_eq!(subject::magnitude(i32::MAX),2147483647)}\n",
    )?;
    fs::write(
        project.join("check.py"),
        "import pathlib, subprocess, tempfile\nrustup=str(pathlib.Path.home()/'.cargo/bin/rustup')\nwith tempfile.TemporaryDirectory(prefix='antislop-check-') as output:\n    binary=str(pathlib.Path(output)/'checks')\n    subprocess.run([rustup,'run','1.98.1','rustc','--test','-D','warnings','tests.rs','-o',binary],check=True)\n    subprocess.run([binary],check=True)\n",
    )?;
    let mut configuration =
        include_str!("../architect.toml").replacen("mode = \"in-place\"", "mode = \"worktree\"", 1);
    if std::env::var_os("ANTISLOP_LIVE_PINNED_CONTEXT").is_some() {
        let mut selected = false;
        configuration = configuration
            .lines()
            .map(|line| {
                if line.trim_start().starts_with('[') {
                    selected = matches!(line.trim(), "[providers.codex]" | "[providers.agy]");
                }
                if selected && line.trim_start().starts_with("isolation =") {
                    "isolation = \"pinned_context\"".to_owned()
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let config = architect::config::Config::from_toml(&configuration)?;
        for provider in [
            architect::config::ProviderKind::Codex,
            architect::config::ProviderKind::Agy,
        ] {
            architect::require(
                config.providers.get(&provider).is_some_and(|p| {
                    p.isolation == architect::config::IsolationProfile::PinnedContext
                }),
                "fixture requires explicitly configured Codex and Antigravity reader profiles",
            )?;
        }
    }
    fs::write(project.join("architect.toml"), configuration)?;
    git(&project, &["init", "-q"])?;
    git(&project, &["add", "."])?;
    git(&project, &["commit", "-qm", "Original failing fixture"])?;
    let baseline = ["lib.rs", "tests.rs", "check.py", "architect.toml"]
        .into_iter()
        .map(|name| Ok((name.to_owned(), fs::read(project.join(name))?)))
        .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
    fs::write(
        root.join("fixture-baseline.json"),
        serde_json::to_vec(&baseline)?,
    )?;
    let before = Command::new("python3")
        .arg("check.py")
        .current_dir(&project)
        .output()?;
    architect::require(
        !before.status.success(),
        "fixture must reproduce the original bug",
    )?;
    fs::write(
        root.join("original-check.log"),
        before
            .stdout
            .into_iter()
            .chain(before.stderr)
            .collect::<Vec<_>>(),
    )?;
    Ok(project)
}
fn read(state: &Path, id: &str) -> Result<Run> {
    loop {
        match ipc::read_document(state, id, "run") {
            Ok(text) => return architect::transport::decode(text.as_bytes()),
            Err(error) if error.to_string().contains("document changed") => {
                thread::sleep(Duration::from_millis(25))
            }
            Err(error) => return Err(error),
        }
    }
}
fn experiment() -> Result<()> {
    if let Some(path) = std::env::var_os("ANTISLOP_LIVE_INSPECT_RUN") {
        let mut run: Run = serde_json::from_slice(&fs::read(path)?)?;
        if std::env::var_os("ANTISLOP_LIVE_INSPECT_INLINE").is_some() {
            let round = run
                .orchestration_rounds
                .last_mut()
                .ok_or_else(|| architect::Error::Invalid("no retained proposal".into()))?;
            let mut context: serde_json::Value = serde_json::from_str(&round.request_context)?;
            context["review_input_format"] = serde_json::json!("inline-exact-utf8-v7");
            round.request_context = serde_json::to_string(&context)?;
            println!("Hypothetical new input-format measurement only; retained run is unchanged.");
        }
        let round = run
            .orchestration_rounds
            .last()
            .ok_or_else(|| architect::Error::Invalid("no retained proposal".into()))?;
        let response = serde_json::from_str(round.output.as_deref().unwrap_or_default())?;
        match architect::orchestration::planning::validate_response(&run, &response) {
            Ok(team) => {
                println!("Current proposal validation: {}", team.is_some());
                println!(
                    "Planned native review parts: {}",
                    architect::orchestration::planning::parts::plan(&run, round, &response)?.len()
                );
                for (index, part) in
                    architect::orchestration::planning::parts::plan(&run, round, &response)?
                        .into_iter()
                        .enumerate()
                {
                    let request = architect::orchestration::planning::parts::request(
                        &run, round, &response, &part,
                    )?;
                    if index == 0
                        && let Some(destination) = std::env::var_os("ANTISLOP_LIVE_INSPECT_PROMPT")
                    {
                        use std::io::Write;
                        use std::os::unix::fs::OpenOptionsExt;
                        fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .mode(0o600)
                            .open(destination)?
                            .write_all(request.prompt.as_bytes())?;
                    }
                    println!(
                        "Review part {}: {} prompt bytes",
                        part.id,
                        request.prompt.len()
                    );
                }
            }
            Err(error) => println!("Current proposal validation: {error}"),
        }
        return Ok(());
    }
    let existing = std::env::var_os("ANTISLOP_LIVE_ROOT").map(PathBuf::from);
    let root = existing.clone().unwrap_or_else(|| {
        tempfile::Builder::new()
            .prefix("antislop-live-team-")
            .tempdir()
            .unwrap()
            .keep()
    });
    println!("Retained live experiment: {}", root.display());
    let project = if existing.is_some() {
        root.join("project")
    } else {
        fixture(&root)?
    };
    if existing.is_some() && std::env::var_os("ANTISLOP_LIVE_PINNED_CONTEXT").is_some() {
        let config = architect::config::Config::from_toml(&fs::read_to_string(
            project.join("architect.toml"),
        )?)?;
        architect::require(
            [
                architect::config::ProviderKind::Codex,
                architect::config::ProviderKind::Agy,
            ]
            .iter()
            .all(|p| {
                config.providers.get(p).is_some_and(|c| {
                    c.isolation == architect::config::IsolationProfile::PinnedContext
                })
            }),
            "the reader fixture flag never upgrades a retained experiment; create a fresh fixture or keep its original reviewed profile",
        )?;
    }
    let baseline: std::collections::BTreeMap<String, Vec<u8>> =
        serde_json::from_slice(&fs::read(root.join("fixture-baseline.json"))?)?;
    let original = baseline
        .get("lib.rs")
        .ok_or_else(|| architect::Error::Invalid("missing immutable fixture baseline".into()))?
        .clone();
    let protected = ["tests.rs", "check.py", "architect.toml"]
        .into_iter()
        .map(|name| {
            Ok((
                name,
                baseline
                    .get(name)
                    .ok_or_else(|| {
                        architect::Error::Invalid("missing protected fixture baseline".into())
                    })?
                    .clone(),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let state = root.join("state");
    fs::create_dir_all(&state)?;
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700))?;
    let mut daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("daemon")
            .arg(&state)
            .stdout(Stdio::null())
            .stderr(Stdio::from(fs::File::create(root.join("daemon.log"))?))
            .spawn()?,
    );
    let mut backend = IpcBackend::new(&state);
    loop {
        architect::require(
            daemon.0.try_wait()?.is_none(),
            "fixture daemon stopped during startup; inspect retained daemon.log",
        )?;
        if backend.list_runs().is_ok() {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let id = if existing.is_some() {
        fs::read_to_string(root.join("run-id"))?
    } else {
        let started = backend.begin_conversation_in(PROMPT, &project)?;
        fs::write(root.join("run-id"), &started.run_id)?;
        architect::require(started.message_accepted, &started.notice)?;
        started.run_id
    };
    if std::env::var_os("ANTISLOP_LIVE_RECONCILE").is_some() {
        let mut run = read(&state, &id)?;
        if run.status == Status::Paused
            && run.paused_from == Some(Status::Blocked)
            && run.team.is_none()
        {
            backend.act(&id, Action::Resume)?;
            run = read(&state, &id)?;
        }
        architect::require(
            run.team.is_none() && run.artifacts.is_empty() && run.status == Status::Blocked,
            "fixture reconciliation is restricted to interrupted pre-execution management",
        )?;
        for (name, bytes) in &baseline {
            architect::require(
                fs::read(project.join(name))? == *bytes,
                "fixture source changed before management reconciliation",
            )?;
            architect::require(
                fs::read(Path::new(&run.spec.workspace).join(name))? == *bytes,
                "fixture candidate changed before management reconciliation",
            )?;
        }
        let snapshot = backend.snapshot(&id)?;
        backend.act(
            &id,
            Action::Reconcile {
                scope_hash: snapshot
                    .scope_hashes
                    .get("reconcile")
                    .cloned()
                    .ok_or_else(|| {
                        architect::Error::Invalid("missing reconciliation scope".into())
                    })?,
                input_hash: snapshot
                    .reconciliation_input_hash
                    .ok_or_else(|| architect::Error::Invalid("missing observed inputs".into()))?,
            },
        )?;
    }
    if std::env::var_os("ANTISLOP_LIVE_CHECKPOINT_ONLY").is_some() {
        let run = read(&state, &id)?;
        fs::write(root.join("run.json"), serde_json::to_vec_pretty(&run)?)?;
        println!(
            "Observed checkpoint only: {:?}; reconciliation_required={}; no provider work launched",
            run.status, run.reconciliation_required
        );
        return Ok(());
    }
    if std::env::var_os("ANTISLOP_LIVE_REVIEW_RETRY").is_some() {
        let started = std::time::Instant::now();
        let snapshot = loop {
            let snapshot = backend.snapshot(&id)?;
            if !snapshot.orchestration_recovery_preparing {
                break snapshot;
            }
            if started.elapsed() >= Duration::from_secs(120) {
                return Err(architect::Error::Invalid(
                    "recovery validation still running; no native replacement was requested".into(),
                ));
            }
            thread::sleep(Duration::from_millis(50));
        };
        let package=snapshot.orchestration_recoveries.values().find(|p|p.role==architect::config::RoleKind::Verifier && p.kind==architect::runtime::orchestration_recovery::Kind::Replacement).ok_or_else(||architect::Error::Invalid("no eligible incomplete-review replacement; do not bypass substantive rejection".into()))?;
        backend.act(
            &id,
            Action::RecoverOrchestration {
                round_id: package.round_id.clone(),
                kind: package.kind,
                scope_hash: package.scope_hash.clone(),
            },
        )?;
    }
    if let Ok(answer) = std::env::var("ANTISLOP_LIVE_REPLY") {
        backend.act(&id, Action::ChatMessage { text: answer })?;
    }
    let mut prior = String::new();
    loop {
        let run = read(&state, &id)?;
        if run.status == Status::Running
            || architect::recovery::WorkspaceLease::acquire(&run.spec.workspace).is_err()
        {
            thread::sleep(Duration::from_millis(250));
            continue;
        }
        fs::write(root.join("run.json"), serde_json::to_vec_pretty(&run)?)?;
        let phase = format!(
            "{:?} rev={} attempt={} {:?}",
            run.status, run.spec_revision, run.attempt, run.diagnostic
        );
        if phase != prior {
            println!("{phase}");
            prior = phase;
        }
        architect::require(
            !run.reconciliation_required,
            "native experiment requires explicit reconciliation; retained state must not be replayed",
        )?;
        if run.intake
            && let Some((_, _)) = intake::pending(&run)
        {
            let round = run.orchestration_rounds.last().unwrap();
            let response: architect::orchestration::planning::Response =
                architect::transport::decode(round.output.as_ref().unwrap().as_bytes())?;
            let proposed =
                intake::proposed_spec(&run, response.intake_proposal_json.as_ref().unwrap())?;
            architect::require(
                proposed.editable_files == vec!["lib.rs"]
                    && proposed.check_command == vec!["python3", "check.py"],
                "live proposal exceeds the fixed fixture's edit/check authorization",
            )?;
        }
        for (name, bytes) in &protected {
            architect::require(
                fs::read(project.join(name))? == *bytes,
                "fixture changed a protected source file",
            )?;
        }
        if run
            .delivery
            .as_ref()
            .is_some_and(|d| d.state == architect::runtime::delivery::State::Delivered)
        {
            let checked = Command::new("python3")
                .arg("check.py")
                .current_dir(&project)
                .output()?;
            fs::write(
                root.join("delivered-check.log"),
                checked
                    .stdout
                    .into_iter()
                    .chain(checked.stderr)
                    .collect::<Vec<_>>(),
            )?;
            architect::require(
                checked.status.success(),
                "delivered result fails the immutable fixture checks",
            )?;
            println!("Completed authenticated fixture: {}", root.display());
            return Ok(());
        }
        architect::require(
            fs::read(project.join("lib.rs"))? == original,
            "original checkout changed before separately approved delivery",
        )?;
        let snapshot = backend.snapshot(&id)?;
        let mut app = App::default();
        app.update(snapshot);
        let next = app.next_approval().or_else(|| {
            run.team.as_ref().and_then(|team| {
                team.tasks.iter().find_map(|t| {
                    run.task_source_refresh_scope(&t.id).ok().map(|scope_hash| {
                        (
                            Action::RefreshTaskSources {
                                task_id: t.id.clone(),
                                scope_hash,
                            },
                            "Refresh exact stale task sources".into(),
                        )
                    })
                })
            })
        });
        let Some((action, label)) = next else {
            return Err(architect::Error::Invalid(format!(
                "No approved next step. Inspect {}/run.json; latest round: {:?}",
                root.display(),
                run.orchestration_rounds
                    .last()
                    .map(|r| (&r.status, &r.diagnostic))
            )));
        };
        // Freeze the same UI confirmation used by the user for guided actions.
        if app.next_approval().is_some() {
            app.handle(Event::Key(KeyEvent::new(
                KeyCode::Char('a'),
                KeyModifiers::NONE,
            )));
            architect::require(
                matches!(app.view, View::Confirm { .. }),
                "guided action did not show its approval card",
            )?;
            let effects = app.handle(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )));
            architect::require(
                effects
                    .iter()
                    .any(|e| matches!(e,Effect::Action(a) if *a==action)),
                "confirmation changed the scoped action",
            )?;
        }
        if matches!(action, Action::StepTeam { .. }) {
            let wave = app
                .snapshot
                .pending_wave
                .as_ref()
                .ok_or_else(|| architect::Error::Invalid("fixture wave missing".into()))?;
            architect::require(
                wave.actions.iter().all(|a| {
                    a.checks
                        .iter()
                        .all(|c| c.argv == vec!["python3", "check.py"])
                        && a.write_scopes.iter().all(|p| p == "lib.rs")
                }),
                "fixture wave exceeds its fixed command/file authorization",
            )?;
        }
        if matches!(action, Action::DeliverSource { .. }) {
            let plan = architect::runtime::delivery::pending(&run)?;
            architect::require(
                plan.changes.len() == 1 && plan.changes[0].path == "lib.rs",
                "fixture delivery contains unapproved changes",
            )?;
        }
        println!("Approve fixture action: {label}");
        backend.act(&id, action)?;
    }
}
#[test]
#[ignore = "Uses authenticated Codex/Antigravity CLI and provider usage; run explicitly"]
fn authenticated_chat_team_and_source_delivery() {
    experiment().unwrap();
}
