//! Lean core: native CLI agents working on the project's own files, small
//! rows of state and an event log. See docs/lean-core.md.

pub mod agent;
pub mod cli;
pub mod daemon;
pub mod diff;
pub mod export;
pub mod git;
pub mod store;
pub mod tasks;
pub mod tui;

use crate::{
    Result,
    config::{Config, ResolvedProfile, RoleKind, WorkKind, WorkspaceMode},
    require,
};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use store::{Comment, Conversation, Mail, Message, Store, Task, TurnStatus};
use tasks::{TaskSpec, Verdict, status};

const ORCHESTRATOR: &str = "orchestrator";
const RUNTIME: &str = "runtime";
/// Author of a code review's findings, and how its message starts.
const CODE_REVIEW: &str = "code-review";
const CODE_REVIEW_PREFIX: &str = "[code review";
/// Largest patch the diff view gets.
const DIFF_LIMIT: usize = 2 * 1024 * 1024;
/// How a comment's message starts, before its id.
pub const COMMENT_PREFIX: &str = "[user comment #";
/// Checks each plan before the user sees it.
const PLAN_VERIFIER: &str = "plan-verifier";
/// Checks, after a plan ran, that the user's request was fully delivered.
const AUDITOR: &str = "auditor";
/// Extra worker turns, per round, for messages that arrived while it worked.
const MAIL_TURNS: usize = 2;
/// Longest worker report handed back to the orchestrator per task.
const REPORT_LIMIT: usize = 40_000;

pub struct Core {
    pub store: Store,
    /// One process owns a state directory at a time.
    _lock: std::fs::File,
    config: Config,
    /// Private state directory; writers' worktrees live under it.
    state: PathBuf,
    /// Writers land on the user's branch one at a time.
    integrating: Mutex<()>,
    /// In-place writers work in the user's directory one at a time.
    in_place: Mutex<()>,
    /// Conversations with an orchestrator loop running, and its cancel flag.
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Conversations with a plan running, and its cancel flag.
    plans: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Conversations with a code review the user asked for, and its flag.
    reviews: Mutex<HashMap<String, Arc<AtomicBool>>>,
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
        daemon::private_directory(state)?;
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
        let cut_off = store.interrupt_running_turns()?;
        let interrupted = store.interrupt_unfinished_tasks()?;
        let core = Arc::new(Self {
            store,
            _lock: lock,
            config,
            state: state.canonicalize()?,
            integrating: Mutex::new(()),
            in_place: Mutex::new(()),
            running: Mutex::new(HashMap::new()),
            plans: Mutex::new(HashMap::new()),
            reviews: Mutex::new(HashMap::new()),
        });
        // Pick up where the previous process stopped: the orchestrator answers
        // what it was answering, and an autonomous conversation resumes its
        // plan through the agents' own sessions.
        for conversation in cut_off {
            core.wake(&conversation);
        }
        if core.config.policy.automatic_progression() {
            for (conversation, plan) in interrupted {
                let _ = core.resume_plan(&conversation, &plan);
            }
        }
        Ok(core)
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

    /// A message from the user: `@t2 text` goes to that task's worker (with
    /// a copy for the orchestrator), anything else to the orchestrator.
    pub fn submit(self: &Arc<Self>, conversation: &str, text: &str) -> Result<serde_json::Value> {
        if let Some((to, body)) = text
            .strip_prefix('@')
            .and_then(|rest| rest.split_once(char::is_whitespace))
            && let Some(task) = self
                .store
                .latest_plan(conversation)?
                .into_iter()
                .find(|task| task.spec["id"] == to)
        {
            let body = body.trim();
            require(!body.is_empty(), "the message is empty")?;
            let mail = self.store.add_mail(
                conversation,
                &format!("worker:{}", task.id),
                &Mail {
                    id: 0,
                    sender: "user".into(),
                    recipient: to.into(),
                    kind: "user_instruction".into(),
                    text: body.into(),
                },
            )?;
            self.store.add_mail(
                conversation,
                ORCHESTRATOR,
                &Mail {
                    id: 0,
                    sender: "user".into(),
                    recipient: ORCHESTRATOR.into(),
                    kind: "copy".into(),
                    text: format!("(the user wrote to {to}) {body}"),
                },
            )?;
            return Ok(
                json!({"mail": mail.id, "recipient_finished": status::finished(&task.status)}),
            );
        }
        Ok(json!(self.send(conversation, text)?.seq))
    }

    /// The user's comment on one element: it reaches the orchestrator as a
    /// message now and stays in its prompts until the user resolves it.
    pub fn comment(
        self: &Arc<Self>,
        conversation: &str,
        target: &str,
        text: &str,
    ) -> Result<Comment> {
        self.store.conversation(conversation)?;
        let text = text.trim();
        require(!text.is_empty(), "the comment is empty")?;
        require(!target.trim().is_empty(), "the comment has no element")?;
        let plan = self.store.latest_plan(conversation)?;
        let label = comment_label(target, &plan);
        let comment = self.store.add_comment(
            conversation,
            &Comment {
                id: 0,
                plan_id: plan.first().map(|task| task.plan_id.clone()),
                target: target.into(),
                label: label.clone(),
                text: text.into(),
                status: "open".into(),
                message: 0,
            },
            |id| format!("{COMMENT_PREFIX}{id} on {label}] {text}"),
        )?;
        self.wake(conversation);
        Ok(comment)
    }

