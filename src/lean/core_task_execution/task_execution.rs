use super::super::core_worktree_cleanup::WorktreeLease;
use super::super::*;

mod review;
mod rounds;
pub(in crate::lean) mod worker;

pub(super) struct Execution<'a> {
    core: &'a Arc<Core>,
    conversation: &'a str,
    task: &'a mut Task,
    cancel: &'a AtomicBool,
    spec: TaskContract,
    config: Arc<Config>,
    project: PathBuf,
    worktree: Option<PathBuf>,
    _worktree_lease: Option<WorktreeLease<'a>>,
    workdir: PathBuf,
    project_text: String,
    add_dirs: Vec<PathBuf>,
    upstream: String,
    worker: String,
    reviewer: String,
    integration_repairs: u64,
}

struct RoundState {
    findings: Vec<serde_json::Value>,
    run: super::super::progress::Run,
    resumed: bool,
    first_round: u32,
    dependents_invalidated: bool,
}

struct Prepared {
    spec: TaskContract,
    config: Arc<Config>,
    project: PathBuf,
    worktree: Option<PathBuf>,
    workdir: PathBuf,
    add_dirs: Vec<PathBuf>,
    upstream: String,
    worker: String,
    reviewer: String,
    integration_repairs: u64,
}

impl Prepared {
    fn load(core: &Core, conversation: &str, task: &mut Task) -> Result<Self> {
        let spec = Execution::load_spec(core, task)?;
        let project = PathBuf::from(core.store.conversation(conversation)?.project);
        let worktree = worker::writer_worktree(core, task, &spec, &project)?;
        let (workdir, add_dirs) = worker::task_directories(core, task, &spec, &project, &worktree)?;
        if git::is_repository(&workdir) {
            core.store.db().execute("INSERT OR IGNORE INTO task_source_revisions(task_id,generation,revision) VALUES(?1,?2,?3)",rusqlite::params![task.id,task.input_generation,git::head(&workdir)?])?;
        }
        let mut upstream = core.upstream(&task.plan_id, &spec)?;
        if let Some(owner_id) = spec.options_for.as_deref()
            && let Some(owner) = core.store.task_in_plan_by_id(&task.plan_id, owner_id)?
            && let Some(base) = owner.base.as_deref()
        {
            let head = git::head(&workdir)?;
            upstream.push_str(&format!(
                "\nOptions review checkout context: this options task shares the owner's read-only worktree. Owner task: {owner_id}. Owner base: {base}. Current checkout HEAD: {head}. Independently inspect the current files, `git diff {base} HEAD` for the owner's committed changes, and `git diff` for any current uncommitted changes. Do not modify files.\n"
            ));
        }
        if let Some(conflict) = task
            .feedback
            .as_ref()
            .and_then(|f| f.get("checkpoint_merge"))
        {
            upstream.push_str(&format!(
                "\nRuntime checkpoint integration context: {conflict}. Your files are the preserved checkpoint because rebasing it onto reviewed upstream code conflicted. Assess the saved code and compare the referenced upstream through Git; do not infer that upstream changes are present in these files. Preserve both intended behaviors. The normal integration repair will reconcile any remaining content conflicts after review. References are runtime inputs, not acceptance gates.\n"
            ));
        }
        Ok(Self {
            spec,
            config: core.config_snapshot(),
            project,
            worktree,
            workdir,
            add_dirs,
            upstream,
            worker: format!("worker:{}", task.id),
            reviewer: format!("reviewer:{}", task.id),
            integration_repairs: task
                .feedback
                .as_ref()
                .and_then(|f| f["integration_repairs"].as_u64())
                .unwrap_or(0),
        })
    }
}

impl<'a> Execution<'a> {
    pub(super) fn new(
        core: &'a Arc<Core>,
        conversation: &'a str,
        task: &'a mut Task,
        cancel: &'a AtomicBool,
        worktree_lease: Option<WorktreeLease<'a>>,
    ) -> Result<Self> {
        let prepared = Prepared::load(core, conversation, task)?;
        Ok(Self {
            core,
            conversation,
            task,
            cancel,
            spec: prepared.spec,
            config: prepared.config,
            project: prepared.project,
            worktree: prepared.worktree,
            _worktree_lease: worktree_lease,
            project_text: prepared.workdir.to_string_lossy().into_owned(),
            workdir: prepared.workdir,
            add_dirs: prepared.add_dirs,
            upstream: prepared.upstream,
            worker: prepared.worker,
            reviewer: prepared.reviewer,
            integration_repairs: prepared.integration_repairs,
        })
    }

    fn load_spec(core: &Core, task: &Task) -> Result<TaskContract> {
        let spec: TaskContract = serde_json::from_value(task.spec.clone())?;
        spec.require_not_deferred()?;
        core.store.assert_task_inputs(task)?;
        Ok(spec)
    }

    pub(super) fn run(mut self) -> Result<()> {
        if self.resume_approved_writer()? {
            return self.integrate_approved();
        }
        let mut state = self.round_state()?;
        for round in state.first_round.. {
            if !self.run_round(round, &mut state)? {
                return Ok(());
            }
        }
        self.exhausted()
    }

    fn resume_approved_writer(&self) -> Result<bool> {
        let verdict = self
            .task
            .verdict
            .as_ref()
            .and_then(|v| serde_json::from_value::<Verdict>(v.clone()).ok());
        Ok(verdict.is_some_and(|v| v.approved) && self.worktree.is_some())
    }

    fn integrate_approved(&mut self) -> Result<()> {
        self.core.integrate(
            self.conversation,
            self.task,
            &self.spec,
            &self.project,
            self.worktree.as_ref().expect("approved writer worktree"),
            &self.config,
            self.cancel,
        )
    }

    fn round_state(&self) -> Result<RoundState> {
        let verdict = self
            .task
            .verdict
            .as_ref()
            .and_then(|v| serde_json::from_value::<Verdict>(v.clone()).ok());
        let mut findings = verdict.map(|v| v.findings).unwrap_or_default();
        let dependents_invalidated = self.dependents_invalidated()?;
        let run = self.resumed_progress()?;
        if findings.is_empty() {
            findings = run.findings.clone();
        }
        Ok(RoundState {
            findings,
            run,
            resumed: self.task.round > 0,
            first_round: self.task.round + 1,
            dependents_invalidated,
        })
    }

    fn dependents_invalidated(&self) -> Result<bool> {
        Ok(self
            .core
            .store
            .pending_mail(self.conversation, &self.worker)?
            .iter()
            .any(|mail| mail.kind == "agent_work"))
    }

    fn resumed_progress(&self) -> Result<super::super::progress::Run> {
        Ok(super::super::progress::Run::resume(
            self.core
                .store
                .task_progress(self.conversation, &self.task.id)?,
            &self.task.spec,
            self.task.input_generation,
        ))
    }

    fn exhausted(&mut self) -> Result<()> {
        self.core.stop(
            self.conversation,
            self.task,
            status::NEEDS_USER,
            Some("task attempt counter exhausted"),
        )
    }
}
