use architect::{
    Result,
    ipc::{self, Operation, Request},
    runtime::Command,
};
use std::{io::Read, path::PathBuf};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let verb = args.get(1).map(String::as_str).unwrap_or("help");
    if verb == "--version" {
        println!("antislop {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if verb == "__supervise" {
        return architect::process::supervise(&args[2..]);
    }
    if verb == "ui" {
        return architect::tui::run_options(architect::tui::options::Options::parse(&args[2..])?);
    }
    if verb == "workspace-snapshot" {
        architect::require(
            args.len() >= 3,
            "workspace-snapshot requires DIRECTORY [ADDITIONAL_PATH ...]",
        )?;
        let paths = args[3..]
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let snapshot = architect::workspace::snapshot::observe(
            std::path::Path::new(&args[2]),
            &paths,
            &std::sync::atomic::AtomicBool::new(false),
        )?;
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        return Ok(());
    }
    let state = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".antislop"));
    if verb == "daemon" {
        return ipc::serve(&state);
    }
    if matches!(
        verb,
        "orchestrate"
            | "resume-orchestration"
            | "replace-orchestration"
            | "restore-orchestration"
            | "accept-intake"
            | "accept-team"
            | "step-team"
            | "accept-team-stage"
            | "repair-task"
            | "retry-task-verification"
            | "resume-task-session"
            | "replace-task-session"
            | "reconcile-wave-effects"
            | "reconcile-task-effects"
            | "restore-task-result"
            | "restore-task-check-result"
            | "refresh-task-sources"
    ) {
        let flag = match verb {
            "orchestrate" | "step-team" => 4,
            "accept-intake"
            | "resume-orchestration"
            | "replace-orchestration"
            | "restore-orchestration"
            | "accept-team"
            | "accept-team-stage"
            | "retry-task-verification"
            | "resume-task-session"
            | "replace-task-session"
            | "reconcile-wave-effects"
            | "reconcile-task-effects"
            | "restore-task-result"
            | "restore-task-check-result"
            | "refresh-task-sources" => 5,
            _ => 6,
        };
        architect::require(
            args.len() == flag + 1
                && args.get(flag).map(String::as_str) == Some("--approve-scoped"),
            "team mutation commands require exact arguments and --approve-scoped",
        )?;
        let run: architect::domain::Run = architect::transport::decode(
            ipc::read_document(&state, arg(&args, 3)?, "run")?.as_bytes(),
        )?;
        architect::require(run.id == arg(&args, 3)?, "mismatched run document")?;
        let command = match verb {
            "orchestrate" => Command::Orchestrate {
                run_id: run.id.clone(),
                scope_hash: architect::orchestration::planning::start_scope(&run)?,
            },
            "resume-orchestration" | "replace-orchestration" | "restore-orchestration" => {
                use architect::runtime::orchestration_recovery::{Kind, for_round};
                let round_id = arg(&args, 4)?;
                let kind = match verb {
                    "resume-orchestration" => Kind::Resume,
                    "replace-orchestration" => Kind::Replacement,
                    _ => Kind::Restore,
                };
                let package = for_round(&run, round_id, kind)?;
                Command::RecoverOrchestration {
                    run_id: run.id.clone(),
                    round_id: round_id.into(),
                    kind,
                    scope_hash: package.scope_hash,
                }
            }
            "accept-intake" => {
                let (round_id, scope_hash) =
                    architect::runtime::intake::pending(&run).ok_or_else(|| {
                        architect::Error::Invalid("no current reviewed intake proposal".into())
                    })?;
                architect::require(round_id == arg(&args, 4)?, "intake round is stale")?;
                Command::AcceptIntake {
                    run_id: run.id.clone(),
                    round_id,
                    scope_hash,
                }
            }
            "accept-team-stage" => {
                let key = arg(&args, 4)?;
                let pending = architect::runtime::team::stage::pending(&run);
                let package = pending.get(key).ok_or_else(|| {
                    architect::Error::Invalid("no current stage result package".into())
                })?;
                Command::AcceptTeamStage {
                    run_id: run.id.clone(),
                    stage: package.stage,
                    task_ids: package.task_ids.clone(),
                    scope_hash: package.scope_hash.clone(),
                }
            }
            "step-team" => Command::StepTeam {
                run_id: run.id.clone(),
                scope_hash: run.team_step_scope()?,
            },
            "reconcile-wave-effects" => {
                let waves = architect::runtime::team::adjudicate::pending_waves(&run);
                let package = waves.get(arg(&args, 4)?).ok_or_else(|| {
                    architect::Error::Invalid("no current observed wave effect package".into())
                })?;
                Command::ReconcileWaveEffects {
                    run_id: run.id.clone(),
                    task_ids: package.task_ids.clone(),
                    scope_hash: package.scope_hash.clone(),
                }
            }
            "reconcile-task-effects" => {
                let task_id = arg(&args, 4)?;
                Command::ReconcileTaskEffects {
                    run_id: run.id.clone(),
                    task_id: task_id.into(),
                    scope_hash: run.task_effect_reconciliation_scope(task_id)?,
                }
            }
            "restore-task-result" => {
                let task_id = arg(&args, 4)?;
                Command::RestoreTaskResult {
                    run_id: run.id.clone(),
                    task_id: task_id.into(),
                    scope_hash: run.task_result_restore_scope(task_id)?,
                }
            }
            "restore-task-check-result" => {
                let invocation_id = arg(&args, 4)?;
                let p =
                    architect::runtime::team::check_recovery::for_invocation(&run, invocation_id)?;
                Command::RestoreTaskCheckResult {
                    run_id: run.id.clone(),
                    invocation_id: invocation_id.into(),
                    scope_hash: p.scope_hash,
                }
            }
            "refresh-task-sources" => {
                let task_id = arg(&args, 4)?;
                Command::RefreshTaskSources {
                    run_id: run.id.clone(),
                    task_id: task_id.to_string(),
                    scope_hash: run.task_source_refresh_scope(task_id)?,
                }
            }
            "retry-task-verification" => {
                let task_id = arg(&args, 4)?;
                Command::RetryTaskVerification {
                    run_id: run.id.clone(),
                    task_id: task_id.to_string(),
                    scope_hash: run.task_verification_retry_scope(task_id)?,
                }
            }
            "resume-task-session" => {
                let task_id = arg(&args, 4)?;
                let package = architect::runtime::team::resume::for_task(&run, task_id)?;
                Command::ResumeTaskSession {
                    run_id: run.id.clone(),
                    task_id: task_id.into(),
                    scope_hash: package.scope_hash,
                }
            }
            "replace-task-session" => {
                let task_id = arg(&args, 4)?;
                let package = architect::runtime::team::replacement::for_task(&run, task_id)?;
                Command::ReplaceTaskSession {
                    run_id: run.id.clone(),
                    task_id: task_id.into(),
                    scope_hash: package.scope_hash,
                }
            }
            "repair-task" => {
                let task_id = arg(&args, 4)?;
                let plan = arg(&args, 5)?;
                Command::RepairTask {
                    run_id: run.id.clone(),
                    task_id: task_id.to_string(),
                    plan: plan.to_string(),
                    scope_hash: run.team_repair_scope(task_id, plan)?,
                }
            }
            _ => {
                let round = run
                    .orchestration_rounds
                    .iter()
                    .find(|r| r.id == arg(&args, 4).unwrap_or_default())
                    .ok_or_else(|| {
                        architect::Error::Invalid("unknown orchestration round".into())
                    })?;
                Command::AcceptTeam {
                    run_id: run.id.clone(),
                    round_id: round.id.clone(),
                    scope_hash: architect::orchestration::planning::accept_scope(&run, round)?,
                }
            }
        };
        let response = ipc::call(
            &state,
            &Request {
                version: ipc::PROTOCOL_VERSION,
                request_id: architect::id(),
                idempotency_key: Some(architect::id()),
                operation: Operation::Mutate {
                    command: Box::new(command),
                },
            },
        )?;
        println!("{}", serde_json::to_string_pretty(&response)?);
        if let Some(error) = response.error {
            return Err(architect::Error::Invalid(error));
        }
        return Ok(());
    }
    if verb == "run" {
        architect::require(
            args.get(4).map(String::as_str) == Some("--approve-scoped"),
            "run requires --approve-scoped: authorize this spec's files, local check argv and stage approvals",
        )?;
        let repairs = args
            .get(5)
            .map(|v| v.parse::<usize>())
            .transpose()
            .map_err(|_| architect::Error::Invalid("invalid repair limit".into()))?
            .unwrap_or(0);
        let spec = architect::transport::decode(&std::fs::read(arg(&args, 3)?)?)?;
        let run = architect::drive::run(&state, spec, repairs, timeout(&args, 6)?)?;
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"id":run.id,"status":run.status,"stage":run.stage})
            )?
        );
        return Ok(());
    }
    if verb == "follow" {
        architect::require(
            args.get(5).map(String::as_str) == Some("--approve-scoped"),
            "follow requires --approve-scoped for the original immutable spec",
        )?;
        let repairs = args
            .get(6)
            .map(|v| v.parse::<usize>())
            .transpose()
            .map_err(|_| architect::Error::Invalid("invalid repair limit".into()))?
            .unwrap_or(0);
        let spec = architect::transport::decode(&std::fs::read(arg(&args, 4)?)?)?;
        let run =
            architect::drive::follow(&state, arg(&args, 3)?, spec, repairs, timeout(&args, 7)?)?;
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({"id":run.id,"status":run.status,"stage":run.stage})
            )?
        );
        return Ok(());
    }
    if verb == "verify" {
        architect::require(
            args.len() == 5 && args.get(4).map(String::as_str) == Some("--approve-scoped"),
            "verify requires STATE RUN_ID --approve-scoped: authorize a fresh review of the existing artifact",
        )?;
        let run: architect::domain::Run = architect::transport::decode(
            ipc::read_document(&state, arg(&args, 3)?, "run")?.as_bytes(),
        )?;
        architect::require(run.id == arg(&args, 3)?, "mismatched run document")?;
        let artifact = run
            .artifacts
            .iter()
            .rev()
            .find(|artifact| {
                artifact.stage == run.stage
                    && artifact.spec_revision == run.spec_revision
                    && artifact.attempt == run.attempt
            })
            .ok_or_else(|| architect::Error::Invalid("current artifact is missing".into()))?;
        let scope_hash = run.scope_hash(&format!("verify:{}", artifact.hash));
        let command = Command::RetryVerification {
            run_id: run.id.clone(),
            artifact_hash: artifact.hash.clone(),
            scope_hash,
        };
        let response = ipc::call(
            &state,
            &Request {
                version: ipc::PROTOCOL_VERSION,
                request_id: architect::id(),
                idempotency_key: Some(architect::id()),
                operation: Operation::Mutate {
                    command: Box::new(command),
                },
            },
        )?;
        println!("{}", serde_json::to_string_pretty(&response)?);
        if let Some(error) = response.error {
            return Err(architect::Error::Invalid(error));
        }
        return Ok(());
    }
    if verb == "check" {
        architect::require(
            args.len() == 6 && args.get(5).map(String::as_str) == Some("--approve-scoped"),
            "check requires STATE RUN_ID ARGV.json --approve-scoped: authorize exactly this local argv",
        )?;
        let mut bytes = Vec::new();
        std::fs::File::open(arg(&args, 4)?)?
            .take((architect::transport::MAX_FRAME + 1) as u64)
            .read_to_end(&mut bytes)?;
        architect::require(
            bytes.len() <= architect::transport::MAX_FRAME,
            "check argv exceeds transport limit",
        )?;
        let argv: Vec<String> = architect::transport::decode(&bytes)?;
        architect::domain::Run::check_action(&argv)?;
        let run: architect::domain::Run = architect::transport::decode(
            ipc::read_document(&state, arg(&args, 3)?, "run")?.as_bytes(),
        )?;
        architect::require(run.id == arg(&args, 3)?, "mismatched run document")?;
        let scope_hash = run.check_scope_hash(&argv)?;
        let response = ipc::call(
            &state,
            &Request {
                version: ipc::PROTOCOL_VERSION,
                request_id: architect::id(),
                idempotency_key: Some(architect::id()),
                operation: Operation::Mutate {
                    command: Box::new(Command::Check {
                        run_id: run.id,
                        argv,
                        scope_hash,
                    }),
                },
            },
        )?;
        println!("{}", serde_json::to_string_pretty(&response)?);
        if let Some(error) = response.error {
            return Err(architect::Error::Invalid(error));
        }
        return Ok(());
    }
    if verb == "diff" {
        let run_id = arg(&args, 3)?;
        let text = ipc::read_document(&state, run_id, "diff")?;
        let diff: architect::workspace::DiffReport = serde_json::from_str(&text)?;
        let run_text = ipc::read_document(&state, run_id, "run")?;
        let run: architect::domain::Run = architect::transport::decode(run_text.as_bytes())?;
        println!("Diff report for run {run_id}:");
        println!(
            "Total files changed: {}, +{} / -{} lines",
            diff.files.len(),
            diff.total_additions,
            diff.total_deletions
        );
        for file in &diff.files {
            println!(
                "\n--- {} [{:?}] (+{} / -{}) ---",
                file.path, file.status, file.additions, file.deletions
            );
            let file_comments: Vec<_> = run
                .diff_comments
                .iter()
                .filter(|c| c.path == file.path)
                .collect();
            for comment in file_comments.iter().filter(|c| c.line_number.is_none()) {
                println!(
                    "  [Comment by {} (resolved: {})]: {}",
                    comment.author, comment.resolved, comment.text
                );
            }
            for hunk in &file.hunks {
                println!("{}", hunk.header);
                for line in &hunk.lines {
                    let prefix = match line.kind {
                        architect::workspace::DiffLineKind::Addition => "+",
                        architect::workspace::DiffLineKind::Deletion => "-",
                        architect::workspace::DiffLineKind::Context => " ",
                    };
                    let line_no = line.new_line_number.or(line.old_line_number).unwrap_or(0);
                    println!("{prefix} {:4} | {}", line_no, line.content);
                    if let Some(nl) = line.new_line_number {
                        for comment in file_comments.iter().filter(|c| c.line_number == Some(nl)) {
                            println!("    >> [Comment by {}]: {}", comment.author, comment.text);
                        }
                    }
                }
            }
        }
        return Ok(());
    }
    if verb == "comment" {
        let run_id = arg(&args, 3)?;
        let path = arg(&args, 4)?;
        let (line_number, text) =
            if args.len() >= 7 && args.get(5).map(String::as_str) == Some("--line") {
                let line: usize = args[6]
                    .parse()
                    .map_err(|_| architect::Error::Invalid("invalid line number".into()))?;
                (Some(line), arg(&args, 7)?)
            } else {
                (None, arg(&args, 5)?)
            };
        let command = Command::AddDiffComment {
            run_id: run_id.to_string(),
            path: path.to_string(),
            line_number,
            text: text.to_string(),
            author: "user".to_string(),
        };
        let response = ipc::call(
            &state,
            &Request {
                version: ipc::PROTOCOL_VERSION,
                request_id: architect::id(),
                idempotency_key: Some(architect::id()),
                operation: Operation::Mutate {
                    command: Box::new(command),
                },
            },
        )?;
        println!("{}", serde_json::to_string_pretty(&response)?);
        if let Some(error) = response.error {
            return Err(architect::Error::Invalid(error));
        }
        return Ok(());
    }
    if verb == "delete-comment" {
        let run_id = arg(&args, 3)?;
        let comment_id = arg(&args, 4)?;
        let command = Command::DeleteDiffComment {
            run_id: run_id.to_string(),
            comment_id: comment_id.to_string(),
        };
        let response = ipc::call(
            &state,
            &Request {
                version: ipc::PROTOCOL_VERSION,
                request_id: architect::id(),
                idempotency_key: Some(architect::id()),
                operation: Operation::Mutate {
                    command: Box::new(command),
                },
            },
        )?;
        println!("{}", serde_json::to_string_pretty(&response)?);
        if let Some(error) = response.error {
            return Err(architect::Error::Invalid(error));
        }
        return Ok(());
    }
    if verb == "review-diff" {
        architect::require(
            args.len() == 5 && args.get(4).map(String::as_str) == Some("--approve-scoped"),
            "review-diff requires STATE RUN_ID --approve-scoped",
        )?;
        let run: architect::domain::Run = architect::transport::decode(
            ipc::read_document(&state, arg(&args, 3)?, "run")?.as_bytes(),
        )?;
        architect::require(run.id == arg(&args, 3)?, "mismatched run document")?;
        let scope_hash = run.diff_review_scope()?;
        let command = Command::ReviewDiff {
            run_id: run.id.clone(),
            scope_hash,
        };
        let response = ipc::call(
            &state,
            &Request {
                version: ipc::PROTOCOL_VERSION,
                request_id: architect::id(),
                idempotency_key: Some(architect::id()),
                operation: Operation::Mutate {
                    command: Box::new(command),
                },
            },
        )?;
        println!("{}", serde_json::to_string_pretty(&response)?);
        if let Some(error) = response.error {
            return Err(architect::Error::Invalid(error));
        }
        return Ok(());
    }
    if verb == "help" || verb == "--help" {
        println!(
            "antislop resume-orchestration STATE_DIR RUN_ID ROUND_ID --approve-scoped\nantislop replace-orchestration STATE_DIR RUN_ID ROUND_ID --approve-scoped\nantislop restore-orchestration STATE_DIR RUN_ID ROUND_ID --approve-scoped\nantislop orchestrate STATE_DIR RUN_ID --approve-scoped\nantislop accept-intake STATE_DIR RUN_ID ROUND_ID --approve-scoped\nantislop accept-team STATE_DIR RUN_ID ROUND_ID --approve-scoped\nantislop step-team STATE_DIR RUN_ID --approve-scoped\nantislop accept-team-stage STATE_DIR RUN_ID STAGE --approve-scoped\nantislop repair-task STATE_DIR RUN_ID TASK_ID PLAN --approve-scoped\nantislop replace-task-session STATE_DIR RUN_ID TASK_ID --approve-scoped\nantislop resume-task-session STATE_DIR RUN_ID TASK_ID --approve-scoped\nantislop retry-task-verification STATE_DIR RUN_ID TASK_ID --approve-scoped\nantislop refresh-task-sources STATE_DIR RUN_ID TASK_ID --approve-scoped\nantislop restore-task-check-result STATE_DIR RUN_ID INVOCATION_ID --approve-scoped\nantislop restore-task-result STATE_DIR RUN_ID TASK_ID --approve-scoped\nantislop reconcile-task-effects STATE_DIR RUN_ID TASK_ID --approve-scoped\nantislop reconcile-wave-effects STATE_DIR RUN_ID WAVE_ATTEMPT --approve-scoped\nantislop diff STATE_DIR RUN_ID\nantislop comment STATE_DIR RUN_ID PATH [--line N] TEXT\nantislop delete-comment STATE_DIR RUN_ID COMMENT_ID\nantislop review-diff STATE_DIR RUN_ID --approve-scoped\n"
        );
        println!(
            "antislop local prototype\n\nantislop daemon STATE_DIR\nantislop ui [STATE_DIR] [RUN_ID] [--orchestrator codex|claude|gemini|agy]\nantislop run STATE_DIR SPEC.json --approve-scoped [MAX_REPAIRS] [TIMEOUT_SECONDS]\nantislop follow STATE_DIR RUN_ID SPEC.json --approve-scoped [MAX_REPAIRS] [TIMEOUT_SECONDS]\nantislop check STATE_DIR RUN_ID ARGV.json --approve-scoped\nantislop verify STATE_DIR RUN_ID --approve-scoped\nantislop request STATE_DIR < request.json\nantislop create STATE_DIR SPEC.json\nantislop snapshot STATE_DIR RUN_ID\nantislop workspace-snapshot DIRECTORY [ADDITIONAL_PATH ...]\nantislop list STATE_DIR\nantislop export STATE_DIR RUN_ID\nantislop events STATE_DIR CURSOR\n\nrun/follow delegate approvals only for the immutable supplied spec and its local check argv.\nMAX_REPAIRS is the total persisted repair budget; time is unlimited unless explicitly bounded.\nOther mutations require explicit idempotency keys via request. Stage approvals use scope_hashes from snapshot."
        );
        return Ok(());
    }
    if verb == "export" {
        let document = args.get(4).map(String::as_str).unwrap_or("run");
        let text = ipc::read_document(&state, arg(&args, 3)?, document)?;
        println!("{text}");
        return Ok(());
    }
    let request = match verb {
        "request" => {
            let mut input = String::new();
            std::io::stdin()
                .take(4 * 1024 * 1024 + 1)
                .read_to_string(&mut input)?;
            architect::require(
                input.len() <= 4 * 1024 * 1024,
                "request exceeds transport limit",
            )?;
            architect::transport::decode(input.as_bytes())?
        }
        "create" => Request {
            version: ipc::PROTOCOL_VERSION,
            request_id: architect::id(),
            idempotency_key: Some(architect::id()),
            operation: Operation::Mutate {
                command: Box::new(Command::Create {
                    spec: architect::transport::decode(&std::fs::read(arg(&args, 3)?)?)?,
                }),
            },
        },
        "snapshot" => Request {
            version: ipc::PROTOCOL_VERSION,
            request_id: architect::id(),
            idempotency_key: None,
            operation: Operation::Snapshot {
                run_id: arg(&args, 3)?.into(),
            },
        },
        "list" => Request {
            version: ipc::PROTOCOL_VERSION,
            request_id: architect::id(),
            idempotency_key: None,
            operation: args
                .get(3)
                .map_or(Operation::List, |after| Operation::ListPage {
                    after: after.clone(),
                }),
        },
        "events" => Request {
            version: ipc::PROTOCOL_VERSION,
            request_id: architect::id(),
            idempotency_key: None,
            operation: Operation::Events {
                after: arg(&args, 3)?
                    .parse()
                    .map_err(|_| architect::Error::Invalid("invalid event cursor".into()))?,
            },
        },
        _ => {
            return Err(architect::Error::Invalid(
                "unknown command; use --help".into(),
            ));
        }
    };
    let response = ipc::call(&state, &request)?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    if let Some(error) = response.error {
        return Err(architect::Error::Invalid(error));
    }
    Ok(())
}
fn arg(args: &[String], index: usize) -> Result<&str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| architect::Error::Invalid("missing command argument".into()))
}
fn timeout(args: &[String], index: usize) -> Result<std::time::Duration> {
    let Some(value) = args.get(index) else {
        return Ok(std::time::Duration::MAX);
    };
    let seconds = value
        .parse::<u64>()
        .map_err(|_| architect::Error::Invalid("invalid timeout seconds".into()))?;
    architect::require(seconds > 0, "timeout seconds must be positive")?;
    Ok(std::time::Duration::from_secs(seconds))
}
