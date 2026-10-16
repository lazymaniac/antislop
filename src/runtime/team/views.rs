//! Task-worktree mode. Team work reads a private base commit of the project
//! instead of the live checkout. A writing task applies its reviewed edits in
//! its own worktree, stacked on the results of its dependencies; readers read
//! the commit their dependencies produced. Verified results are merged into the
//! project three-way, never over local changes, with one commit per task.
use super::*;
use crate::{
    config::WorkspaceMode,
    domain::FileOperation,
    recovery::{EditReceipt, EditState},
    team::{IntegrationConflict, TaskRecord, TeamPlan, checks::CheckPhase},
    workspace::inputs::after_text,
    workspace::views::{self as git, Repo},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Inlined file contents per prompt; other files are listed by manifest only.
pub const SOURCE_BUDGET: usize = 1_200_000;
/// Inlined diff per verifier prompt.
pub const DIFF_BUDGET: usize = 400_000;
/// Receipt hash of a file that does not exist before an edit.
pub const ABSENT: &str = "absent";

/// Project integration is one critical section per daemon.
static INTEGRATION: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamViews {
    /// Private bare repository; task worktrees live next to it.
    pub repo: String,
    /// The project as captured when team work started.
    pub base: String,
    /// `base` plus every integrated task result.
    pub head: String,
    #[serde(default)]
    pub tasks: BTreeMap<String, TaskView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskView {
    pub attempt: u64,
    /// What the task's dependencies produced, on top of the integrated head.
    pub view: String,
    /// Where this attempt starts: `view`, or an earlier attempt's result.
    pub start: String,
    #[serde(default)]
    pub worktree: Option<String>,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub integration: Option<Integration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Integration {
    pub result: String,
    /// Private commit of the project files after landing.
    pub landed: String,
    pub paths: Vec<String>,
    /// Commit in the user's repository, if any path could be committed.
    pub commit: Option<String>,
    /// Landed paths left uncommitted because they had local changes.
    #[serde(default)]
    pub uncommitted: Vec<String>,
}

/// Effects of one execution, applied in the task's private worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewEffect {
    pub worktree: Option<String>,
    pub start: String,
    /// Path -> content hash at `start`, or `ABSENT`.
    pub before: BTreeMap<String, String>,
    pub result: Option<String>,
}

pub(crate) fn enabled(run: &Run) -> bool {
    run.spec
        .config
        .as_ref()
        .is_some_and(|c| c.config.workspace.mode == WorkspaceMode::TaskWorktrees)
}

pub(crate) fn writer(task: &TaskRecord) -> bool {
    !task.write_scopes.is_empty()
}

/// Model-chosen IDs become path and ref components only through this.
fn slug(id: &str) -> String {
    let readable: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(40)
        .collect();
    format!("{readable}-{}", &hash(id)[..12])
}

impl TeamViews {
    pub fn repo(&self) -> Result<Repo> {
        Repo::open_or_init(Path::new(&self.repo))
    }
    fn root(&self) -> PathBuf {
        Path::new(&self.repo)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    }
    /// The commit a task's dependents observe.
    fn effective(&self, task_id: &str) -> Option<&str> {
        self.tasks
            .get(task_id)
            .map(|v| v.result.as_deref().unwrap_or(&v.view))
    }
}

pub(crate) fn task_view<'a>(
    run: &'a Run,
    task: &TaskRecord,
) -> Result<(&'a TeamViews, &'a TaskView)> {
    let views = run
        .task_views
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("task views were not prepared".into()))?;
    let view = views
        .tasks
        .get(&task.id)
        .filter(|v| v.attempt == task.attempt)
        .ok_or_else(|| crate::Error::Invalid("task view belongs to another attempt".into()))?;
    Ok((views, view))
}

/// Directory where a task's planned checks run: the writer's worktree, or a
/// shared clean checkout of what a reader reads.
pub(crate) fn task_directory(run: &Run, task: &TaskRecord, phase: CheckPhase) -> Result<PathBuf> {
    let (views, view) = task_view(run, task)?;
    let repo = views.repo()?;
    if let Some(worktree) = &view.worktree {
        let path = PathBuf::from(worktree);
        let expected = if phase == CheckPhase::BeforeExecution {
            &view.start
        } else {
            view.result
                .as_ref()
                .ok_or_else(|| crate::Error::Invalid("task result commit missing".into()))?
        };
        require(
            repo.head(&path)? == *expected,
            "task worktree is not at the commit under check",
        )?;
        if !repo.clean(&path)? {
            repo.reset(&path, expected)?;
        }
        return Ok(path);
    }
    let path = views.root().join("views").join(&view.view);
    repo.checkout(&path, &view.view, None)?;
    if !repo.clean(&path)? {
        repo.reset(&path, &view.view)?;
    }
    Ok(path)
}

