use super::super::*;

mod review;
mod rounds;
mod worker;

const MAX_ROUNDS_PER_RUN: u32 = 16;

pub(super) struct Execution<'a> {
    core: &'a Arc<Core>,
    conversation: &'a str,
    task: &'a mut Task,
    cancel: &'a AtomicBool,
    spec: TaskContract,
    config: Arc<Config>,
    project: PathBuf,
    worktree: Option<PathBuf>,
    workdir: PathBuf,
    project_text: String,
    add_dirs: Vec<PathBuf>,
    upstream: String,
    worker: String,
    verifier: String,
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
    verifier: String,
    integration_repairs: u64,
}

impl Prepared {
    fn load(core: &Core, conversation: &str, task: &mut Task) -> Result<Self> {
        let spec = Execution::load_spec(core, task)?;
        let project = PathBuf::from(core.store.conversation(conversation)?.project);
        let worktree = worker::writer_worktree(core, task, &spec, &project)?;
        let (workdir, add_dirs) = worker::task_directories(core, task, &spec, &project, &worktree)?;
        let upstream = core.upstream(&task.plan_id, &spec)?;
        Ok(Self {
            spec,
            config: core.config_snapshot(),
            project,
            worktree,
            workdir,
            add_dirs,
            upstream,
            worker: format!("worker:{}", task.id),
            verifier: format!("verifier:{}", task.id),
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
            project_text: prepared.workdir.to_string_lossy().into_owned(),
            workdir: prepared.workdir,
            add_dirs: prepared.add_dirs,
            upstream: prepared.upstream,
            worker: prepared.worker,
            verifier: prepared.verifier,
            integration_repairs: prepared.integration_repairs,
        })
    }

    fn load_spec(core: &Core, task: &Task) -> Result<TaskContract> {
        core.store.assert_task_inputs(task)?;
        Ok(serde_json::from_value(task.spec.clone())?)
    }

    pub(super) fn run(mut self) -> Result<()> {
        if self.resume_approved_writer()? {
            return self.integrate_approved();
        }
        let mut state = self.round_state()?;
        for round in state.first_round.. {
            if state.run.rejections >= MAX_ROUNDS_PER_RUN {
                return self.round_limit();
            }
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

    fn round_limit(&mut self) -> Result<()> {
        let why = format!(
            "{MAX_ROUNDS_PER_RUN} correction rounds ran without approval; the orchestrator must narrow the contract, supply a new basis or ask the user"
        );
        self.core.stop(
            self.conversation,
            self.task,
            status::COORDINATING,
            Some(&why),
        )
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
