use crate::lean::core::worktree::cleanup::WorktreeLease;
use crate::lean::prompts;
use crate::lean::*;

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
    run: crate::lean::progress::Run,
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
        let mut upstream = if spec.integration_batch {
            String::new()
        } else {
            core.upstream(&task.plan_id, &spec)?
        };
        if let Some(owner_id) = spec.options_for.as_deref()
            && let Some(owner) = core.store.task_in_plan_by_id(&task.plan_id, owner_id)?
            && let Some(base) = owner.base.as_deref()
        {
            let head = git::head(&workdir)?;
            upstream.push_str(&format!(
                prompts::template!(OPTIONS_REVIEW_CHECKOUT),
                owner_id = owner_id,
                base = base,
                head = head
            ));
        }
        if let Some(conflict) = task
            .feedback
            .as_ref()
            .and_then(|f| f.get("checkpoint_merge"))
        {
            upstream.push_str(&format!(
                prompts::template!(CHECKPOINT_REVIEW_CONTEXT),
                conflict = conflict
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
        if self.spec.integration_batch {
            return self.run_batch_integration();
        }
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

    fn run_batch_integration(&mut self) -> Result<()> {
        let worktree = self.worktree.clone().ok_or_else(|| {
            Error::Invalid("final integration task has no writer worktree".into())
        })?;
        let base = self
            .core
            .initial_plan_anchor(&self.task.plan_id, &self.project)?;
        if !self.assemble_batch(&worktree, &base)? {
            return Ok(());
        }
        self.task.base = Some(base);
        self.task.report = Some(
            "Integrated all accepted code task results after their producers completed.".into(),
        );
        self.core.store.update_task(self.task)?;
        loop {
            self.core.integrate(
                self.conversation,
                self.task,
                &self.spec,
                &self.project,
                &worktree,
                &self.config,
                self.cancel,
            )?;
            if self.task.status != status::WORKING {
                return Ok(());
            }
        }
    }

    fn assemble_batch(&mut self, worktree: &Path, base: &str) -> Result<bool> {
        let completed: std::collections::BTreeSet<String> = self
            .task
            .feedback
            .as_ref()
            .and_then(|feedback| feedback["batch_completed"].as_array())
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect();
        let mut next_completed = completed;
        let producer_ids = self.spec.depends_on.clone();
        for producer_id in &producer_ids {
            if next_completed.contains(producer_id) {
                continue;
            }
            require(
                !self.cancel.load(Ordering::SeqCst),
                "final integration cancelled",
            )?;
            let producer = self
                .core
                .store
                .task_in_plan_by_id(&self.task.plan_id, producer_id)?
                .ok_or_else(|| Error::Invalid(format!("missing producer {producer_id}")))?;
            require(
                producer.status == status::APPROVED,
                &format!("producer {producer_id} is not accepted"),
            )?;
            let producer_spec: TaskContract = serde_json::from_value(producer.spec.clone())?;
            if producer_spec.change_kind() != tasks::ChangeKind::Code {
                continue;
            }
            let Some(producer_base) = producer.base else {
                return Err(Error::Invalid(format!(
                    "accepted code producer {producer_id} has no recorded base for final integration"
                )));
            };
            if let Some(result) = producer.result {
                let onto = git::snapshot(worktree, git::CommitPurpose::WorkspaceSnapshot)?;
                match apply_batch_delta(worktree, &producer_base, &result)? {
                    BatchDelta::Conflict(outcome, files) => {
                        self.repair_batch_conflict(
                            worktree,
                            &producer_base,
                            &result,
                            &onto,
                            outcome,
                            &files,
                        )?;
                        if self.task.status != status::WORKING {
                            return Ok(false);
                        }
                    }
                    BatchDelta::Applied => {
                        if !git::uncommitted(worktree)?.is_empty() {
                            self.task.result =
                                Some(git::commit_all(worktree, git::CommitPurpose::TaskResult)?);
                        }
                    }
                }
            } else if producer
                .feedback
                .as_ref()
                .and_then(|feedback| feedback["integration"]["outcome"].as_str())
                != Some("no_changes")
            {
                return Err(Error::Invalid(format!(
                    "accepted code producer {producer_id} has no recorded result and was not accepted with no changes"
                )));
            }
            self.task.base = Some(base.to_owned());
            next_completed.insert(producer_id.clone());
            self.task.feedback.get_or_insert_with(|| json!({}))["batch_completed"] =
                json!(next_completed);
            self.core.store.update_task(self.task)?;
        }
        Ok(true)
    }

    fn repair_batch_conflict(
        &mut self,
        worktree: &Path,
        producer_base: &str,
        result: &str,
        onto: &str,
        outcome: Outcome,
        files: &[String],
    ) -> Result<()> {
        self.task.base = Some(producer_base.to_owned());
        self.core.store.update_task(self.task)?;
        self.core.repair_integration(
            self.conversation,
            self.task,
            &self.spec,
            worktree,
            onto,
            result,
            outcome,
            files,
            self.cancel,
        )?;
        Ok(())
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

    fn resumed_progress(&self) -> Result<crate::lean::progress::Run> {
        Ok(crate::lean::progress::Run::resume(
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

enum BatchDelta {
    Applied,
    Conflict(Outcome, Vec<String>),
}

fn apply_batch_delta(worktree: &Path, base: &str, result: &str) -> Result<BatchDelta> {
    match git::integrate(worktree, base, result, false)? {
        git::Integration::Conflict(files) => Ok(BatchDelta::Conflict(Outcome::Conflict, files)),
        git::Integration::UserEdits(files) => Ok(BatchDelta::Conflict(Outcome::UserEdits, files)),
        git::Integration::Applied | git::Integration::Committed(_) => Ok(BatchDelta::Applied),
    }
}

#[cfg(test)]
mod integration_batch_tests {
    use super::*;

    #[test]
    fn generated_batch_task_lands_the_accepted_producer_after_its_dependency() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir(&project).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(&project)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(project.join("base.txt"), "base\n").unwrap();
        let base = git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
        let config =
            Config::from_toml(include_str!("../../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&directory.path().join("state"), config.clone()).unwrap();
        let conversation = core
            .store
            .create_conversation(project.to_str().unwrap())
            .unwrap();
        core.store
            .select_team(
                &conversation.id,
                tasks::Team::NoQualityGates,
                "No quality gates",
            )
            .unwrap();
        let request = core
            .store
            .add_message(&conversation.id, "user", "Add one file")
            .unwrap();
        core.store
            .interview_summary(&conversation.id, "Add one file", request.seq)
            .unwrap();
        core.store.confirm_scope_for_test(&conversation.id);
        core.create_session_branch(&conversation.id, &project)
            .unwrap();
        let run = core
            .store
            .create_scope(&conversation.id, "Add one file")
            .unwrap();
        let producer = serde_json::from_value::<TaskContract>(json!({
            "id":"writer", "title":"Write file", "goal":"Add delivered file",
            "kind":"implementation", "change_kind":"code",
            "acceptance_criteria":[{"id":"C1","text":"The file exists"}]
        }))
        .unwrap();
        let mut second_producer = producer.clone();
        second_producer.id = "writer-two".into();
        second_producer.title = "Write another file".into();
        let mut no_change_producer = producer.clone();
        no_change_producer.id = "writer-no-change".into();
        no_change_producer.title = "Confirm existing file".into();
        let mut finalizer = producer.clone();
        finalizer.id = "final-integration".into();
        finalizer.title = "Integrate delivered changes".into();
        finalizer.kind = WorkKind::Integration;
        finalizer.integration_batch = true;
        finalizer.depends_on = vec![
            "writer".into(),
            "writer-two".into(),
            "writer-no-change".into(),
        ];
        let mut rows = core
            .store
            .add_plan(
                &conversation.id,
                &[
                    serde_json::to_value(&producer).unwrap(),
                    serde_json::to_value(&second_producer).unwrap(),
                    serde_json::to_value(&no_change_producer).unwrap(),
                    serde_json::to_value(&finalizer).unwrap(),
                ],
            )
            .unwrap();
        core.store
            .bind_plan_scope(&conversation.id, &rows[0].plan_id, &run.id)
            .unwrap();
        let prepare_writer = |mut writer: Task, path: &str| {
            let spec: TaskContract = serde_json::from_value(writer.spec.clone()).unwrap();
            let worktree = worker::writer_worktree(&core, &mut writer, &spec, &project)
                .unwrap()
                .unwrap();
            std::fs::write(worktree.join(path), format!("{path}\n")).unwrap();
            writer.result =
                Some(git::commit_all(&worktree, git::CommitPurpose::TestFixture).unwrap());
            writer.status = status::APPROVED.into();
            core.store.update_task(&writer).unwrap();
        };
        prepare_writer(rows.remove(0), "delivered.txt");
        prepare_writer(rows.remove(0), "also-delivered.txt");
        let mut no_change_producer = rows.remove(0);
        no_change_producer.base = Some(base.clone());
        no_change_producer.feedback = Some(json!({
            "round": no_change_producer.round,
            "integration": {"outcome": "no_changes", "detail": "no file changes"}
        }));
        no_change_producer.status = status::APPROVED.into();
        core.store.update_task(&no_change_producer).unwrap();

        let mut finalizer = rows.remove(0);
        finalizer.status = status::WORKING.into();
        core.store.update_task(&finalizer).unwrap();
        Execution::new(
            &core,
            &conversation.id,
            &mut finalizer,
            &AtomicBool::new(false),
            None,
        )
        .unwrap()
        .run()
        .unwrap();

        assert_eq!(finalizer.status, status::APPROVED);
        let session = core.session_worktree(&conversation.id, &project).unwrap();
        assert_eq!(
            std::fs::read_to_string(session.join("delivered.txt")).unwrap(),
            "delivered.txt\n"
        );
        assert_eq!(
            std::fs::read_to_string(session.join("also-delivered.txt")).unwrap(),
            "also-delivered.txt\n"
        );
        assert_eq!(git::head(&project).unwrap(), base);
        assert_eq!(
            core.store
                .task_in_plan_by_id(&finalizer.plan_id, "writer")
                .unwrap()
                .unwrap()
                .status,
            status::APPROVED,
            "producer acceptance remains available while final integration lands its result"
        );
    }

    #[test]
    fn batch_merge_surfaces_conflicts_and_git_failures_without_mutating_the_project() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir(&project).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(&project)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(project.join("shared.txt"), "base\n").unwrap();
        let base = git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
        let first_tree = directory.path().join("first");
        let second_tree = directory.path().join("second");
        let batch_tree = directory.path().join("batch");
        git::add_worktree(&project, &first_tree, "test-first", &base).unwrap();
        git::add_worktree(&project, &second_tree, "test-second", &base).unwrap();
        git::add_worktree(&project, &batch_tree, "test-batch", &base).unwrap();
        std::fs::write(first_tree.join("shared.txt"), "first producer\n").unwrap();
        let first = git::commit_all(&first_tree, git::CommitPurpose::TestFixture).unwrap();
        std::fs::write(second_tree.join("shared.txt"), "second producer\n").unwrap();
        let second = git::commit_all(&second_tree, git::CommitPurpose::TestFixture).unwrap();
        assert!(matches!(
            apply_batch_delta(&batch_tree, &base, &first).unwrap(),
            BatchDelta::Applied
        ));
        git::commit_all(&batch_tree, git::CommitPurpose::TestFixture).unwrap();
        match apply_batch_delta(&batch_tree, &base, &second).unwrap() {
            BatchDelta::Conflict(Outcome::Conflict, files) => assert_eq!(files, ["shared.txt"]),
            _ => panic!("overlapping producer results must enter conflict repair"),
        }
        assert_eq!(
            std::fs::read_to_string(batch_tree.join("shared.txt")).unwrap(),
            "first producer\n"
        );
        assert_eq!(
            std::fs::read_to_string(project.join("shared.txt")).unwrap(),
            "base\n"
        );
        assert!(apply_batch_delta(&batch_tree, &base, "missing-commit").is_err());
        assert_eq!(
            std::fs::read_to_string(project.join("shared.txt")).unwrap(),
            "base\n"
        );
    }
}