    /// Resolves, reopens or deletes a comment; only the user does this.
    pub fn set_comment(&self, conversation: &str, id: u64, status: &str) -> Result<Comment> {
        require(
            matches!(status, "open" | "resolved" | "deleted"),
            "a comment is open, resolved or deleted",
        )?;
        self.store.set_comment_status(conversation, id, status)
    }

    /// Stops the orchestrator's turn and any running plan; queued messages stay.
    /// Pausing starts no new plan work (worker, verifier and audit turns):
    /// turns already running finish, the orchestrator still answers, and
    /// everything waiting goes on when the user continues. It survives a
    /// restart.
    pub fn pause(&self, conversation: &str, paused: bool) -> Result<()> {
        self.store.conversation(conversation)?;
        self.store.set_paused(conversation, paused)
    }

    pub fn cancel(&self, conversation: &str) {
        for flags in [&self.running, &self.plans, &self.reviews] {
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
            && !self
                .reviews
                .lock()
                .expect("reviews mutex")
                .contains_key(conversation)
    }

    /// Whether any conversation has an agent or a plan running.
    pub fn busy(&self) -> bool {
        !self.running.lock().expect("running mutex").is_empty()
            || !self.plans.lock().expect("plans mutex").is_empty()
            || !self.reviews.lock().expect("reviews mutex").is_empty()
    }

    /// One task of a plan, by the id the orchestrator gave it.
    pub fn task(&self, plan: &str, id: &str) -> Result<Task> {
        self.store
            .plan(plan)?
            .into_iter()
            .find(|task| task.spec["id"] == id)
            .ok_or_else(|| crate::Error::Invalid(format!("no task {id} in plan {plan}")))
    }

    /// Where a writer's change can be read: its worktree while it works (the
    /// working tree against its base), else the project's repository between
    /// its base and result commits.
    fn change(&self, conversation: &str, task: &Task) -> Result<(PathBuf, String, Option<String>)> {
        let id = task.spec["id"].as_str().unwrap_or_default();
        let base = task.base.clone().ok_or_else(|| {
            crate::Error::Invalid(format!(
                "task {id} has no change: it does not write files or has not started"
            ))
        })?;
        let worktree = self.state.join("worktrees").join(&task.id);
        if worktree.exists() {
            return Ok((worktree, base, None));
        }
        let result = task.result.clone().ok_or_else(|| {
            crate::Error::Invalid(format!("task {id} has no committed change yet"))
        })?;
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        Ok((project, base, Some(result)))
    }

    /// A writer's change as files with per-file counts and numbered lines.
    pub fn diff(&self, conversation: &str, plan: &str, id: &str) -> Result<serde_json::Value> {
        let task = self.task(plan, id)?;
        let (dir, base, result) = self.change(conversation, &task)?;
        let mut patch = git::patch(&dir, &base, result.as_deref())?;
        let truncated = patch.len() > DIFF_LIMIT;
        if truncated {
            let mut end = DIFF_LIMIT;
            while !patch.is_char_boundary(end) {
                end -= 1;
            }
            patch.truncate(end);
        }
        Ok(json!({
            "task": id,
            "base": base,
            "head": result,
            "files": diff::parse(&patch),
            "truncated": truncated,
        }))
    }

    /// Reviews a writer's change on the user's request, with the task's
    /// verifier profile in a session of its own. The findings go to the
    /// orchestrator; the review never changes the task's status.
    pub fn review_change(self: &Arc<Self>, conversation: &str, plan: &str, id: &str) -> Result<()> {
        let task = self.task(plan, id)?;
        let spec: TaskSpec = serde_json::from_value(task.spec.clone())?;
        let (dir, base, result) = self.change(conversation, &task)?;
        let cancel = {
            let mut reviews = self.reviews.lock().expect("reviews mutex");
            require(
                !reviews.contains_key(conversation),
                "a code review is already running",
            )?;
            let cancel = Arc::new(AtomicBool::new(false));
            reviews.insert(conversation.to_owned(), cancel.clone());
            cancel
        };
        let comments: Vec<Comment> = self
            .store
            .comments(conversation, "open")?
            .into_iter()
            .filter(|comment| {
                comment.plan_id.as_deref() == Some(plan)
                    && comment.target.starts_with(&format!("diff:{id}"))
            })
            .collect();
        self.store.append_event(
            conversation,
            None,
            "code_review",
            &json!({"id": id, "status": "running"}),
        )?;
        let core = self.clone();
        let conversation = conversation.to_owned();
        std::thread::spawn(move || {
            let role = format!("code-review:{}", task.id);
            let setup = TurnSetup {
                conversation: &conversation,
                role: role.clone(),
                profile: core.config.resolve(spec.kind, RoleKind::Verifier),
                cwd: dir.clone(),
                add_dirs: vec![],
                access: agent::Access::ReadOnly,
                upto_message: 0,
            };
            let outcome = core.turn(setup, &cancel, |_| {
                tasks::code_review_prompt(
                    &dir.to_string_lossy(),
                    &spec,
                    &base,
                    result.as_deref(),
                    &comments,
                )
            });
            let review = match outcome {
                Ok(Some(answer)) => Ok(tasks::parse_verdict(&answer)),
                Ok(None) => Err(core
                    .store
                    .last_turn(&conversation, &role)
                    .ok()
                    .flatten()
                    .and_then(|turn| turn.error)
                    .unwrap_or_else(|| "no reason recorded".into())),
                Err(error) => Err(error.to_string()),
            };
            let mut payload = review_json(&review);
            payload["id"] = json!(spec.id);
            payload["status"] = json!("done");
            let _ = core
                .store
                .append_event(&conversation, None, "code_review", &payload);
            if let Ok(verdict) = &review {
                let text = format!(
                    "{CODE_REVIEW_PREFIX} of task {} \"{}\", requested by the user]\n{}",
                    spec.id,
                    spec.title,
                    if verdict.findings.is_empty() {
                        "No findings.".to_owned()
                    } else {
                        tasks::findings_text(&verdict.findings)
                    }
                );
                let _ = core.store.add_message(&conversation, CODE_REVIEW, &text);
            }
            core.reviews
                .lock()
                .expect("reviews mutex")
                .remove(&conversation);
            core.wake(&conversation);
        });
        Ok(())
    }

    /// Starts the proposed plan `plan_id`: exactly the one the user saw, even
    /// if the orchestrator has proposed another since.
    pub fn accept_plan(self: &Arc<Self>, conversation: &str, plan_id: &str) -> Result<Vec<Task>> {
        self.start_plan(conversation, plan_id, |store| {
            store.queue_plan(conversation, plan_id).map(|_| ())
        })
    }

    /// Runs again every task of `plan_id` that ended without approval
    /// (interrupted, failed, waiting for a login, needing the user, skipped).
    /// Agents resume their own sessions with what they already did; a change
    /// that was approved but could not be integrated is integrated again.
    pub fn resume_plan(self: &Arc<Self>, conversation: &str, plan_id: &str) -> Result<Vec<Task>> {
        self.start_plan(conversation, plan_id, |store| {
            require(
                store.requeue_plan(conversation, plan_id)? > 0,
                "this plan has nothing to resume",
            )
        })
    }

    fn start_plan(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        queue: impl FnOnce(&Store) -> Result<()>,
    ) -> Result<Vec<Task>> {
        let mut plans = self.plans.lock().expect("plans mutex");
        require(
            !plans.contains_key(conversation),
            "a plan is already running",
        )?;
        queue(&self.store)?;
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
            // The audit still counts as the plan running: it can be cancelled
            // and keeps the daemon busy.
            let report = core.plan_report(&conversation, &plan_id, &cancel);
            // The orchestrator starts on the results before the plan stops
            // counting as running, so the conversation never looks idle in
            // between; a plan it proposes waits for this lock.
            let mut plans = core.plans.lock().expect("plans mutex");
            if let Ok(report) = report
                && core
                    .store
                    .add_message(&conversation, RUNTIME, &report)
                    .is_ok()
            {
                core.wake(&conversation);
            }
            plans.remove(&conversation);
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

    fn orchestrate(self: &Arc<Self>, conversation: &str, cancel: &AtomicBool) {
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
        // A failed turn counts as handled (the next message asks again); one
        // cut off by a restart does not.
        let after = match self.store.last_turn(conversation, ORCHESTRATOR)? {
            Some(turn) if turn.status == TurnStatus::Interrupted => {
                self.store.last_answered(conversation, ORCHESTRATOR)?
            }
            Some(turn) => turn.upto_message,
            None => 0,
        };
        Ok(self
            .store
            .messages_after(conversation, after)?
            .into_iter()
            .filter(|message| message.author != ORCHESTRATOR)
            .map(|message| message.seq)
            .next_back())
    }

    fn orchestrator_turn(
        self: &Arc<Self>,
        conversation: &str,
        upto: u64,
        cancel: &AtomicBool,
    ) -> Result<()> {
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
        // Comments the user has not resolved stay in view; the ones asked in
        // this turn already are.
        let open: Vec<Comment> = self
            .store
            .comments(conversation, "open")?
            .into_iter()
            .filter(|comment| comment.message <= answered)
            .collect();
        let Some(answer) = self.turn(setup, cancel, |first| {
            orchestrator_prompt(&project, first, &asked, &open)
        })?
        else {
            return Ok(());
        };
        let latest = self.store.latest_plan(conversation)?;
        let answer = self.post(
            conversation,
            latest.first().map(|task| task.plan_id.as_str()),
            ORCHESTRATOR,
            &answer,
        )?;
        let (text, plan) = tasks::parse_plan(&answer);
        self.store.add_message(conversation, ORCHESTRATOR, &text)?;
        match plan {
            Some(Ok(specs)) => self.propose(conversation, &project, &specs, cancel)?,
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

    /// Stores the plan and has it checked. A plan the verifier sends back
    /// goes back to the orchestrator (a bounded number of times); otherwise it
    /// is shown to the user with the verdict, and in autonomous mode a checked
    /// plan starts at once.
    fn propose(
        self: &Arc<Self>,
        conversation: &str,
        project: &Path,
        specs: &[TaskSpec],
        cancel: &AtomicBool,
    ) -> Result<()> {
        let values = specs
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let tasks = self.store.add_plan(conversation, &values)?;
        let plan_id = tasks[0].plan_id.clone();
        let project_text = project.to_string_lossy().into_owned();
        let review = self.review(
            conversation,
            PLAN_VERIFIER,
            project,
            cancel,
            |first, request| {
                if first {
                    tasks::plan_review_prompt(&project_text, request, specs)
                } else {
                    tasks::plan_review_followup(request, specs)
                }
            },
        )?;
        if cancel.load(Ordering::SeqCst) {
            self.store
                .set_plan_status(&plan_id, status::PROPOSED, status::CANCELLED)?;
            return Ok(());
        }
        if let Ok(verdict) = &review
            && !verdict.approved
            && self.store.count_since_user(conversation, PLAN_VERIFIER)?
                < tasks::MAX_ROUNDS as usize
        {
            self.store
                .set_plan_status(&plan_id, status::PROPOSED, status::REVISED)?;
            self.store.append_event(
                conversation,
                None,
                "plan_review",
                &json!({"plan": plan_id, "approved": false, "findings": verdict.findings}),
            )?;
            // The orchestrator's loop answers this message next.
            self.store.add_message(
                conversation,
                PLAN_VERIFIER,
                &tasks::plan_rejected(&verdict.findings),
            )?;
            return Ok(());
        }
        let checked = review.as_ref().is_ok_and(|verdict| verdict.approved);
        let waiting = if !self.config.policy.automatic_progression() {
            None
        } else if !checked {
            Some("the plan verifier did not approve this plan")
        } else if self.store.count_since_user(conversation, RUNTIME)? > 0
            && self.repeats_a_plan_that_ran(conversation, &plan_id, specs)?
        {
            Some("the same plan already ran without you and did not close what was found")
        } else {
            None
        };
        self.store.append_event(
            conversation,
            None,
            "plan_proposed",
            &json!({
                "plan": plan_id,
                "tasks": tasks.iter().map(|task| task.spec.clone()).collect::<Vec<_>>(),
                "review": review_json(&review),
                "automatic": self.config.policy.automatic_progression() && waiting.is_none(),
                "waiting": waiting,
            }),
        )?;
        if self.config.policy.automatic_progression() && waiting.is_none() {
            self.accept_plan(conversation, &plan_id)?;
        }
        Ok(())
    }

    /// An independent reviewer's verdict (a verifier profile, its own
    /// session per role), or why there is none. `prompt` gets whether this is
    /// the session's first turn and the user's messages it has not seen.
    fn review(
        &self,
        conversation: &str,
        role: &str,
        project: &Path,
        cancel: &AtomicBool,
        prompt: impl FnOnce(bool, &str) -> String,
    ) -> Result<std::result::Result<Verdict, String>> {
        let answered = self.store.last_answered(conversation, role)?;
        let all = self.store.messages_by(conversation, "user", 0)?;
        let upto = all.last().map_or(0, |message| message.seq);
        let request = |after: u64| {
            all.iter()
                .filter(|message| message.seq > after)
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        let setup = TurnSetup {
            conversation,
            role: role.into(),
            profile: self
                .config
                .resolve(WorkKind::Orchestration, RoleKind::Verifier),
            cwd: project.to_path_buf(),
            add_dirs: vec![],
            access: agent::Access::ReadOnly,
            upto_message: upto,
        };
        let answer = self.turn(setup, cancel, |first| {
            prompt(first, &request(if first { 0 } else { answered }))
        })?;
        Ok(match answer {
            Some(answer) => Ok(tasks::parse_verdict(&answer)),
            None => Err(self
                .store
                .last_turn(conversation, role)?
                .and_then(|turn| turn.error)
                .unwrap_or_else(|| "no reason recorded".into())),
        })
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
        if role != ORCHESTRATOR && role != PLAN_VERIFIER {
            // Cancelling a paused conversation ends what waits, too.
            while !cancel.load(Ordering::SeqCst) && self.store.paused(conversation)? {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
        let provider = profile
            .as_ref()
            .map_or("unconfigured", |profile| agent::name(profile.provider));
        let resume = self.store.last_session(conversation, &role, provider)?;
        let inbox = self.store.pending_mail(conversation, &role)?;
        let turn = self
            .store
            .start_turn(conversation, &role, provider, upto_message)?;
        let announce = |kind: &str, payload: serde_json::Value| {
            let _ = self
                .store
                .append_event(conversation, Some(&turn.id), kind, &payload);
        };
        // What ran, for the history and its export (design §15).
        announce(
            "turn_started",
            json!({
                "role": role,
                "provider": provider,
                "model": profile.as_ref().ok().and_then(|profile| profile.model.clone()),
                "effort": profile.as_ref().ok().and_then(|profile| profile.native_effort.clone()),
            }),
        );
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
            let mut text = if inbox.is_empty() {
                String::new()
            } else {
                tasks::inbox_text(&inbox)
            };
            text.push_str(&prompt(resume.is_none()));
            agent::run(&spec, &text, cancel, &mut |event| {
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
                let delivered: Vec<u64> = inbox.iter().map(|mail| mail.id).collect();
                self.store
                    .mark_delivered(conversation, &delivered, &turn.id)?;
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
        let in_place = spec.writes && self.config.workspace.mode == WorkspaceMode::InPlace;
        let autonomous = self.config.policy.automatic_progression();
        // In place, writers take turns in the user's directory.
        let _writing = if in_place {
            require(
                git::is_repository(&project),
                "tasks that change files need the project to be a git repository",
            )?;
            require(
                self.config.policy.local_write,
                "in-place writers need policy.local_write",
            )?;
            loop {
                match self.in_place.try_lock() {
                    Ok(guard) => break Some(guard),
                    // A writer that panicked left nothing half-locked.
                    Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                        break Some(poisoned.into_inner());
                    }
                    Err(std::sync::TryLockError::WouldBlock) => {}
                }
                if cancel.load(Ordering::SeqCst) {
                    return self.set_status(conversation, task, status::CANCELLED);
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        } else {
            None
        };
        if in_place && task.base.is_none() {
            task.base = Some(git::snapshot(&project, &format!("before {}", spec.title))?);
        }
        // What the user had not committed when the writer started: theirs,
        // not the writer's to change.
        let user_edits = match (&task.base, in_place) {
            (Some(base), true) => git::changed_files(&project, &format!("{base}^"), base)?,
            _ => vec![],
        };
        // A writer works in its own worktree (in place: in the project);
        // everyone else in the project.
        let worktree = if spec.writes && !in_place {
            require(
                git::is_repository(&project),
                "tasks that change files need the project to be a git repository",
            )?;
            let base = match &task.base {
                Some(base) => base.clone(),
                None => self.writer_base(&task.plan_id, &spec, &project)?,
            };
            let path = self.state.join("worktrees").join(&task.id);
            // A resumed writer whose worktree is gone continues from its work.
            let start = task.result.clone().unwrap_or_else(|| base.clone());
            git::add_worktree(&project, &path, &branch(task), &start)?;
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
        // are made readable; paths inside it already are. A writer's CLI
        // would make them writable too, so a writer only reads them where
        // its tools allow: a plan never widens where an agent may write.
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
        let team: Vec<TaskSpec> = self
            .store
            .plan(&task.plan_id)?
            .into_iter()
            .filter_map(|task| serde_json::from_value(task.spec).ok())
            .collect();
        let worker = format!("worker:{}", task.id);
        let verifier = format!("verifier:{}", task.id);
        // A resumed task keeps its rounds and its verifier's last word.
        let verdict = task
            .verdict
            .as_ref()
            .and_then(|verdict| serde_json::from_value::<Verdict>(verdict.clone()).ok());
        if let Some(verdict) = &verdict
            && verdict.approved
        {
            if let Some(path) = &worktree {
                return self.integrate(conversation, task, &spec, &project, path);
            }
            if in_place {
                return self.finish_in_place(conversation, task, &spec, &project);
            }
        }
        let mut findings = verdict.map(|verdict| verdict.findings).unwrap_or_default();
        let resumed = task.round > 0;
        let first_round = task.round + 1;
        // Supervised: a few rounds run on their own, then the user decides.
        // Autonomous: rounds go on until approval or until the verifier
        // repeats its findings, which means the worker is not getting closer.
        let last_round = if autonomous {
            u32::MAX
        } else {
            first_round + tasks::MAX_ROUNDS - 1
        };
        let mut previous: Option<String> = None;
        for round in first_round..=last_round {
            task.round = round;
            self.set_status(conversation, task, status::WORKING)?;
            let worker_setup = || TurnSetup {
                conversation,
                role: worker.clone(),
                profile: self.config.resolve(spec.kind, RoleKind::Executor),
                cwd: workdir.clone(),
                add_dirs: if spec.writes {
                    vec![]
                } else {
                    add_dirs.clone()
                },
                access,
                upto_message: 0,
            };
            let report = self.turn(worker_setup(), cancel, |first| {
                if first {
                    tasks::worker_prompt(&project_text, &spec, &team, autonomous)
                        + &tasks::user_edits_note(&user_edits)
                } else if resumed && round == first_round {
                    tasks::worker_resume(&findings)
                } else {
                    tasks::worker_followup(&findings)
                }
            })?;
            let Some(report) = report else {
                return self.stop_task(conversation, task, &worker, cancel);
            };
            let mut report = self.post(conversation, Some(&task.plan_id), &spec.id, &report)?;
            // Messages that arrived while it worked get a turn of their own.
            for _ in 0..MAIL_TURNS {
                if self.store.pending_mail(conversation, &worker)?.is_empty() {
                    break;
                }
                let next = self.turn(worker_setup(), cancel, |first| {
                    if first {
                        tasks::worker_prompt(&project_text, &spec, &team, autonomous)
                    } else {
                        tasks::INBOX_FOLLOWUP.into()
                    }
                })?;
                let Some(next) = next else {
                    return self.stop_task(conversation, task, &worker, cancel);
                };
                report = self.post(conversation, Some(&task.plan_id), &spec.id, &next)?;
            }
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
                (None, Some(base)) if in_place => {
                    let result =
                        git::snapshot(&project, &format!("{} (round {round})", spec.title))?;
                    task.result = Some(result.clone());
                    let touched: Vec<String> = git::changed_files(&project, base, &result)?
                        .into_iter()
                        .filter(|file| user_edits.contains(file))
                        .collect();
                    if !touched.is_empty() {
                        let text = format!(
                            "the worker changed files you were editing: {}. Your version is kept: \
                             `git checkout {base} -- <file>` restores it",
                            touched.join(", ")
                        );
                        let _ = self.store.append_event(
                            conversation,
                            None,
                            "integration",
                            &json!({"id": spec.id, "title": spec.title,
                                    "status": status::NEEDS_USER, "detail": text}),
                        );
                        task.report = Some(format!("{report}\n\n[in place] {text}"));
                        return self.set_status(conversation, task, status::NEEDS_USER);
                    }
                    Some(tasks::changes_text(
                        base,
                        &result,
                        &git::diff_stat(&project, base, &result)?,
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
                    tasks::verifier_followup(&spec, &report, changes.as_deref())
                }
            })?;
            let Some(answer) = answer else {
                return self.stop_task(conversation, task, &verifier, cancel);
            };
            let answer = self.post(
                conversation,
                Some(&task.plan_id),
                &format!("{}/verifier", spec.id),
                &answer,
            )?;
            let mut verdict = tasks::parse_verdict(&answer);
            let obligations = tasks::obligations(&spec);
            let missing = tasks::unaccounted(&verdict, &obligations);
            if !missing.is_empty() {
                // A verdict that skips an obligation does not count; the
                // verifier gets one turn to account for it.
                let prompt = tasks::coverage_followup(&missing);
                let setup = TurnSetup {
                    conversation,
                    role: verifier.clone(),
                    profile: self.config.resolve(spec.kind, RoleKind::Verifier),
                    cwd: workdir.clone(),
                    add_dirs: add_dirs.clone(),
                    access: agent::Access::ReadOnly,
                    upto_message: 0,
                };
                let Some(answer) = self.turn(setup, cancel, |_| prompt)? else {
                    return self.stop_task(conversation, task, &verifier, cancel);
                };
                let answer = self.post(
                    conversation,
                    Some(&task.plan_id),
                    &format!("{}/verifier", spec.id),
                    &answer,
                )?;
                let earlier = std::mem::replace(&mut verdict, tasks::parse_verdict(&answer));
                for (id, entry) in earlier.coverage {
                    verdict.coverage.entry(id).or_insert(entry);
                }
            }
            let incomplete = !tasks::unaccounted(&verdict, &obligations).is_empty();
            tasks::settle(&mut verdict, &obligations);
            task.verdict = Some(serde_json::to_value(&verdict)?);
            if incomplete {
                // The worker cannot fix a verifier that skips its checks.
                return self.set_status(conversation, task, status::NEEDS_USER);
            }
            if verdict.approved {
                return match &worktree {
                    Some(path) => self.integrate(conversation, task, &spec, &project, path),
                    None if in_place => self.finish_in_place(conversation, task, &spec, &project),
                    None => self.set_status(conversation, task, status::APPROVED),
                };
            }
            let text = tasks::findings_text(&verdict.findings);
            if previous.as_deref() == Some(text.as_str()) {
                let report = task.report.take().unwrap_or_default();
                task.report = Some(format!(
                    "{report}\n\n[runtime] the verifier repeated the same findings; \
                     the worker is not getting closer"
                ));
                return self.set_status(conversation, task, status::NEEDS_USER);
            }
            previous = Some(text);
            findings = verdict.findings;
        }
        self.set_status(conversation, task, status::NEEDS_USER)
    }

    /// Whether `specs` asks for the same work as the last plan of the
    /// conversation that ran (ids aside).
    fn repeats_a_plan_that_ran(
        &self,
        conversation: &str,
        plan_id: &str,
        specs: &[TaskSpec],
    ) -> Result<bool> {
        let work = |spec: &TaskSpec| {
            (
                spec.title.clone(),
                spec.goal.clone(),
                spec.kind,
                spec.writes,
            )
        };
        let Some(earlier) = self.store.last_plan_that_ran(conversation, plan_id)? else {
            return Ok(false);
        };
        let mut before: Vec<_> = earlier
            .iter()
            .filter_map(|task| serde_json::from_value::<TaskSpec>(task.spec.clone()).ok())
            .map(|spec| work(&spec))
            .collect();
        let mut now: Vec<_> = specs.iter().map(work).collect();
        before.sort();
        now.sort();
        Ok(before == now)
    }

    /// Sends the ```message blocks of `answer` and returns the answer
    /// without them.
    fn post(
        &self,
        conversation: &str,
        plan_id: Option<&str>,
        sender: &str,
        answer: &str,
    ) -> Result<String> {
        let (text, outgoing) = tasks::parse_messages(answer);
        for message in outgoing {
            self.deliver(conversation, plan_id, sender, message)?;
        }
        Ok(text)
    }

    /// Queues a message for its recipients: the orchestrator, one task's
    /// worker, or (`all`) the workers of every unfinished task but the
    /// sender's.
    fn deliver(
        &self,
        conversation: &str,
        plan_id: Option<&str>,
        sender: &str,
        message: tasks::Outgoing,
    ) -> Result<()> {
        let plan = match plan_id {
            Some(plan_id) => self.store.plan(plan_id)?,
            None => vec![],
        };
        let own_task = sender.split('/').next().unwrap_or(sender);
        let targets: Vec<(String, String)> = match message.to.as_str() {
            ORCHESTRATOR => vec![(ORCHESTRATOR.into(), ORCHESTRATOR.into())],
            "all" => plan
                .iter()
                .filter(|task| !status::finished(&task.status) && task.spec["id"] != own_task)
                .map(|task| {
                    (
                        task.spec["id"].as_str().unwrap_or_default().to_owned(),
                        format!("worker:{}", task.id),
                    )
                })
                .collect(),
            to => plan
                .iter()
                .filter(|task| task.spec["id"] == to)
                .map(|task| (to.to_owned(), format!("worker:{}", task.id)))
                .collect(),
        };
        if targets.is_empty() {
            self.store.append_event(
                conversation,
                None,
                "mail_rejected",
                &json!({"sender": sender, "recipient": message.to, "text": message.text,
                        "reason": "no such recipient in the current plan"}),
            )?;
        }
        for (label, role) in targets {
            self.store.add_mail(
                conversation,
                &role,
                &Mail {
                    id: 0,
                    sender: sender.into(),
                    recipient: label,
                    kind: message.kind.clone(),
                    text: message.text.clone(),
                },
            )?;
        }
        Ok(())
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

    /// An approved in-place change is already in the project; with commit
    /// permission exactly its files are committed.
    fn finish_in_place(
        &self,
        conversation: &str,
        task: &mut Task,
        spec: &TaskSpec,
        project: &Path,
    ) -> Result<()> {
        let (Some(base), Some(result)) = (task.base.clone(), task.result.clone()) else {
            return self.set_status(conversation, task, status::APPROVED);
        };
        let files = git::changed_files(project, &base, &result)?;
        let text = if files.is_empty() {
            "no file changes".to_owned()
        } else if self.config.policy.local_commits {
            let _one_at_a_time = self.integrating.lock().expect("integration mutex");
            format!(
                "committed in place as {}",
                git::commit_paths(project, &files, &spec.title)?
            )
        } else {
            format!("applied in place, not committed: {}", files.join(", "))
        };
        let _ = self.store.append_event(
            conversation,
            None,
            "integration",
            &json!({"id": spec.id, "title": spec.title, "status": status::APPROVED, "detail": text}),
        );
        let report = task.report.take().unwrap_or_default();
        task.report = Some(format!("{report}\n\n[in place] {text}"));
        self.set_status(conversation, task, status::APPROVED)
    }

    /// Ends a task whose agent turn failed or was cancelled, keeping why.
    fn stop_task(
        &self,
        conversation: &str,
        task: &mut Task,
        role: &str,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let error = self
            .store
            .last_turn(conversation, role)?
            .and_then(|turn| turn.error)
            .unwrap_or_else(|| "no reason recorded".into());
        let next = if cancel.load(Ordering::SeqCst) {
            status::CANCELLED
        } else if tasks::needs_user_action(&error) {
            status::WAITING
        } else {
            status::FAILED
        };
        let who = role.split(':').next().unwrap_or(role);
        let report = task.report.take().unwrap_or_default();
        task.report = Some(
            format!("{report}\n\n[{next}] the {who}'s turn ended: {error}")
                .trim_start()
                .to_owned(),
        );
        self.set_status(conversation, task, next)
    }

    /// The finished plan's results, and an independent audit of whether they
    /// deliver the user's request, for the orchestrator to answer the user.
    fn plan_report(
        &self,
        conversation: &str,
        plan_id: &str,
        cancel: &AtomicBool,
    ) -> Result<String> {
        let tasks = self.store.plan(plan_id)?;
        let mut results = String::new();
        for task in &tasks {
            let report = task.report.as_deref().unwrap_or("(no report)");
            let report = match report.char_indices().nth(REPORT_LIMIT) {
                Some((end, _)) => format!("{}\n[report truncated]", &report[..end]),
                None => report.to_owned(),
            };
            let findings = task
                .verdict
                .as_ref()
                .and_then(|verdict| serde_json::from_value::<Verdict>(verdict.clone()).ok())
                .map(|verdict| tasks::findings_text(&verdict.findings))
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| "none".into());
            results.push_str(&format!(
                "\n## {} — {} ({}, {} round(s))\nGoal: {}\nWorker report:\n{report}\nOpen verifier findings:\n{findings}\n",
                task.spec["id"].as_str().unwrap_or_default(),
                task.spec["title"].as_str().unwrap_or_default(),
                task.status,
                task.round,
                task.spec["goal"].as_str().unwrap_or_default(),
            ));
        }
        let mut text = format!(
            "The plan finished. Results of each task follow. Answer the user's \
             request using them; verify anything doubtful in the files yourself.\n{results}"
        );
        if !cancel.load(Ordering::SeqCst) {
            let project = PathBuf::from(self.store.conversation(conversation)?.project);
            let project_text = project.to_string_lossy().into_owned();
            let audit =
                self.review(conversation, AUDITOR, &project, cancel, |first, request| {
                    if first {
                        tasks::audit_prompt(&project_text, request, &results)
                    } else {
                        tasks::audit_followup(request, &results)
                    }
                })?;
            let mut payload = review_json(&audit);
            payload["plan"] = json!(plan_id);
            self.store
                .append_event(conversation, None, "audit", &payload)?;
            text.push_str(&match &audit {
                Ok(verdict) if verdict.approved => {
                    "\n## Independent audit\nThe request is fully delivered.\n".to_owned()
                }
                Ok(verdict) => format!(
                    "\n## Independent audit\nThe request is not fully delivered:\n{}\n\
                     Propose a plan for what is missing, or tell the user why it stays open.\n",
                    tasks::findings_text(&verdict.findings)
                ),
                Err(error) => format!(
                    "\n## Independent audit\nThe audit could not run ({error}); tell the \
                     user the result is unaudited.\n"
                ),
            });
        }
        Ok(text)
    }
}

/// A review's outcome as an event payload.
fn review_json(review: &std::result::Result<Verdict, String>) -> serde_json::Value {
    match review {
        Ok(verdict) => json!({"approved": verdict.approved, "findings": verdict.findings}),
        Err(error) => json!({"approved": false, "error": error}),
    }
}

/// The branch a writer's worktree is on.
fn branch(task: &Task) -> String {
    format!("antislop/{}", &task.id[..8])
}

/// What a comment's target is, in the orchestrator's words.
fn comment_label(target: &str, plan: &[Task]) -> String {
    let task = |id: &str| {
        plan.iter()
            .find(|task| task.spec["id"] == id)
            .map(|task| {
                format!(
                    "task {id} \"{}\"",
                    task.spec["title"].as_str().unwrap_or_default()
                )
            })
            .unwrap_or_else(|| format!("task {id}"))
    };
    match target.split_once(':') {
        Some(("worker", id)) => format!("the worker of {}", task(id)),
        Some(("verifier", id)) => format!("the verifier of {}", task(id)),
        Some(("diff", _)) => {
            let Some(diff::Target {
                task: id,
                file,
                line,
            }) = diff::target(target)
            else {
                return target.into();
            };
            let change = format!("the change of {}", task(id));
            match (file, line) {
                (Some(file), Some(line)) => match line.strip_prefix("old") {
                    Some(old) => format!("removed line {old} of {file} in {change}"),
                    None => format!("line {line} of {file} in {change}"),
                },
                (Some(file), None) => format!("{file} in {change}"),
                _ => change,
            }
        }
        _ => match target {
            ORCHESTRATOR => "your own work".into(),
            PLAN_VERIFIER => "the plan verifier's review".into(),
            AUDITOR => "the auditor's report".into(),
            other => other.into(),
        },
    }
}

fn orchestrator_prompt(
    project: &Path,
    first: bool,
    messages: &[Message],
    open: &[Comment],
) -> String {
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
            PLAN_VERIFIER => format!("[plan verifier]\n{}", message.text),
            _ => message.text.clone(),
        })
        .collect();
    prompt.push_str(&texts.join("\n\n"));
    if !open.is_empty() {
        prompt.push_str(
            "\n\nComments the user has not resolved yet (answer, ask, or propose work for \
             them; only the user resolves a comment):",
        );
        for comment in open {
            prompt.push_str(&format!(
                "\n- #{} on {}: {}",
                comment.id, comment.label, comment.text
            ));
        }
    }
    prompt
}
