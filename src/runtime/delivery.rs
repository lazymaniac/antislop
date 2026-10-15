//! Separately approved delivery of an authenticated complete managed result.
use super::*;
use crate::{
    team::TaskState,
    workspace::{delivery as files, snapshot},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Inspecting,
    Ready,
    Applying,
    Delivered,
    Unknown,
    Conflict,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub id: String,
    pub run_id: String,
    pub spec_revision: u64,
    pub task_binding: String,
    pub source_root: String,
    pub source_identity: files::Identity,
    pub candidate: snapshot::Snapshot,
    pub changes: Vec<files::Change>,
    pub conflicts: Vec<String>,
    pub scope_hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub state: State,
    pub plan: Option<Plan>,
    pub effects: BTreeMap<String, files::Effect>,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recovery {
    pub scope_hash: String,
    pub observation_hash: String,
    pub targets: BTreeMap<String, Option<files::Stamp>>,
    pub backups: BTreeMap<String, Option<files::Stamp>>,
    pub restorable: Vec<String>,
    pub desired_state_observed: bool,
}
pub fn recovery(run: &Run) -> Result<Recovery> {
    let record = run
        .delivery
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("no source delivery".into()))?;
    require(
        matches!(record.state, State::Unknown | State::Conflict),
        "source delivery has no uncertain/conflicting effects",
    )?;
    let plan = record
        .plan
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("source delivery plan missing".into()))?;
    require(
        files::Identity::read(Path::new(&plan.source_root))? == plan.source_identity,
        "delivery source root identity changed",
    )?;
    let mut targets = BTreeMap::new();
    let mut backups = BTreeMap::new();
    let mut restorable = vec![];
    let mut desired = !plan.changes.is_empty();
    for (index, change) in plan.changes.iter().enumerate() {
        let target = files::stamp(Path::new(&plan.source_root), &change.path)?;
        let backup = files::stamp(
            Path::new(&plan.source_root),
            &format!(".antislop-delivery-{}/{index}", plan.id),
        )?;
        if target.is_none() && backup.is_some() {
            restorable.push(change.path.clone());
        }
        desired &= target == change.after.as_ref().map(|e| e.stamp.clone())
            && record.effects.contains_key(&change.path);
        targets.insert(change.path.clone(), target);
        backups.insert(change.path.clone(), backup);
    }
    let observation_hash = hash(serde_json::to_vec(&(
        &plan.id,
        &plan.source_identity,
        &targets,
        &backups,
    ))?);
    let scope_hash = hash(serde_json::to_vec(&(
        "reconcile_source_delivery_v1",
        &run.id,
        run.spec_revision,
        &plan.scope_hash,
        &record.state,
        &record.effects,
        &observation_hash,
    ))?);
    Ok(Recovery {
        scope_hash,
        observation_hash,
        targets,
        backups,
        restorable,
        desired_state_observed: desired,
    })
}
pub(super) fn reconcile(run: &mut Run, scope_hash: &str, observation_hash: &str) -> Result<()> {
    let observed = recovery(run)?;
    require(
        observed.scope_hash == scope_hash && observed.observation_hash == observation_hash,
        "source delivery observation changed",
    )?;
    require(
        observed.restorable.is_empty(),
        "restore missing source entries from retained backups before settling transfer effects",
    )?;
    let record = run.delivery.as_mut().unwrap();
    record.state = if observed.desired_state_observed {
        State::Delivered
    } else {
        State::Conflict
    };
    record.diagnostic=Some("Explicitly acknowledged observed source/backup versions without replay; matching state does not invent authorship".into());
    run.reconciliation_required = false;
    if run.status != Status::Cancelled {
        run.status = Status::Completed;
        run.paused_from = None;
    }
    record_approval(run, "reconcile-source-delivery", scope_hash);
    Ok(())
}
pub(super) fn restore(run: &mut Run, path: &str, scope_hash: &str) -> Result<()> {
    let observed = recovery(run)?;
    require(
        observed.scope_hash == scope_hash && observed.restorable.iter().any(|p| p == path),
        "source backup restoration scope is stale or destination is not missing",
    )?;
    let plan = run.delivery.as_ref().unwrap().plan.as_ref().unwrap();
    let index = plan.changes.iter().position(|c| c.path == path).unwrap();
    files::restore_missing(
        Path::new(&plan.source_root),
        &plan.source_identity,
        path,
        &format!(".antislop-delivery-{}/{index}", plan.id),
        observed.backups[path].as_ref().unwrap(),
    )?;
    record_approval(run, "restore-source-delivery-backup", scope_hash);
    Ok(())
}
pub fn eligible(run: &Run) -> Result<()> {
    require(
        run.status == Status::Completed && !run.reconciliation_required,
        "delivery requires a completed reconciled task",
    )?;
    crate::workspace::managed::require_active(run)?;
    require(
        run.managed_workspace.is_some() && run.team.is_some(),
        "delivery requires a managed, independently verified team result",
    )?;
    require(
        run.spec
            .config
            .as_ref()
            .is_some_and(|p| p.config.policy.local_write),
        "source delivery is disabled by local-write policy",
    )?;
    require(
        !run.delivery.as_ref().is_some_and(|r| {
            matches!(r.state, State::Applying | State::Unknown | State::Delivered)
        }),
        "inspect/reconcile the existing transfer; never replay it",
    )?;
    require(
        run.element_comments.iter().all(|c| c.deleted || c.resolved)
            && run.diff_comments.iter().all(|c| c.resolved),
        "resolve outstanding user comments before delivery",
    )
}
fn task_state(run: &Run) -> Result<String> {
    Ok(hash(serde_json::to_vec(&(
        &run.id,
        &run.spec,
        run.spec_revision,
        run.source_hashes(),
        &run.team,
        &run.task_executions,
        &run.team_result_approvals,
        &run.session_ledger,
        &run.quality_gates,
        &run.provenance,
        &run.provenance_reviews,
    ))?))
}
fn scope(run: &Run, plan: &Plan) -> Result<String> {
    Ok(hash(serde_json::to_vec(&(
        "deliver_source_v1",
        task_state(run)?,
        &plan.id,
        &plan.run_id,
        plan.spec_revision,
        &plan.task_binding,
        &plan.source_root,
        &plan.source_identity,
        &plan.candidate,
        &plan.changes,
        &plan.conflicts,
    ))?))
}
pub fn pending(run: &Run) -> Result<&Plan> {
    eligible(run)?;
    let record = run.delivery.as_ref().ok_or_else(|| {
        crate::Error::Invalid("inspect completed candidate before delivery".into())
    })?;
    require(
        record.state == State::Ready,
        "delivery proposal is not ready",
    )?;
    let plan = record
        .plan
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("delivery plan missing".into()))?;
    require(
        plan.conflicts.is_empty()
            && !plan.changes.is_empty()
            && scope(run, plan)? == plan.scope_hash,
        "delivery plan is conflicting, empty or stale",
    )?;
    Ok(plan)
}