/// Reconciliation: every private worktree returns to its last recorded commit.
/// Interrupted writes and commits never recorded as applied are discarded;
/// they never reached the project.
pub(crate) fn restore_worktrees(run: &Run) -> Result<()> {
    let Some(views) = &run.task_views else {
        return Ok(());
    };
    let repo = views.repo()?;
    for view in views.tasks.values() {
        let Some(worktree) = &view.worktree else {
            continue;
        };
        let path = Path::new(worktree);
        let expected = view.result.as_ref().unwrap_or(&view.start);
        if !path.exists() {
            continue;
        }
        if repo.head(path)? != *expected || !repo.clean(path)? {
            repo.reset(path, expected)?;
        }
    }
    Ok(())
}

/// After a planned check: the task checkout must still be exactly its commit.
/// Changed files are discarded (the checkout is private) and reported.
pub(crate) fn check_directory_unchanged(
    run: &Run,
    task: &TaskRecord,
    phase: CheckPhase,
    directory: &Path,
) -> Result<()> {
    let views = run
        .task_views
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("task views were not prepared".into()))?;
    let repo = views.repo()?;
    let expected = views
        .tasks
        .get(&task.id)
        .and_then(|v| {
            if phase == CheckPhase::BeforeExecution || v.worktree.is_none() {
                Some(if v.worktree.is_some() {
                    &v.start
                } else {
                    &v.view
                })
            } else {
                v.result.as_ref()
            }
        })
        .ok_or_else(|| crate::Error::Invalid("task view missing after check".into()))?;
    let head = repo.head(directory)?;
    let clean = repo.clean(directory)?;
    if head != *expected || !clean {
        repo.reset(directory, expected)?;
        return Err(crate::Error::Invalid(
            "planned task check changed tracked files of its task checkout; changes were discarded"
                .into(),
        ));
    }
    Ok(())
}

/// Bounded prompt sources at `commit`: every in-scope file by manifest, with
/// write-scope files and then the smallest files inlined up to the budget.
pub(crate) fn prompt_sources(
    run: &Run,
    task: &TaskRecord,
    commit: &str,
    directory: Option<&str>,
) -> Result<serde_json::Value> {
    let views = run
        .task_views
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("task views were not prepared".into()))?;
    let repo = views.repo()?;
    let root = Path::new(&run.spec.workspace).canonicalize()?;
    let scopes = run
        .spec
        .input_files
        .iter()
        .map(|f| crate::workspace::inputs::relative(&root, f))
        .collect::<Result<Vec<_>>>()?;
    let within = |path: &str, scope: &str| {
        scope == "." || path == scope || path.starts_with(&format!("{scope}/"))
    };
    let entries: Vec<_> = repo
        .files(commit)?
        .into_iter()
        .filter(|e| scopes.iter().any(|s| within(&e.path, s)))
        .collect();
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by_key(|&i| {
        (
            !task
                .write_scopes
                .iter()
                .any(|s| within(&entries[i].path, s)),
            entries[i].size,
        )
    });
    let mut budget = SOURCE_BUDGET;
    let mut chosen = BTreeSet::new();
    for i in order {
        let size = entries[i].size as usize;
        if size <= budget {
            budget -= size;
            chosen.insert(i);
        }
    }
    let ids: Vec<&str> = chosen.iter().map(|&i| entries[i].blob.as_str()).collect();
    let mut blobs = repo.blobs(&ids)?.into_iter();
    let mut files = serde_json::Map::new();
    let mut omitted = 0;
    for (i, entry) in entries.iter().enumerate() {
        let content = if chosen.contains(&i) {
            blobs.next().and_then(|b| String::from_utf8(b).ok())
        } else {
            None
        };
        omitted += usize::from(content.is_none());
        files.insert(
            entry.path.clone(),
            serde_json::json!({"bytes":entry.size,"git_blob":entry.blob,"content":content}),
        );
    }
    let mut other = run.sources();
    other.retain(|key, _| !key.starts_with("file:"));
    Ok(serde_json::json!({
        "commit": commit,
        "directory": directory,
        "files": files,
        "omitted_contents": omitted,
        "context": other,
        "source_hashes": run.source_hashes(),
        "note": "Paths are relative to the project root. Files with null content were omitted for size or are binary; their manifest entry identifies the exact blob. The directory, when present, is a private checkout of this commit. Cite sources with the exact source_ref keys and hashes of source_hashes (the pinned run inputs)."
    }))
}

