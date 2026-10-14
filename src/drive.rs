//! Explicitly delegated approvals for one immutable, locally supplied task.
use crate::{
    Result,
    domain::{Run, RunSpec, Status},
    id,
    ipc::{self, Operation, Request},
    require,
    runtime::Command,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn request(state: &Path, operation: Operation) -> Result<serde_json::Value> {
    let mutation = matches!(&operation, Operation::Mutate { .. });
    let response = ipc::call(
        state,
        &Request {
            version: ipc::PROTOCOL_VERSION,
            request_id: id(),
            idempotency_key: mutation.then(id),
            operation,
        },
    )?;
    if let Some(error) = response.error {
        return Err(crate::Error::Invalid(error));
    }
    response
        .data
        .ok_or_else(|| crate::Error::Invalid("empty daemon response".into()))
}
fn mutate(state: &Path, command: Command) -> Result<serde_json::Value> {
    request(
        state,
        Operation::Mutate {
            command: Box::new(command),
        },
    )
}

/// Approves only this spec/revision; uncertainty, cancellation and drift stop it.
/// The caller explicitly authorizes the task's argv and file scope beforehand.
pub fn run(state: &Path, mut spec: RunSpec, max_repairs: usize, timeout: Duration) -> Result<Run> {
    spec.workspace = std::fs::canonicalize(&spec.workspace)?
        .to_string_lossy()
        .into_owned();
    spec.validate()?;
    deadline(timeout)?;
    let created = mutate(state, Command::Create { spec: spec.clone() })?;
    let run_id = created["id"]
        .as_str()
        .ok_or_else(|| crate::Error::Invalid("missing created run ID".into()))?
        .to_owned();
    eprintln!("Run {run_id}");
    follow(state, &run_id, spec, max_repairs, timeout)
}

fn deadline(timeout: Duration) -> Result<Option<Instant>> {
    if timeout == Duration::MAX {
        return Ok(None);
    }
    Instant::now()
        .checked_add(timeout)
        .map(Some)
        .ok_or_else(|| crate::Error::Invalid("driver timeout exceeds the clock range".into()))
}

/// Follows an existing run without creating or replaying it. Approvals are limited
/// to this exact spec at revision 1. `max_repairs` is the total persisted repair
/// budget for that revision, including approvals made by previous clients.
/// `Duration::MAX` follows without a deadline; expiry never cancels the daemon.
pub fn follow(
    state: &Path,
    run_id: &str,
    mut spec: RunSpec,
    max_repairs: usize,
    timeout: Duration,
) -> Result<Run> {
    spec.workspace = std::fs::canonicalize(&spec.workspace)?
        .to_string_lossy()
        .into_owned();
    spec.validate()?;
    let expected = serde_json::to_value(&spec)?;
    let deadline = deadline(timeout)?;
    let mut last = None;
    let mut initial_validation = true;
    loop {
        require(
            deadline.is_none_or(|deadline| Instant::now() < deadline),
            "driver timeout; inspect the persisted run before continuing",
        )?;
        let snapshot = request(
            state,
            Operation::Snapshot {
                run_id: run_id.into(),
            },
        )?;
        let status: Status = serde_json::from_value(snapshot["run"]["status"].clone())?;
        let label = (snapshot["run"]["stage"].clone(), status);
        if last.as_ref() != Some(&label) {
            eprintln!("{} {:?}", label.0, status);
            last = Some(label);
        }
        if status == Status::Running && !initial_validation {
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }
        let run: Run =
            crate::transport::decode(ipc::read_document(state, run_id, "run")?.as_bytes())?;
        require(
            run.id == run_id
                && run.spec_revision == 1
                && serde_json::to_value(&run.spec)? == expected,
            "task scope changed; delegated approvals stopped",
        )?;
        initial_validation = false;
        // A live worker can have effects in flight. Following it only observes;
        // uncertainty must be resolved before any subsequent delegated action.
        if run.status == Status::Running {
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }
        require(
            !run.reconciliation_required,
            "uncertain effects require explicit reconciliation; no automatic retry",
        )?;
        let repairs = run
            .approvals
            .iter()
            .filter(|a| a.action == "repair" && a.spec_revision == run.spec_revision)
            .count();
        // The full document can be newer than the lightweight snapshot. Only
        // this validated state may authorize an action.
        let status = run.status;
        let command = match status {
            Status::Completed => return Ok(run),
            Status::AwaitingStart => Command::ApproveStart {
                run_id: run_id.into(),
                scope_hash: run.scope_hash("start"),
            },
            Status::AwaitingResult => Command::AcceptResult {
                run_id: run_id.into(),
                scope_hash: run.scope_hash("accept"),
            },
            Status::Ready => Command::Advance {
                run_id: run_id.into(),
            },
            Status::ChangesRequested if repairs < max_repairs => {
                eprintln!("Repair {}/{max_repairs}: {:?}", repairs + 1, run.diagnostic);
                Command::ApproveRepair {
                    run_id: run_id.into(),
                    scope_hash: run.scope_hash("repair"),
                }
            }
            _ => {
                return Err(crate::Error::Invalid(format!(
                    "Run {run_id} stopped ({status:?}): {}",
                    run.diagnostic
                        .as_deref()
                        .unwrap_or("inspect persisted findings")
                )));
            }
        };
        require(
            deadline.is_none_or(|deadline| Instant::now() < deadline),
            "driver timeout; inspect the persisted run before continuing",
        )?;
        mutate(state, command)?;
    }
}