fn inspect(run: &Run, token: &AtomicBool) -> Result<Plan> {
    eligible(run)?;
    let verified = team::stage::delivery_binding(run)?;
    require(
        run.team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .all(|t| t.state == TaskState::Verified),
        "unverified task cannot be delivered",
    )?;
    let workspace = run.managed_workspace.as_ref().unwrap();
    let identity=workspace.plan.project_identity.clone().ok_or_else(||crate::Error::Invalid("legacy workspace lacks its original project identity; explicitly resolve it before transfer".into()))?;
    let source = Path::new(&workspace.original_spec.workspace);
    require(
        files::Identity::read(source)? == identity,
        "original source project was replaced",
    )?;
    let current = snapshot::observe(Path::new(&run.spec.workspace), &BTreeSet::new(), token)?;
    require(
        current.gitlinks.is_empty() && current.unobserved_directories.is_empty(),
        "delivery requires fully observed candidate files",
    )?;
    require(
        current.repository.as_ref().and_then(|r| r.head.as_ref())
            == workspace.receipt.as_ref().map(|r| &r.commit),
        "candidate commit changed outside its accepted workspace binding",
    )?;
    let prefix = &workspace.plan.project_prefix;
    let baseline: BTreeMap<_, _> = workspace
        .plan
        .files
        .iter()
        .filter_map(|(p, f)| p.strip_prefix(prefix).map(|p| (p.to_owned(), f)))
        .collect();
    let paths: BTreeSet<_> = baseline
        .keys()
        .chain(current.files.keys())
        .cloned()
        .collect();
    let owned = team::delivery_owned_effects(run)?;
    let mut conflicts = vec![];
    if workspace.plan.source.validate_current(token).is_err() {
        conflicts.push("Original checkout context changed; integrate and verify the current user version before delivery".into());
    }
    let unowned: Vec<_> = workspace
        .plan
        .source
        .local_changes
        .difference(&workspace.plan.selected)
        .collect();
    if !unowned.is_empty() {
        conflicts.push(format!("Integration must include unselected user local changes before source delivery: {unowned:?}"));
    }
    let mut changes = vec![];
    for path in paths {
        let before = baseline.get(&path);
        let observed = current.files.get(&path).and_then(|v| v.as_ref());
        if before.map(|f| &f.content_hash) == observed.and_then(|f| f.content_hash.as_ref())
            && before.map(|f| f.mode == "120000")
                == observed.map(|f| f.kind == snapshot::FileKind::Symlink)
            && before.map(|f| f.mode == "100755") == observed.map(|f| f.permissions & 0o100 != 0)
        {
            continue;
        }
        let mut after = files::read(Path::new(&run.spec.workspace), &path)?;
        let source_path = format!("{prefix}{path}");
        let original = workspace
            .plan
            .source
            .files
            .get(&source_path)
            .and_then(|f| f.as_ref());
        let source_entry = files::read(source, &path)?;
        if original.map(|f| f.content_hash.as_ref())
            != source_entry.as_ref().map(|e| Some(&e.stamp.sha256))
            || original.map(|f| (f.permissions, f.owner_uid, f.owner_gid))
                != source_entry
                    .as_ref()
                    .map(|e| (e.stamp.permissions, e.stamp.owner_uid, e.stamp.owner_gid))
        {
            conflicts.push(format!("Source target changed or was not observed: {path}"));
        }
        let supported = owned.get(&path) == Some(&after.as_ref().map(|e| e.stamp.sha256.clone()))
            && after
                .as_ref()
                .is_none_or(|e| e.stamp.kind == files::Kind::File)
            && before.is_none_or(|f| f.mode != "120000")
            && after.as_ref().is_none_or(|e| {
                (e.stamp.permissions & 0o100 != 0) == before.is_some_and(|f| f.mode == "100755")
            });
        if !supported {
            conflicts.push(format!(
                "Candidate change has no supported authenticated managed effect: {path}"
            ));
        }
        if let Some(entry) = &mut after
            && let Some(old) = &source_entry
        {
            entry.stamp.permissions = old.stamp.permissions;
            entry.stamp.owner_uid = old.stamp.owner_uid;
            entry.stamp.owner_gid = old.stamp.owner_gid;
        }
        changes.push(files::Change {
            path,
            before: source_entry.map(|e| e.stamp),
            after,
        });
    }
    let mut plan = Plan {
        id: id(),
        run_id: run.id.clone(),
        spec_revision: run.spec_revision,
        task_binding: verified,
        source_root: source.to_string_lossy().into(),
        source_identity: identity,
        candidate: current,
        changes,
        conflicts,
        scope_hash: String::new(),
    };
    plan.scope_hash = scope(run, &plan)?;
    Ok(plan)
}