/// The change a verifier reviews: everything this task's attempts changed on
/// top of what its dependencies produced.
pub(crate) fn change(run: &Run, task: &TaskRecord) -> Result<serde_json::Value> {
    let (views, view) = task_view(run, task)?;
    let Some(result) = &view.result else {
        return Ok(serde_json::Value::Null);
    };
    let repo = views.repo()?;
    let mut diff = repo.diff(&view.view, result)?;
    let truncated = diff.len() > DIFF_BUDGET;
    if truncated {
        let mut end = DIFF_BUDGET;
        while !diff.is_char_boundary(end) {
            end -= 1;
        }
        diff.truncate(end);
    }
    Ok(serde_json::json!({
        "from": view.view,
        "to": result,
        "paths": repo.changed(&view.view, result)?
            .into_iter()
            .map(|(status, path)| serde_json::json!({"status":status.to_string(),"path":path}))
            .collect::<Vec<_>>(),
        "diff": diff,
        "truncated": truncated,
    }))
}

/// Executions without their full input snapshots, which are bound by hash.
pub(crate) fn effects_summary(run: &Run, task_id: &str) -> Result<Vec<serde_json::Value>> {
    run.task_executions
        .iter()
        .filter(|e| e.task_id == task_id)
        .map(|e| {
            let mut value = serde_json::to_value(e)?;
            let object = value.as_object_mut().unwrap();
            object.insert(
                "input_snapshot".into(),
                serde_json::json!({"hash":hash(serde_json::to_vec(&e.input_snapshot)?)}),
            );
            object.remove("source_hashes");
            Ok(value)
        })
        .collect()
}

/// Receipts for a prompt; runtime input-snapshot outputs are the pinned base,
/// bound by their output hash and already represented by the task sources.
pub(crate) fn receipts_summary<'a>(
    evidence: impl Iterator<Item = &'a Evidence>,
) -> Vec<serde_json::Value> {
    evidence
        .map(|e| {
            if e.source == "runtime:input-snapshot" {
                serde_json::json!({
                    "id":e.id,"source":e.source,"artifact_hash":e.artifact_hash,
                    "spec_revision":e.spec_revision,"output_hash":e.output_hash,
                    "input_hash":e.input_hash,"success":e.success,
                    "output":"Pinned base input snapshot; contents are bound by output_hash and presented as the task sources."
                })
            } else {
                serde_json::to_value(e).unwrap_or_default()
            }
        })
        .collect()
}

fn checked_edit_path(run: &Run, path: &str) -> Result<()> {
    let root = Path::new(&run.spec.workspace).canonicalize()?;
    let relative = crate::workspace::inputs::relative(&root, path)?;
    require(
        relative == path
            && Path::new(path)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
            && !git::RUNTIME_STATE
                .iter()
                .any(|s| path == *s || path.starts_with(&format!("{s}/")))
            && run.spec.editable_files.iter().any(|scope| {
                let scope = crate::workspace::inputs::relative(&root, scope).unwrap_or_default();
                scope == "." || path == scope || path.starts_with(&format!("{scope}/"))
            }),
        "task edit is outside the editable project files",
    )
}

/// Receipt hash of an edit's result; a deletion leaves the path absent.
pub(super) fn after_hash(edit: &FileEdit) -> String {
    after_text(edit).map_or_else(|| ABSENT.into(), hash)
}

fn write_file(worktree: &Path, path: &str, content: Option<&str>) -> Result<()> {
    let mut target = worktree.to_path_buf();
    for component in Path::new(path).components() {
        target.push(component);
        if let Ok(meta) = target.symlink_metadata() {
            require(
                !meta.file_type().is_symlink(),
                "task edit crosses a symlink",
            )?;
        }
    }
    let Some(content) = content else {
        std::fs::remove_file(&target)?;
        return Ok(());
    };
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&target, content)?;
    Ok(())
}

