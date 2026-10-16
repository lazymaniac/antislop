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
    // Observation is safe to repeat; scoped mutations below are never passed
    // through this retry path. A socket read timeout must not immediately kill
    // the owned daemon (and its currently running native reviewer).
    read_observed(
        || ipc::read_document(state, id, "run"),
        || {
            thread::sleep(Duration::from_millis(100));
        },
    )
}

fn confirm_guided(app: &mut App, action: &Action) -> Result<()> {
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::NONE,
    )));
    architect::require(
        matches!(&app.view, View::Confirm { action: displayed, .. } if displayed == action),
        "guided confirmation did not retain the exact scoped action",
    )?;
    let effects = app.handle(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    architect::require(
        effects
            .iter()
            .any(|e| matches!(e,Effect::Action(a) if a==action)),
        "confirmation changed the scoped action",
    )
}
fn confirm_selected(app: &mut App, action: &Action) -> Result<()> {
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('p'),
        KeyModifiers::CONTROL,
    )));
    app.palette_index = app
        .palette()
        .iter()
        .position(|(_, offered)| offered.as_ref() == Some(action))
        .ok_or_else(|| {
            architect::Error::Invalid("selected scoped action is not offered by the UI".into())
        })?;
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    architect::require(
        matches!(&app.view, View::Confirm { action: displayed, .. } if displayed == action),
        "selected confirmation did not retain its exact action",
    )?;
    let effects = app.handle(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    architect::require(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Action(a) if a == action)),
        "selected confirmation changed the scoped action",
    )
}
fn ready_snapshot(backend: &mut IpcBackend, id: &str) -> Result<architect::tui::model::Snapshot> {
    loop {
        let snapshot = backend.snapshot(id)?;
        if !snapshot.orchestration_recovery_preparing && !snapshot.task_recovery_preparing {
            return Ok(snapshot);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn unchanged_fixture(
    run: &Run,
    project: &Path,
    baseline: &std::collections::BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    for root in [project, Path::new(&run.spec.workspace)] {
        let mut names = std::collections::BTreeSet::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if entry.file_name() == ".git" {
                continue;
            }
            architect::require(
                entry.file_type()?.is_file(),
                "recovery fixture acquired a directory or link outside its immutable baseline",
            )?;
            let name = entry.file_name().into_string().map_err(|_| {
                architect::Error::Invalid("non-UTF-8 recovery fixture member".into())
            })?;
            architect::require(
                baseline.get(&name).is_some_and(|bytes| {
                    fs::read(entry.path()).is_ok_and(|current| current == *bytes)
                }),
                "recovery fixture membership or original bytes changed",
            )?;
            names.insert(name);
        }
        architect::require(
            names == baseline.keys().cloned().collect(),
            "recovery fixture has missing baseline members",
        )?;
    }
    Ok(())
}
fn read_observed(
    mut download: impl FnMut() -> Result<String>,
    mut wait: impl FnMut(),
) -> Result<Run> {
    let mut transient_failures = 0;
    loop {
        match download() {
            Ok(text) => return architect::transport::decode(text.as_bytes()),
            Err(error) if error.to_string().contains("document changed") => {
                wait();
            }
            Err(architect::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) && transient_failures < 2 =>
            {
                transient_failures += 1;
                eprintln!(
                    "Read-only fixture observation timed out/interrupted; retry {transient_failures}/2; no mutation replay"
                );
                wait();
            }
            Err(error) => return Err(error),
        }
    }
}

#[test]
fn read_only_observation_retries_transient_transport_errors_but_stays_bounded() {
    for kind in [
        std::io::ErrorKind::WouldBlock,
        std::io::ErrorKind::TimedOut,
        std::io::ErrorKind::Interrupted,
    ] {
        let mut attempts = 0;
        let mut waits = 0;
        let result = read_observed(
            || {
                attempts += 1;
                Err(architect::Error::Io(std::io::Error::from(kind)))
            },
            || waits += 1,
        );
        assert!(matches!(result,Err(architect::Error::Io(e)) if e.kind()==kind));
        assert_eq!(attempts, 3);
        assert_eq!(waits, 2);
    }
}

#[test]
fn read_only_observation_does_not_retry_other_errors_or_invalid_data() {
    for kind in [
        std::io::ErrorKind::PermissionDenied,
        std::io::ErrorKind::ConnectionRefused,
    ] {
        let mut attempts = 0;
        assert!(
            read_observed(
                || {
                    attempts += 1;
                    Err(architect::Error::Io(std::io::Error::from(kind)))
                },
                || panic!("must not retry this failure"),
            )
            .is_err()
        );
        assert_eq!(attempts, 1);
    }
    assert!(read_observed(|| Ok("{invalid}".into()), || panic!("invalid bytes")).is_err());
}

#[test]
fn read_only_observation_restarts_changed_download_and_validates_the_final_bytes() {
    let mut attempts = 0;
    let mut waits = 0;
    let result = read_observed(
        || {
            attempts += 1;
            match attempts {
                1 => Err(architect::Error::Invalid(
                    "document changed; restart from offset zero".into(),
                )),
                2 => Err(architect::Error::Io(std::io::Error::from(
                    std::io::ErrorKind::WouldBlock,
                ))),
                _ => Ok("{invalid}".into()),
            }
        },
        || waits += 1,
    );
    assert!(result.is_err(), "invalid final bytes must not become a run");
    assert_eq!(attempts, 3);
    assert_eq!(waits, 2);
}

#[test]
fn read_only_observation_can_return_the_complete_run_after_a_transient_failure() {
    let root = tempfile::tempdir().unwrap();
    let mut spec: architect::domain::RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into_owned();
    let run = Run::new(architect::id(), spec).unwrap();
    let bytes = serde_json::to_string(&run).unwrap();
    let mut attempts = 0;
    let observed = read_observed(
        || {
            attempts += 1;
            if attempts == 1 {
                Err(architect::Error::Io(std::io::Error::from(
                    std::io::ErrorKind::WouldBlock,
                )))
            } else {
                Ok(bytes.clone())
            }
        },
        || {},
    )
    .unwrap();
    assert_eq!(serde_json::to_string(&observed).unwrap(), bytes);
    assert_eq!(attempts, 2);
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
    architect::require(
        std::env::var_os("ANTISLOP_LIVE_VERIFIER_RECOVERY").is_none()
            || (existing.is_some() && std::env::var_os("ANTISLOP_LIVE_TASK_RECOVERY").is_some()),
        "selected verifier recovery requires an explicit retained fixture and task",
    )?;
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
    if let Ok(task_id) = std::env::var("ANTISLOP_LIVE_TASK_RECOVERY") {
        let verifier_only = std::env::var_os("ANTISLOP_LIVE_VERIFIER_RECOVERY").is_some();
        architect::require(
            existing.is_some()
                && std::env::var_os("ANTISLOP_LIVE_RECONCILE").is_none()
                && std::env::var_os("ANTISLOP_LIVE_REVIEW_RETRY").is_none(),
            "task recovery requires an explicit retained fixture without other recovery modes",
        )?;
        let before = read(&state, &id)?;
        unchanged_fixture(&before, &project, &baseline)?;
        let task = before
            .team
            .as_ref()
            .and_then(|team| team.tasks.iter().find(|t| t.id == task_id))
            .ok_or_else(|| architect::Error::Invalid("recovery fixture task is missing".into()))?;
        architect::require(
            before.status == Status::Blocked
                && before.spec.editable_files == vec!["lib.rs"]
                && before.spec.check_command == vec!["python3", "check.py"]
                && before.pending_edits.is_empty()
                && before.check_resources.is_empty()
                && before.task_executions.iter().all(|e| e.receipts.is_empty())
                && before
                    .task_checks
                    .iter()
                    .all(|c| c.state == architect::runtime::team::checks::CheckState::Completed)
                && task.stage == architect::team::TaskStage::Discovery
                && task.state == architect::team::TaskState::Blocked
                && task.artifact.is_some() == verifier_only
                && task.review.is_none()
                && task.write_scopes.is_empty()
                && task
                    .check_plan
                    .iter()
                    .all(|c| c.argv == vec!["python3", "check.py"]),
            "fixture recovery requires the explicitly selected failed read-only discovery operation, unchanged original files and settled checks",
        )?;
        if before.reconciliation_required {
            let mut app = App::default();
            app.update(ready_snapshot(&mut backend, &id)?);
            let (action, label) = app.next_approval().ok_or_else(|| {
                architect::Error::Invalid("no guided checkpoint observation available".into())
            })?;
            architect::require(
                matches!(action, Action::Reconcile { .. }),
                "fixture requires a source-only checkpoint, not effect replay",
            )?;
            confirm_guided(&mut app, &action)?;
            println!("Approve fixture action: {label}");
            backend.act(&id, action)?;
            let after = read(&state, &id)?;
            unchanged_fixture(&after, &project, &baseline)?;
            architect::require(
                serde_json::to_value(&before.artifacts)? == serde_json::to_value(&after.artifacts)?
                    && serde_json::to_value(&before.session_ledger)?
                        == serde_json::to_value(&after.session_ledger)?
                    && serde_json::to_value(&before.team)? == serde_json::to_value(&after.team)?
                    && before.spec_revision == after.spec_revision
                    && before.attempt == after.attempt,
                "checkpoint reconciliation changed retained team, artifacts or native sessions",
            )?;
            println!(
                "Read-only task checkpoint reconciled; retained artifacts/sessions/team and original source/candidate bytes unchanged; no provider work launched"
            );
        }
        if std::env::var_os("ANTISLOP_LIVE_CHECKPOINT_ONLY").is_none() {
            let before_action = read(&state, &id)?;
            let restore_only = std::env::var_os("ANTISLOP_LIVE_RESTORE_ONLY").is_some();
            architect::require(
                !verifier_only || !restore_only,
                "verifier replacement cannot also restore an executor result",
            )?;
            let mut app = App::default();
            app.update(ready_snapshot(&mut backend, &id)?);
            let selected = if verifier_only {
                app.snapshot.task_session_replacements.get(&task_id).map(|package| {
                    (Action::ReplaceTaskSession {task_id:task_id.clone(), scope_hash:package.scope_hash.clone()},
                    "Replace only the selected failed verifier; retain the executor artifact".into())
                })
            } else {
                app.next_approval()
            };
            let (action, label) = selected.ok_or_else(|| {
                architect::Error::Invalid(
                    "no guided interrupted-task continuation available".into(),
                )
            })?;
            architect::require(
                if restore_only {
                    matches!(&action, Action::RestoreTaskResult { task_id: target, .. } if *target == task_id)
                } else {
                    matches!(&action, Action::ReplaceTaskSession { task_id: target, .. } if *target == task_id)
                },
                "fixture recovery must explicitly select only its authorized retained result or distinct read-only session",
            )?;
            if verifier_only {
                confirm_selected(&mut app, &action)?;
            } else {
                confirm_guided(&mut app, &action)?;
            }
            println!("Approve fixture action: {label}");
            let started = std::time::Instant::now();
            backend.act(&id, action)?;
            println!(
                "Guided recovery approval acknowledged in {:.3}s",
                started.elapsed().as_secs_f64()
            );
            if verifier_only {
                loop {
                    let after = read(&state, &id)?;
                    if after.status == Status::Running
                        || architect::recovery::WorkspaceLease::acquire(&after.spec.workspace)
                            .is_err()
                    {
                        thread::sleep(Duration::from_millis(250));
                        continue;
                    }
                    unchanged_fixture(&after, &project, &baseline)?;
                    fs::write(root.join("run.json"), serde_json::to_vec_pretty(&after)?)?;
                    let old_team = before_action.team.as_ref().unwrap();
                    let new_team = after.team.as_ref().unwrap();
                    let target = new_team.tasks.iter().find(|t| t.id == task_id).unwrap();
                    let original_target = old_team.tasks.iter().find(|t| t.id == task_id).unwrap();
                    let old_turns = &before_action.session_ledger.turns;
                    let new_turns: Vec<_> = after
                        .session_ledger
                        .turns
                        .values()
                        .filter(|t| !old_turns.contains_key(&t.id))
                        .collect();
                    architect::require(
                        new_turns.len() == 1
                            && before_action
                                .session_ledger
                                .turns
                                .iter()
                                .all(|(id, old)| after.session_ledger.turns.get(id) == Some(old))
                            && before_action
                                .session_ledger
                                .sessions
                                .iter()
                                .all(|(id, old)| {
                                    after.session_ledger.sessions.get(id) == Some(old)
                                })
                            && after.session_ledger.sessions.len()
                                == before_action.session_ledger.sessions.len() + 1
                            && serde_json::to_value(&before_action.artifacts)?
                                == serde_json::to_value(&after.artifacts)?
                            && serde_json::to_value(&before_action.task_checks)?
                                == serde_json::to_value(&after.task_checks)?
                            && serde_json::to_value(&before_action.quality_gates)?
                                == serde_json::to_value(&after.quality_gates)?
                            && old_team
                                .tasks
                                .iter()
                                .filter(|t| t.id != task_id)
                                .all(|old| {
                                    new_team.tasks.iter().find(|t| t.id == old.id).is_some_and(
                                        |new| {
                                            serde_json::to_value(old).ok()
                                                == serde_json::to_value(new).ok()
                                        },
                                    )
                                })
                            && target.artifact == original_target.artifact
                            && serde_json::to_value(&before_action.task_executions)?
                                == serde_json::to_value(&after.task_executions)?
                            && serde_json::to_value(&before_action.team_result_approvals)?
                                == serde_json::to_value(&after.team_result_approvals)?
                            && after.spec_revision == before_action.spec_revision
                            && target.native_replacements.len()
                                == original_target.native_replacements.len() + 1,
                        "selected verifier replacement changed retained turns, sibling tasks, artifacts, checks or quality gates",
                    )?;
                    let turn = new_turns[0];
                    let session = &after.session_ledger.sessions[&turn.session_id];
                    architect::require(
                        session.scope.task_id.as_deref() == Some(&task_id)
                            && session.scope.role == architect::config::RoleKind::Verifier
                            && session.scope.provider == "agy"
                            && session.scope.binary_version == "1.4.0",
                        "replacement launched another native operation",
                    )?;
                    println!(
                        "Selected native verifier stopped: {:?}; completed={}; model-format rejections={}; review={}; retained original files/artifacts/checks and sibling tasks unchanged",
                        after.status,
                        turn.provider_completed,
                        turn.native_rejections.len(),
                        target.review.is_some()
                    );
                    architect::require(
                        turn.provider_completed && target.review.is_some(),
                        "selected native verifier did not produce an accepted independent review; retained state must not be replayed",
                    )?;
                    return Ok(());
                }
            }
            if restore_only {
                let after = read(&state, &id)?;
                unchanged_fixture(&after, &project, &baseline)?;
                let task = after
                    .team
                    .as_ref()
                    .unwrap()
                    .tasks
                    .iter()
                    .find(|t| t.id == task_id)
                    .unwrap();
                let artifact = after
                    .artifacts
                    .iter()
                    .find(|a| Some(&a.id) == task.artifact.as_ref().map(|r| &r.id))
                    .ok_or_else(|| {
                        architect::Error::Invalid("retained native result was not attached".into())
                    })?;
                let session = &after.session_ledger.sessions[&artifact.session_id];
                let turn = &after.session_ledger.turns[session.current_turn_id.as_ref().unwrap()];
                architect::require(
                    task.state == architect::team::TaskState::AwaitingVerification
                        && task.review.is_none()
                        && turn.output.as_deref() == Some(&artifact.content)
                        && turn.provider_completed
                        && after.artifacts.len() == before_action.artifacts.len() + 1
                        && before_action.artifacts.iter().all(|a| {
                            after
                                .artifacts
                                .iter()
                                .any(|b| a.id == b.id && a.hash == b.hash && a.content == b.content)
                        })
                        && serde_json::to_value(&after.session_ledger)?
                            == serde_json::to_value(&before_action.session_ledger)?
                        && serde_json::to_value(&after.task_checks)?
                            == serde_json::to_value(&before_action.task_checks)?
                        && serde_json::to_value(&after.quality_gates)?
                            == serde_json::to_value(&before_action.quality_gates)?
                        && after.team_result_approvals.len()
                            == before_action.team_result_approvals.len()
                        && after.spec_revision == before_action.spec_revision
                        && after.attempt == before_action.attempt
                        && after
                            .task_executions
                            .iter()
                            .filter(|e| e.task_id == task_id)
                            .all(|e| e.read_only_admission.is_some() && e.receipts.is_empty()),
                    "read-only result admission changed original bytes, native sessions/checks or acceptance state",
                )?;
                println!(
                    "Exact completed native output admitted for independent verification; prior artifacts, all sessions/checks and source/candidate bytes unchanged; no provider work or checks repeated"
                );
            }
        }
    }
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
    if std::env::var_os("ANTISLOP_LIVE_CHECKPOINT_ONLY").is_some()
        || std::env::var_os("ANTISLOP_LIVE_RESTORE_ONLY").is_some()
    {
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
        if snapshot.orchestration_recovery_preparing || snapshot.task_recovery_preparing {
            thread::sleep(Duration::from_millis(100));
            continue;
        }
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
            confirm_guided(&mut app, &action)?;
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
        let admitted_at = std::time::Instant::now();
        backend.act(&id, action)?;
        println!(
            "Fixture approval acknowledged in {:.3}s",
            admitted_at.elapsed().as_secs_f64()
        );
    }
}
#[test]
#[ignore = "Uses authenticated Codex/Antigravity CLI and provider usage; run explicitly"]
fn authenticated_chat_team_and_source_delivery() {
    experiment().unwrap();
}