impl Runtime {
    pub(super) fn work_delivery(
        self: Arc<Self>,
        mut run: Run,
        apply: bool,
        token: Arc<AtomicBool>,
        leases: Vec<crate::recovery::WorkspaceLease>,
    ) {
        let fds: Vec<_> = leases.iter().flat_map(|l| l.fds()).collect();
        let _lease_scope = crate::process::LeaseScope::new(&fds);
        run.status = Status::Completed; // Inspect against the accepted task, not temporary worker status.
        run.reconciliation_required = false;
        let outcome = (|| -> Result<Plan> {
            require(!token.load(Ordering::SeqCst), "delivery cancelled")?;
            if !apply {
                return inspect(&run, &token);
            }
            let plan = run
                .delivery
                .as_ref()
                .and_then(|r| r.plan.clone())
                .ok_or_else(|| crate::Error::Invalid("approved delivery plan missing".into()))?;
            require(
                team::stage::delivery_binding(&run)? == plan.task_binding,
                "accepted task verification changed before transfer",
            )?;
            plan.candidate.validate_current(&token)?;
            run.managed_workspace
                .as_ref()
                .unwrap()
                .plan
                .source
                .validate_current(&token)?;
            files::apply(
                Path::new(&plan.source_root),
                &plan.source_identity,
                &plan.id,
                &plan.changes,
                &mut |effect| {
                    require(
                        !token.load(Ordering::SeqCst),
                        "delivery cancelled before effect checkpoint",
                    )?;
                    let mut store = if effect.state == files::EffectState::Intent {
                        self.active_store(&run)?
                    } else {
                        self.store.lock().expect("store mutex")
                    };
                    let mut current = store.get(&run.id)?;
                    require(
                        current.spec_revision == run.spec_revision
                            && current.status != Status::Cancelled,
                        "delivery task changed or was cancelled",
                    )?;
                    require(
                        serde_json::to_value(&current.spec)? == serde_json::to_value(&run.spec)?,
                        "delivery task configuration changed before an effect",
                    )?;
                    let record = current.delivery.as_mut().unwrap();
                    require(
                        record.plan.as_ref().is_some_and(|p| p.id == plan.id)
                            && record.state == State::Applying,
                        "delivery intent no longer active",
                    )?;
                    record.effects.insert(effect.path.clone(), effect.clone());
                    store.save(&current, "source_delivery_effect", None)
                },
            )?;
            Ok(plan)
        })();
        let mut store = self.store.lock().expect("store mutex");
        if let Ok(mut current) = store.get(&run.id)
            && current.spec_revision == run.spec_revision
        {
            let cancelled = current.status == Status::Cancelled || token.load(Ordering::SeqCst);
            let record = current.delivery.as_mut().unwrap();
            match outcome {
                Ok(plan) if !cancelled => {
                    record.state = if apply {
                        State::Delivered
                    } else if plan.conflicts.is_empty() {
                        State::Ready
                    } else {
                        State::Conflict
                    };
                    record.plan = Some(plan);
                    record.diagnostic = None;
                    current.reconciliation_required = false;
                    current.diagnostic=Some(if apply {"Approved file changes delivered; retained previous versions remain available. Commit/push/publication are separate operations."} else {"Delivery candidate inspected; inspect conflicts and approve the exact transfer separately."}.into());
                }
                result => {
                    record.state = if apply { State::Unknown } else { State::Failed };
                    record.diagnostic =
                        Some(result.err().map(|e| e.to_string()).unwrap_or_else(|| {
                            "Delivery cancelled; inspect retained state before any further effect"
                                .into()
                        }));
                    current.reconciliation_required = apply;
                    current.diagnostic = record.diagnostic.clone();
                }
            }
            if current.status == Status::Paused {
                current.paused_from = Some(Status::Completed);
            } else if current.status != Status::Cancelled {
                current.status = Status::Completed;
            }
            if let Err(error) = store.save(&current, "source_delivery_finished", None) {
                eprintln!("Cannot persist source delivery outcome: {error}");
            }
        }
        drop(store);
        drop(_lease_scope);
        drop(leases);
        self.workers.lock().expect("workers mutex").remove(&run.id);
        self.work_changed.notify_all();
    }
}