impl Runtime {
    /// Capture the run base once, then give every starting task its view and,
    /// for writers, a private worktree at the attempt's start commit.
    pub(super) fn prepare_task_views(
        &self,
        run: &Run,
        team: &TeamPlan,
        task_ids: &[String],
    ) -> Result<()> {
        if !enabled(run) {
            return Ok(());
        }
        let mut store = self.store.lock().expect("store mutex");
        let mut current = store.get(&run.id)?;
        require(
            current.spec_revision == run.spec_revision && current.attempt == run.attempt,
            "task views belong to a stale attempt",
        )?;
        if current.task_views.is_none() {
            require(
                crate::workspace::inputs_current(&current)?,
                "workspace changed before team work captured it; reconcile first",
            )?;
            let dir = store
                .data_directory()?
                .join("task-views")
                .join(slug(&current.id))
                .join("repo.git");
            let repo = Repo::open_or_init(&dir)?;
            let base = repo.capture(
                Path::new(&current.spec.workspace),
                None,
                &format!("antislop: base of run {}", current.id),
            )?;
            repo.pin("base", &base)?;
            repo.pin("head", &base)?;
            current.task_views = Some(TeamViews {
                repo: repo.dir.to_string_lossy().into_owned(),
                base: base.clone(),
                head: base,
                tasks: BTreeMap::new(),
            });
        }
        let views = current.task_views.as_mut().unwrap();
        let repo = views.repo()?;
        let mut changed = false;
        for task in team
            .tasks
            .iter()
            .filter(|t| task_ids.contains(&t.id) && t.state == TaskState::Running)
        {
            if views
                .tasks
                .get(&task.id)
                .is_some_and(|v| v.attempt == task.attempt)
            {
                continue;
            }
            let conflict = task
                .history
                .last()
                .filter(|h| h.attempt + 1 == task.attempt)
                .and_then(|h| h.integration_conflict.as_ref());
            let (view, start) = if let Some(conflict) = conflict {
                (conflict.view.clone(), conflict.start.clone())
            } else {
                let mut commits = vec![views.head.clone()];
                for dependency in &task.dependencies {
                    if let Some(commit) = views.effective(dependency) {
                        commits.push(commit.to_owned());
                    }
                }
                let view = repo.combine(&commits)?.map_err(|paths| {
                    crate::Error::Invalid(format!(
                        "dependency results of task {} conflict in {paths:?}",
                        task.id
                    ))
                })?;
                // A new attempt keeps the previous attempt's changes.
                let start = match views
                    .tasks
                    .get(&task.id)
                    .filter(|_| writer(task))
                    .and_then(|p| p.result.clone())
                {
                    Some(previous) => repo
                        .combine(&[view.clone(), previous])?
                        .unwrap_or_else(|_| view.clone()),
                    None => view.clone(),
                };
                (view, start)
            };
            let name = slug(&task.id);
            let worktree = if writer(task) {
                let path = views.root().join("tasks").join(&name);
                repo.checkout(&path, &start, Some(&format!("task/{name}")))?;
                repo.reset(&path, &start)?;
                Some(path.to_string_lossy().into_owned())
            } else {
                None
            };
            repo.pin(&format!("tasks/{name}/{}/view", task.attempt), &view)?;
            repo.pin(&format!("tasks/{name}/{}/start", task.attempt), &start)?;
            views.tasks.insert(
                task.id.clone(),
                TaskView {
                    attempt: task.attempt,
                    view,
                    start,
                    worktree,
                    result: None,
                    integration: None,
                },
            );
            changed = true;
        }
        if changed || run.task_views.is_none() {
            store.save(&current, "task_views_prepared", None)?;
        }
        Ok(())
    }

