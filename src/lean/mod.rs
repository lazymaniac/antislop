//! Lean core: native CLI agents working on the project's own files, small
//! rows of state and an event log. See docs/lean-core.md.

pub mod agent;
pub mod cli;
pub mod daemon;
pub mod git;
pub mod store;
pub mod tasks;
pub mod tui;

use crate::{
    Result,
    config::{Config, ResolvedProfile, RoleKind, WorkKind},
    require,
};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use store::{Conversation, Message, Store, Task, TurnStatus};
use tasks::{TaskSpec, status};

const ORCHESTRATOR: &str = "orchestrator";
const RUNTIME: &str = "runtime";
/// Longest worker report handed back to the orchestrator per task.
const REPORT_LIMIT: usize = 40_000;
/// Tasks of one plan running at once; the rest wait their turn.
const PARALLEL_TASKS: usize = 4;

pub struct Core {
    pub store: Store,
    /// One process owns a state directory at a time.
    _lock: std::fs::File,
    config: Config,
    /// Private state directory; writers' worktrees live under it.
    state: PathBuf,
    /// Writers land on the user's branch one at a time.
    integrating: Mutex<()>,
    /// Conversations with an orchestrator loop running, and its cancel flag.
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Conversations with a plan running, and its cancel flag.
    plans: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

/// Who runs a turn, where, and with what access.
struct TurnSetup<'a> {
    conversation: &'a str,
    role: String,
    profile: Result<ResolvedProfile>,
    cwd: PathBuf,
    add_dirs: Vec<PathBuf>,
    access: agent::Access,
    upto_message: u64,
}

impl Core {
    pub fn open(state: &Path, config: Config) -> Result<Arc<Self>> {
        crate::ipc::private_directory(state)?;
        // Keeps the state (and worktrees in it) out of the project's git status.
        let ignore = state.join(".gitignore");
        if !ignore.exists() {
            std::fs::write(ignore, "*\n")?;
        }
        let lock = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(state.join("core.lock"))?;
        // SAFETY: flock on an owned descriptor that lives as long as the Core.
        require(
            unsafe {
                libc::flock(
                    std::os::fd::AsRawFd::as_raw_fd(&lock),
                    libc::LOCK_EX | libc::LOCK_NB,
                )
            } == 0,
            &format!("another antislop process owns {}", state.display()),
        )?;
        let store = Store::open(&state.join("core.db"))?;
        store.interrupt_running_turns()?;
        store.interrupt_unfinished_tasks()?;
        Ok(Arc::new(Self {
            store,
            _lock: lock,
            config,
            state: state.canonicalize()?,
            integrating: Mutex::new(()),
            running: Mutex::new(HashMap::new()),
            plans: Mutex::new(HashMap::new()),
        }))
    }

    pub fn start(&self, project: &Path) -> Result<Conversation> {
        let project = project.canonicalize()?;
        self.store.create_conversation(&project.to_string_lossy())
    }

    /// Records the message and returns at once. The orchestrator answers in
    /// the background; messages sent meanwhile go into its next turn.
    pub fn send(self: &Arc<Self>, conversation: &str, text: &str) -> Result<Message> {
        self.store.conversation(conversation)?;
        let message = self.store.add_message(conversation, "user", text)?;
        self.wake(conversation);
        Ok(message)
    }

    /// Stops the orchestrator's turn and any running plan; queued messages stay.
    pub fn cancel(&self, conversation: &str) {
        for flags in [&self.running, &self.plans] {
            if let Some(flag) = flags.lock().expect("flags mutex").get(conversation) {
                flag.store(true, Ordering::SeqCst);
            }
        }
    }

    pub fn is_idle(&self, conversation: &str) -> bool {
        !self
            .running
            .lock()
            .expect("running mutex")
            .contains_key(conversation)
            && !self
                .plans
                .lock()
                .expect("plans mutex")
                .contains_key(conversation)
    }

