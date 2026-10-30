use super::super::super::core_path_helpers::{task_path_component, worktree_branch, worktree_path};
use super::super::super::*;
use super::RoundState;
mod completion;
pub(super) use completion::{commit_round, complete_report};

pub(super) fn writer_worktree(
    core: &Core,
    task: &mut Task,
    spec: &TaskContract,
    project: &Path,
) -> Result<Option<PathBuf>> {
    if !spec.writes {
        return Ok(None);
    }
    require(
        git::is_repository(project),
        "tasks that change files need the project to be a git repository: run `git init` and commit first",
    )?;
    let base = match &task.base {
        Some(base) => base.clone(),
        None => core.writer_base(&task.plan_id, spec, project)?,
    };
    let path = worktree_path(&core.state, task)?;
    let start = task.result.clone().unwrap_or_else(|| base.clone());
    git::add_worktree(project, &path, &worktree_branch(&core.state, task)?, &start)?;
    task.base = Some(base);
    Ok(Some(path))
}

pub(super) fn task_directories(
    core: &Core,
    task: &Task,
    spec: &TaskContract,
    project: &Path,
    worktree: &Option<PathBuf>,
) -> Result<(PathBuf, Vec<PathBuf>)> {
    let observation = observation_worktree(core, task, spec, project, worktree)?;
    let workdir = worktree
        .clone()
        .or(observation)
        .unwrap_or_else(|| project.to_path_buf());
    Ok((workdir, related_directories(spec, project)))
}

fn observation_worktree(
    core: &Core,
    task: &Task,
    spec: &TaskContract,
    project: &Path,
    worktree: &Option<PathBuf>,
) -> Result<Option<PathBuf>> {
    if worktree.is_some() || !git::is_repository(project) {
        return Ok(None);
    }
    let base = core.writer_base(&task.plan_id, spec, project)?;
    let path = core.state.join("worktrees").join(format!(
        "{}-observation-g{}",
        task_path_component(&task.id)?,
        task.input_generation
    ));
    let branch = format!(
        "antislop/{}-observation-g{}",
        task.id, task.input_generation
    );
    git::add_worktree(project, &path, &branch, &base)?;
    Ok(Some(path))
}

fn related_directories(spec: &TaskContract, project: &Path) -> Vec<PathBuf> {
    spec.scope
        .iter()
        .filter_map(|path| project.join(path).canonicalize().ok())
        .map(|path| {
            if path.is_dir() {
                path
            } else {
                path.parent().map(Path::to_path_buf).unwrap_or(path)
            }
        })
        .filter(|path| !path.starts_with(project))
        .collect()
}

pub(super) fn is_recheck(
    core: &Core,
    conversation: &str,
    task: &Task,
    worker: &str,
    verifier: &str,
) -> Result<bool> {
    if task.report.is_none() {
        return Ok(false);
    }
    if task
        .feedback
        .as_ref()
        .is_some_and(|f| f["resume_role"] == "verifier")
    {
        return Ok(true);
    }
    let verifier_work = core
        .store
        .pending_mail(conversation, verifier)?
        .iter()
        .any(|mail| mail.kind == "agent_work");
    let worker_empty = core.store.pending_mail(conversation, worker)?.is_empty();
    Ok(verifier_work && worker_empty)
}

pub(super) struct WorkerRound<'a> {
    pub core: &'a Arc<Core>,
    pub conversation: &'a str,
    pub task: &'a Task,
    pub cancel: &'a AtomicBool,
    pub spec: &'a TaskContract,
    pub workdir: &'a Path,
    pub project_text: &'a str,
    pub add_dirs: &'a [PathBuf],
    pub upstream: &'a str,
    pub worker: &'a str,
    pub round: u32,
    pub state: &'a RoundState,
    pub recheck: bool,
}

pub(super) fn run(input: WorkerRound<'_>) -> Result<Option<String>> {
    if input.recheck {
        return Ok(Some(
            input
                .core
                .store
                .completed_task_output(input.task)?
                .to_string(),
        ));
    }
    run_worker_round(input)
}

fn run_worker_round(input: WorkerRound<'_>) -> Result<Option<String>> {
    let setup = worker_setup(
        input.core,
        input.conversation,
        input.task,
        input.spec,
        input.workdir,
        input.add_dirs,
        input.worker,
        input.round,
    );
    run_worker_turn(
        input.core,
        input.cancel,
        setup,
        input.project_text,
        input.spec,
        continuation(input.round, input.state),
        input.upstream,
    )
}

fn run_worker_turn(
    core: &Core,
    cancel: &AtomicBool,
    setup: TurnSetup<'_>,
    project_text: &str,
    spec: &TaskContract,
    continuation: Option<String>,
    upstream: &str,
) -> Result<Option<String>> {
    core.turn(setup, cancel, |_, _| {
        tasks::worker_turn_prompt(project_text, spec, continuation.as_deref(), upstream)
    })
}

fn worker_setup<'a>(
    core: &Core,
    conversation: &'a str,
    task: &Task,
    spec: &TaskContract,
    workdir: &Path,
    add_dirs: &[PathBuf],
    worker: &str,
    round: u32,
) -> TurnSetup<'a> {
    TurnSetup {
        session_policy: SessionPolicy::TaskRound {
            task: task.id.clone(),
            round,
        },
        conversation,
        plan_id: Some(task.plan_id.clone()),
        role: worker.to_owned(),
        profile: core
            .config_snapshot()
            .resolve(spec.kind, RoleKind::Executor),
        cwd: workdir.to_path_buf(),
        add_dirs: worker_dirs(spec, add_dirs),
        access: worker_access(spec),
        upto_message: 0,
    }
}

fn worker_dirs(spec: &TaskContract, add_dirs: &[PathBuf]) -> Vec<PathBuf> {
    if spec.writes {
        vec![]
    } else {
        add_dirs.to_vec()
    }
}

fn worker_access(spec: &TaskContract) -> agent::Access {
    if spec.writes || spec.kind == crate::config::WorkKind::Context {
        agent::Access::Write
    } else {
        agent::Access::ReadOnly
    }
}

fn continuation(round: u32, state: &RoundState) -> Option<String> {
    if round == 1 {
        None
    } else if state.resumed && round == state.first_round {
        Some(tasks::worker_resume(&state.findings))
    } else {
        Some(tasks::worker_followup(&state.findings))
    }
}