    /// Apply the observed proposal in the task's private worktree and commit it.
    /// The project and the run's pinned inputs are untouched.
    pub(super) fn apply_task_view_output(
        &self,
        captured: &Run,
        invocation: &TaskInvocation,
        session_id: &str,
        raw: &str,
        edits: &[FileEdit],
        token: &AtomicBool,
    ) -> Result<String> {
        let (mut store, mut current, scope) =
            self.admit_task_output(captured, invocation, session_id, raw, edits, token)?;
        let (views, view) = task_view(&current, &invocation.task)?;
        let repo = views.repo()?;
        let view = view.clone();
        require(
            edits.is_empty() || view.worktree.is_some(),
            "a task without write scopes cannot propose edits",
        )?;
        let mut seen = BTreeSet::new();
        let mut receipts = vec![];
        let mut before = BTreeMap::new();
        for edit in edits {
            checked_edit_path(&current, &edit.path)?;
            require(seen.insert(&edit.path), "duplicate task edit")?;
            let before_hash = repo
                .show(&view.start, &edit.path)?
                .map(hash)
                .unwrap_or_else(|| ABSENT.into());
            let exists = before_hash != ABSENT;
            require(
                match edit.operation {
                    FileOperation::Write => exists,
                    FileOperation::Create => !exists,
                    FileOperation::Delete => exists && edit.content.is_empty(),
                },
                "task edit operation does not match the file at the task start",
            )?;
            before.insert(edit.path.clone(), before_hash.clone());
            receipts.push(EditReceipt {
                path: edit.path.clone(),
                operation: edit.operation,
                before_hash,
                after_hash: after_hash(edit),
                state: EditState::Pending,
            });
        }
        if let Some(worktree) = &view.worktree {
            let path = Path::new(worktree);
            require(
                repo.head(path)? == view.start && repo.clean(path)?,
                "task worktree is not at its prepared start",
            )?;
        }
        let execution_id = id();
        let index = current.task_executions.len();
        current.task_executions.push(effects::ExecutionBinding {
            read_only_admission: None,
            id: execution_id.clone(),
            task_id: invocation.task.id.clone(),
            attempt: invocation.task.attempt,
            wave_attempt: captured.attempt,
            artifact_hash: hash(raw),
            scope,
            session_id: session_id.to_string(),
            input_snapshot: captured.input_snapshot.clone(),
            input_observation: captured.input_observation.clone(),
            source_hashes: captured.source_hashes(),
            dependency_hashes: invocation.dependency_hashes.clone(),
            after_snapshot: None,
            receipts,
            state: effects::ExecutionState::Intent,
            reconciliation: None,
            view: Some(ViewEffect {
                worktree: view.worktree.clone(),
                start: view.start.clone(),
                before,
                result: None,
            }),
        });
        store.save(&current, "task_execution_intent", None)?;
        let result = match &view.worktree {
            Some(worktree) => {
                for edit in edits {
                    require(!token.load(Ordering::SeqCst), "task cancelled before write")?;
                    write_file(Path::new(worktree), &edit.path, after_text(edit))?;
                }
                repo.commit(
                    Path::new(worktree),
                    &format!(
                        "antislop: task {} attempt {}",
                        invocation.task.id, invocation.task.attempt
                    ),
                )?
            }
            None => view.start.clone(),
        };
        let name = slug(&invocation.task.id);
        repo.pin(
            &format!("tasks/{name}/{}/result", invocation.task.attempt),
            &result,
        )?;
        let execution = &mut current.task_executions[index];
        for receipt in &mut execution.receipts {
            receipt.state = EditState::Applied;
        }
        execution.view.as_mut().unwrap().result = Some(result.clone());
        execution.state = effects::ExecutionState::Applied;
        if view.worktree.is_some() {
            current
                .task_views
                .as_mut()
                .unwrap()
                .tasks
                .get_mut(&invocation.task.id)
                .unwrap()
                .result = Some(result);
        }
        effects::authenticate(&current, &current.task_executions[index])?;
        store.save(&current, "task_execution_applied", None)?;
        Ok(execution_id)
    }

    /// Merge verified writer results into the project, dependencies first.
    pub(super) fn integrate_task_results(&self, run: &Run) -> Result<Run> {
        if !enabled(run) || run.task_views.is_none() {
            return self.store.lock().expect("store mutex").get(&run.id);
        }
        let _integration = INTEGRATION.lock().expect("integration mutex");
        loop {
            let mut store = self.store.lock().expect("store mutex");
            let mut current = store.get(&run.id)?;
            require(
                current.spec_revision == run.spec_revision && current.attempt == run.attempt,
                "task integration belongs to a stale attempt",
            )?;
            let (Some(team), Some(views)) = (current.team.clone(), current.task_views.clone())
            else {
                return Ok(current);
            };
            let integrated = |id: &str| {
                views
                    .tasks
                    .get(id)
                    .is_none_or(|v| v.result.is_none() || v.integration.is_some())
            };
            let Some(task) = team.tasks.iter().find(|t| {
                t.state == TaskState::Verified
                    && views.tasks.get(&t.id).is_some_and(|v| {
                        v.attempt == t.attempt && v.result.is_some() && v.integration.is_none()
                    })
                    && t.dependencies.iter().all(|d| integrated(d))
            }) else {
                return Ok(current);
            };
            let event = integrate(&mut current, task)?;
            store.save(&current, event, None)?;
        }
    }
}