    /// Whether any conversation has an agent or a plan running.
    pub fn busy(&self) -> bool {
        !self.running.lock().expect("running mutex").is_empty()
            || !self.plans.lock().expect("plans mutex").is_empty()
    }

    /// Starts the proposed plan `plan_id`: exactly the one the user saw, even
    /// if the orchestrator has proposed another since.
    pub fn accept_plan(self: &Arc<Self>, conversation: &str, plan_id: &str) -> Result<Vec<Task>> {
        let mut plans = self.plans.lock().expect("plans mutex");
        require(
            !plans.contains_key(conversation),
            "a plan is already running",
        )?;
        self.store.queue_plan(conversation, plan_id)?;
        let tasks = self.store.plan(plan_id)?;
        for task in &tasks {
            self.announce_task(conversation, task);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        plans.insert(conversation.to_owned(), cancel.clone());
        drop(plans);
        let core = self.clone();
        let conversation = conversation.to_owned();
        let plan_id = plan_id.to_owned();
        std::thread::spawn(move || {
            core.run_plan(&conversation, &plan_id, &cancel);
            core.plans
                .lock()
                .expect("plans mutex")
                .remove(&conversation);
            let _ = core.report_plan(&conversation, &plan_id);
        });
        Ok(tasks)
    }

    fn wake(self: &Arc<Self>, conversation: &str) {
        let mut running = self.running.lock().expect("running mutex");
        if running.contains_key(conversation) {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        running.insert(conversation.to_owned(), cancel.clone());
        drop(running);
        let core = self.clone();
        let conversation = conversation.to_owned();
        std::thread::spawn(move || core.orchestrate(&conversation, &cancel));
    }

    fn orchestrate(&self, conversation: &str, cancel: &AtomicBool) {
        loop {
            // Decide under the lock so a message sent right now either is seen
            // here or finds no loop running and starts one.
            let upto = {
                let mut running = self.running.lock().expect("running mutex");
                match self.unanswered(conversation) {
                    Ok(Some(upto)) => upto,
                    _ => {
                        running.remove(conversation);
                        return;
                    }
                }
            };
            cancel.store(false, Ordering::SeqCst);
            if self.orchestrator_turn(conversation, upto, cancel).is_err() {
                // The store itself failed; retrying would spin. The next
                // message starts a new loop.
                self.running
                    .lock()
                    .expect("running mutex")
                    .remove(conversation);
                return;
            }
        }
    }

    /// The newest message for the orchestrator after its last turn, if any.
    fn unanswered(&self, conversation: &str) -> Result<Option<u64>> {
        let after = self
            .store
            .last_turn(conversation, ORCHESTRATOR)?
            .map_or(0, |turn| turn.upto_message);
        Ok(self
            .store
            .messages_after(conversation, after)?
            .into_iter()
            .filter(|message| message.author != ORCHESTRATOR)
            .map(|message| message.seq)
            .next_back())
    }

    fn orchestrator_turn(&self, conversation: &str, upto: u64, cancel: &AtomicBool) -> Result<()> {
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        // A failed turn may never have reached the agent: repeat everything
        // asked since the last answer.
        let answered = self.store.last_answered(conversation, ORCHESTRATOR)?;
        let asked: Vec<Message> = self
            .store
            .messages_after(conversation, answered)?
            .into_iter()
            .filter(|message| message.author != ORCHESTRATOR && message.seq <= upto)
            .collect();
        let setup = TurnSetup {
            conversation,
            role: ORCHESTRATOR.into(),
            profile: self
                .config
                .resolve(WorkKind::Orchestration, RoleKind::Orchestrator),
            cwd: project.clone(),
            add_dirs: vec![],
            access: agent::Access::ReadOnly,
            upto_message: upto,
        };
        let Some(answer) = self.turn(setup, cancel, |first| {
            orchestrator_prompt(&project, first, &asked)
        })?
        else {
            return Ok(());
        };
        let (text, plan) = tasks::parse_plan(&answer);
        self.store.add_message(conversation, ORCHESTRATOR, &text)?;
        match plan {
            Some(Ok(specs)) => {
                let specs = specs
                    .iter()
                    .map(serde_json::to_value)
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                let tasks = self.store.add_plan(conversation, &specs)?;
                self.store.append_event(
                    conversation,
                    None,
                    "plan_proposed",
                    &json!({"plan": tasks[0].plan_id, "tasks": tasks.iter().map(|task| json!({
                        "id": task.spec["id"], "title": task.spec["title"],
                        "kind": task.spec["kind"], "writes": task.spec["writes"],
                    })).collect::<Vec<_>>()}),
                )?;
            }
            Some(Err(error)) => {
                self.store.append_event(
                    conversation,
                    None,
                    "plan_invalid",
                    &json!({"error": error.to_string()}),
                )?;
            }
            None => {}
        }
        Ok(())
    }

    /// Runs one agent turn and records it. `Ok(None)` means the turn failed
    /// or was cancelled; that is recorded on the turn and blocks nothing.
    fn turn(
        &self,
        setup: TurnSetup<'_>,
        cancel: &AtomicBool,
        prompt: impl FnOnce(bool) -> String,
    ) -> Result<Option<String>> {
        let TurnSetup {
            conversation,
            role,
            profile,
            cwd,
            add_dirs,
            access,
            upto_message,
        } = setup;
        let provider = profile
            .as_ref()
            .map_or("unconfigured", |profile| agent::name(profile.provider));
        let resume = self.store.last_session(conversation, &role, provider)?;
        let turn = self
            .store
            .start_turn(conversation, &role, provider, upto_message)?;
        let announce = |kind: &str, payload: serde_json::Value| {
            let _ = self
                .store
                .append_event(conversation, Some(&turn.id), kind, &payload);
        };
        announce("turn_started", json!({"role": role, "provider": provider}));
        let outcome = profile.and_then(|profile| {
            let spec = agent::Spec {
                provider: profile.provider,
                binary: profile.binary,
                model: profile.model,
                effort: profile.native_effort,
                cwd,
                add_dirs,
                access,
                resume: resume.clone(),
            };
            agent::run(&spec, &prompt(resume.is_none()), cancel, &mut |event| {
                match &event {
                    agent::Event::Session { id } => {
                        let _ = self.store.set_session(&turn.id, id);
                    }
                    // Streamed fragments: the answer is recorded whole when
                    // the turn ends, so a row per fragment is only weight.
                    agent::Event::Text { .. } => return,
                    _ => {}
                }
                if let Ok(payload) = serde_json::to_value(&event) {
                    announce("agent", payload);
                }
            })
        });
        match outcome {
            Ok(outcome) => {
                self.store.finish_turn(&turn.id, TurnStatus::Done, None)?;
                announce("turn_finished", json!({"role": role, "status": "done"}));
                Ok(Some(outcome.text))
            }
            Err(error) => {
                let status = if cancel.load(Ordering::SeqCst) {
                    TurnStatus::Cancelled
                } else {
                    TurnStatus::Failed
                };
                let error = error.to_string();
                self.store.finish_turn(&turn.id, status, Some(&error))?;
                announce(
                    "turn_finished",
                    json!({"role": role, "status": status, "error": error}),
                );
                Ok(None)
            }
        }
    }

    fn announce_task(&self, conversation: &str, task: &Task) {
        let _ = self.store.append_event(
            conversation,
            None,
            "task",
            &json!({"id": task.spec["id"], "task": task.id, "title": task.spec["title"],
                    "status": task.status, "round": task.round}),
        );
    }

    fn set_status(&self, conversation: &str, task: &mut Task, status: &str) -> Result<()> {
        task.status = status.into();
        self.store.update_task(task)?;
        self.announce_task(conversation, task);
        Ok(())
    }

    /// Runs every task whose dependencies are approved, in waves, until none
    /// can run. Tasks behind a task that was not approved are skipped.
    fn run_plan(&self, conversation: &str, plan_id: &str, cancel: &AtomicBool) {
        loop {
            let Ok(mut tasks) = self.store.plan(plan_id) else {
                return;
            };
            let state: HashMap<String, String> = tasks
                .iter()
                .map(|task| {
                    (
                        task.spec["id"].as_str().unwrap_or_default().to_owned(),
                        task.status.clone(),
                    )
                })
                .collect();
            let mut ready = vec![];
            let mut settled = false;
            for task in &mut tasks {
                if task.status != status::QUEUED {
                    continue;
                }
                let depends: Vec<String> =
                    serde_json::from_value(task.spec["depends_on"].clone()).unwrap_or_default();
                let blocked = cancel.load(Ordering::SeqCst)
                    || depends.iter().any(|dependency| {
                        state.get(dependency).is_some_and(|status| {
                            status::finished(status) && status != status::APPROVED
                        })
                    });
                if blocked {
                    let next = if cancel.load(Ordering::SeqCst) {
                        status::CANCELLED
                    } else {
                        status::SKIPPED
                    };
                    let _ = self.set_status(conversation, task, next);
                    settled = true;
                } else if depends.iter().all(|dependency| {
                    state.get(dependency).map(String::as_str) == Some(status::APPROVED)
                }) {
                    ready.push(task.clone());
                }
            }
            if ready.is_empty() {
                // A skipped task can make its own dependants skippable.
                if settled {
                    continue;
                }
                return;
            }
            ready.truncate(PARALLEL_TASKS);
            std::thread::scope(|scope| {
                for task in ready {
                    scope.spawn(move || {
                        let mut task = task;
                        if let Err(error) = self.run_task(conversation, &mut task, cancel) {
                            task.report = Some(error.to_string());
                            let _ = self.set_status(conversation, &mut task, status::FAILED);
                        }
                    });
                }
            });
        }
    }

    /// Worker and verifier take turns until the verifier approves or the
    /// rounds run out; the worker resumes its own session with the findings.
    fn run_task(&self, conversation: &str, task: &mut Task, cancel: &AtomicBool) -> Result<()> {
        let spec: TaskSpec = serde_json::from_value(task.spec.clone())?;
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        // A writer works in its own worktree; everyone else in the project.
        let worktree = if spec.writes {
            require(
                git::is_repository(&project),
                "tasks that change files need the project to be a git repository",
            )?;
            let base = match &task.base {
                Some(base) => base.clone(),
                None => self.writer_base(&task.plan_id, &spec, &project)?,
            };
            let path = self.state.join("worktrees").join(&task.id);
            git::add_worktree(&project, &path, &branch(task), &base)?;
            task.base = Some(base);
            Some(path)
        } else {
            None
        };
        let workdir = worktree.clone().unwrap_or_else(|| project.clone());
        let project_text = workdir.to_string_lossy().into_owned();
        let access = if spec.writes {
            agent::Access::Write
        } else {
            agent::Access::ReadOnly
        };
        // Related directories outside the project (`../service` included)
        // are made readable; paths inside it already are.
        let add_dirs = spec
            .scope
            .iter()
            .filter_map(|path| project.join(path).canonicalize().ok())
            .map(|path| {
                if path.is_dir() {
                    path
                } else {
                    path.parent().map(Path::to_path_buf).unwrap_or(path)
                }
            })
            .filter(|path| !path.starts_with(&project))
            .collect::<Vec<_>>();
        let worker = format!("worker:{}", task.id);
        let verifier = format!("verifier:{}", task.id);
        let mut findings = vec![];
        for round in 1..=tasks::MAX_ROUNDS {
            task.round = round;
            self.set_status(conversation, task, status::WORKING)?;
            let setup = TurnSetup {
                conversation,
                role: worker.clone(),
                profile: self.config.resolve(spec.kind, RoleKind::Executor),
                cwd: workdir.clone(),
                add_dirs: add_dirs.clone(),
                access,
                upto_message: 0,
            };
            let report = self.turn(setup, cancel, |first| {
                if first {
                    tasks::worker_prompt(&project_text, &spec)
                } else {
                    tasks::worker_followup(&findings)
                }
            })?;
            let Some(report) = report else {
                return self.stop_task(conversation, task, &worker, cancel);
            };
            task.report = Some(report.clone());
            let changes = match (&worktree, &task.base) {
                (Some(path), Some(base)) => {
                    let result = git::commit_all(path, &format!("{} (round {round})", spec.title))?;
                    task.result = Some(result.clone());
                    Some(tasks::changes_text(
                        base,
                        &result,
                        &git::diff_stat(path, base, &result)?,
                    ))
                }
                _ => None,
            };
            self.set_status(conversation, task, status::VERIFYING)?;
            let setup = TurnSetup {
                conversation,
                role: verifier.clone(),
                profile: self.config.resolve(spec.kind, RoleKind::Verifier),
                cwd: workdir.clone(),
                add_dirs: add_dirs.clone(),
                access: agent::Access::ReadOnly,
                upto_message: 0,
            };
            let answer = self.turn(setup, cancel, |first| {
                if first {
                    tasks::verifier_prompt(&project_text, &spec, &report, changes.as_deref())
                } else {
                    tasks::verifier_followup(&report, changes.as_deref())
                }
            })?;
            let Some(answer) = answer else {
                return self.stop_task(conversation, task, &verifier, cancel);
            };
            let verdict = tasks::parse_verdict(&answer);
            task.verdict = Some(serde_json::to_value(&verdict)?);
            if verdict.approved {
                return match &worktree {
                    Some(path) => self.integrate(conversation, task, &spec, &project, path),
                    None => self.set_status(conversation, task, status::APPROVED),
                };
            }
            findings = verdict.findings;
        }
        self.set_status(conversation, task, status::NEEDS_USER)
    }

    /// A writer starts from its writing dependency's result, so it sees that
    /// work; otherwise from the project's current commit.
    fn writer_base(&self, plan_id: &str, spec: &TaskSpec, project: &Path) -> Result<String> {
        let results: Vec<String> = self
            .store
            .plan(plan_id)?
            .into_iter()
            .filter(|task| {
                spec.depends_on
                    .iter()
                    .any(|dependency| task.spec["id"] == dependency.as_str())
            })
            .filter_map(|task| task.result)
            .collect();
        match results.as_slice() {
            [single] => Ok(single.clone()),
            _ => git::head(project),
        }
    }

    /// Lands an approved writer's work on the user's branch as one change.
    /// Anything that would touch the user's own edits, or conflict, waits for
    /// the user with the work kept on its branch.
    fn integrate(
        &self,
        conversation: &str,
        task: &mut Task,
        spec: &TaskSpec,
        project: &Path,
        worktree: &Path,
    ) -> Result<()> {
        let base = task.base.clone().unwrap_or_default();
        let result = git::head(worktree)?;
        let note = |task: &mut Task, text: String| {
            let report = task.report.take().unwrap_or_default();
            task.report = Some(format!("{report}\n\n[integration] {text}"));
        };
        if result == base {
            note(task, "no file changes".into());
            git::remove_worktree(project, worktree, &branch(task))?;
            return self.set_status(conversation, task, status::APPROVED);
        }
        let policy = &self.config.policy;
        if !policy.local_write {
            note(
                task,
                format!(
                    "local writes are not allowed; the change is on branch {}",
                    branch(task)
                ),
            );
            return self.set_status(conversation, task, status::APPROVED);
        }
        let change = git::squash(worktree, &base, &spec.title)?;
        task.result = Some(change.clone());
        let outcome = {
            let _one_at_a_time = self.integrating.lock().expect("integration mutex");
            git::integrate(project, &base, &change, policy.local_commits)?
        };
        let (status, text) = match outcome {
            git::Integration::Committed(commit) => {
                (status::APPROVED, format!("committed as {commit}"))
            }
            git::Integration::Applied => (
                status::APPROVED,
                "applied to the working tree, not committed".into(),
            ),
            git::Integration::Conflict(files) => (
                status::NEEDS_USER,
                format!(
                    "conflicts with the current branch in {}; the change is on branch {}",
                    files.join(", "),
                    branch(task)
                ),
            ),
            git::Integration::UserEdits(files) => (
                status::NEEDS_USER,
                format!(
                    "you have uncommitted edits in {}; the change is on branch {}",
                    files.join(", "),
                    branch(task)
                ),
            ),
        };
        let _ = self.store.append_event(
            conversation,
            None,
            "integration",
            &json!({"id": spec.id, "title": spec.title, "status": status, "detail": text}),
        );
        note(task, text);
        if status == status::APPROVED {
            git::remove_worktree(project, worktree, &branch(task))?;
        }
        self.set_status(conversation, task, status)
    }

    /// Ends a task whose agent turn failed or was cancelled, keeping why.
    fn stop_task(
        &self,
        conversation: &str,
        task: &mut Task,
        role: &str,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let next = if cancel.load(Ordering::SeqCst) {
            status::CANCELLED
        } else {
            status::FAILED
        };
        let error = self
            .store
            .last_turn(conversation, role)?
            .and_then(|turn| turn.error)
            .unwrap_or_else(|| "no reason recorded".into());
        let who = role.split(':').next().unwrap_or(role);
        let report = task.report.take().unwrap_or_default();
        task.report = Some(
            format!("{report}\n\n[{next}] the {who}'s turn ended: {error}")
                .trim_start()
                .to_owned(),
        );
        self.set_status(conversation, task, next)
    }

    /// Hands the finished plan's results to the orchestrator, which then
    /// answers the user.
    fn report_plan(self: &Arc<Self>, conversation: &str, plan_id: &str) -> Result<()> {
        let tasks = self.store.plan(plan_id)?;
        let mut text = String::from(
            "The plan finished. Results of each task follow. Answer the user's \
             request using them; verify anything doubtful in the files yourself.\n",
        );
        for task in &tasks {
            let report = task.report.as_deref().unwrap_or("(no report)");
            let report = match report.char_indices().nth(REPORT_LIMIT) {
                Some((end, _)) => format!("{}\n[report truncated]", &report[..end]),
                None => report.to_owned(),
            };
            let findings = task
                .verdict
                .as_ref()
                .and_then(|verdict| serde_json::from_value::<tasks::Verdict>(verdict.clone()).ok())
                .map(|verdict| tasks::findings_text(&verdict.findings))
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| "none".into());
            text.push_str(&format!(
                "\n## {} — {} ({}, {} round(s))\nWorker report:\n{report}\nOpen verifier findings:\n{findings}\n",
                task.spec["id"].as_str().unwrap_or_default(),
                task.spec["title"].as_str().unwrap_or_default(),
                task.status,
                task.round
            ));
        }
        self.store.add_message(conversation, RUNTIME, &text)?;
        self.wake(conversation);
        Ok(())
    }
}

/// The branch a writer's worktree is on.
fn branch(task: &Task) -> String {
    format!("antislop/{}", &task.id[..8])
}

fn orchestrator_prompt(project: &Path, first: bool, messages: &[Message]) -> String {
    let mut prompt = String::new();
    if first {
        prompt.push_str(&format!(
            "You are the orchestrator of a team of coding agents. The project is your \
             current working directory: {}. Nothing from it is pasted here: read files \
             yourself with your tools whenever you need them, as much as the question \
             requires. Work read-only; do not modify anything. Answer in the user's \
             language, concretely, citing files where it helps.\n\n{}\n\n",
            project.display(),
            tasks::orchestrator_instructions()
        ));
    }
    let texts: Vec<String> = messages
        .iter()
        .map(|message| match message.author.as_str() {
            RUNTIME => format!("[runtime]\n{}", message.text),
            _ => message.text.clone(),
        })
        .collect();
    prompt.push_str(&texts.join("\n\n"));
    prompt
}