/// Land one verified result. Conflicts send the task back to its owner with
/// the conflict markers committed in a fresh start commit.
fn integrate(run: &mut Run, task: &TaskRecord) -> Result<&'static str> {
    let views = run.task_views.clone().unwrap();
    let view = views.tasks.get(&task.id).unwrap().clone();
    let result = view.result.clone().unwrap();
    let repo = views.repo()?;
    let root = Path::new(&run.spec.workspace).canonicalize()?;
    let paths: Vec<String> = repo
        .changed(&view.view, &result)?
        .into_iter()
        .map(|(_, path)| path)
        .collect();
    let mut merged_files = vec![];
    let mut project_files = vec![];
    let mut conflicts = BTreeSet::new();
    let scratch = views.root().join("scratch");
    std::fs::create_dir_all(&scratch)?;
    let label = format!("task {}", task.id);
    for path in &paths {
        let base = repo.show(&view.view, path)?;
        let theirs = repo.show(&result, path)?;
        let ours = git::read_project(&root, path)?;
        let (merged, conflict) = git::merge_file(
            &scratch,
            base.as_deref(),
            ours.as_deref(),
            theirs.as_deref(),
            ("project", &label),
        )?;
        if conflict {
            conflicts.insert(path.clone());
        }
        project_files.push((path.clone(), ours));
        merged_files.push((path.clone(), merged));
    }
    let name = slug(&task.id);
    if !conflicts.is_empty() {
        let project = repo.apply_files(
            &views.head,
            None,
            &project_files,
            "antislop: project state for conflict resolution",
        )?;
        let start = repo.apply_files(
            &project,
            Some(&result),
            &merged_files,
            &format!("antislop: integration conflict of task {}", task.id),
        )?;
        repo.pin(&format!("tasks/{name}/{}/conflict", task.attempt), &start)?;
        let team = run.team.as_mut().unwrap();
        team.request_integration_repair(
            &task.id,
            IntegrationConflict {
                paths: conflicts,
                result,
                view: project,
                start,
            },
        )?;
        return Ok("task_integration_conflict");
    }
    let repository = !paths.is_empty() && git::project_repository(&root)?;
    let dirty = if repository {
        git::project_dirty(&root, &paths)?
    } else {
        BTreeSet::new()
    };
    for ((path, merged), (_, ours)) in merged_files.iter().zip(&project_files) {
        git::land(&root, path, ours.as_deref(), merged.as_deref())?;
    }
    let clean: Vec<String> = paths
        .iter()
        .filter(|p| !dirty.contains(*p))
        .cloned()
        .collect();
    let mut commit = None;
    if repository && !clean.is_empty() {
        let mut add = vec!["add", "-A", "--"];
        add.extend(clean.iter().map(String::as_str));
        let output = git::project_git(&root, &add)?;
        require(
            output.status == 0,
            &format!("staging task result failed: {}", output.stderr.trim()),
        )?;
        let summary: String = task
            .description
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(60)
            .collect();
        let message = format!(
            "antislop: {summary}\n\nTask: {}\nRun: {}\nVerified by: {}",
            task.id,
            run.id,
            task.review
                .as_ref()
                .map(|r| r.session_id.as_str())
                .unwrap_or_default()
        );
        let mut args = vec!["commit", "--quiet", "--no-verify", "-m", &message, "--"];
        args.extend(clean.iter().map(String::as_str));
        let output = git::project_git(&root, &args)?;
        require(
            output.status == 0,
            &format!(
                "committing task result failed (files are landed): {}",
                output.stderr.trim()
            ),
        )?;
        let head = git::project_git(&root, &["rev-parse", "HEAD"])?;
        commit = Some(String::from_utf8_lossy(&head.stdout).trim().to_owned());
    }
    let landed = if paths.is_empty() {
        views.head.clone()
    } else {
        repo.apply_files(
            &views.head,
            Some(&result),
            &merged_files,
            &format!("antislop: integrated task {}", task.id),
        )?
    };
    repo.pin("head", &landed)?;
    let state = run.task_views.as_mut().unwrap();
    state.head = landed.clone();
    state.tasks.get_mut(&task.id).unwrap().integration = Some(Integration {
        result,
        landed,
        paths,
        commit,
        uncommitted: dirty.into_iter().collect(),
    });
    Ok("task_result_integrated")
}
