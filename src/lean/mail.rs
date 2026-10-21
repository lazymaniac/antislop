//! Durable mailbox dispatch. Receipt turns never own the plan scheduler slot.

use super::*;
use std::sync::{MutexGuard, TryLockError, Weak};
use std::time::{Duration, Instant};

pub(super) type TurnLocks = HashMap<(String, String), Weak<Mutex<()>>>;
const DELIVERY_ATTEMPTS: u32 = 3;
const MAIL_BATCH_LIMIT: usize = 4;

pub(super) fn lock_cancellable<'a>(
    lock: &'a Mutex<()>,
    cancel: &AtomicBool,
) -> Option<MutexGuard<'a, ()>> {
    loop {
        if cancel.load(Ordering::SeqCst) {
            return None;
        }
        match lock.try_lock() {
            Ok(guard) => return Some(guard),
            Err(TryLockError::Poisoned(error)) => return Some(error.into_inner()),
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}

fn retryable(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    [
        "temporarily unavailable",
        "temporary failure",
        "connection reset",
        "connection refused",
        "timed out",
        "timeout",
        "database is locked",
        "os error 11",
        "os error 35",
    ]
    .iter()
    .any(|reason| error.contains(reason))
}

fn retry_delay(attempt: u32, cancel: &AtomicBool) -> bool {
    let end = Instant::now() + Duration::from_millis(250 * (1 << attempt));
    while Instant::now() < end {
        if cancel.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    !cancel.load(Ordering::SeqCst)
}

impl Core {
    pub(super) fn turn_lock(&self, conversation: &str, role: &str) -> Arc<Mutex<()>> {
        let mut locks = self
            .turn_locks
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // Weak entries must not accumulate with completed historical sessions.
        locks.retain(|_, lock| lock.strong_count() > 0);
        let key = (conversation.to_owned(), role.to_owned());
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }

    /// Labels are local to a plan; an explicit UUID can name an earlier task
    /// in this conversation, but can never cross the conversation boundary.
    pub(super) fn mail_address_task(
        &self,
        conversation: &str,
        plan: &[Task],
        address: &str,
    ) -> Result<Option<(Task, String)>> {
        let (id, prefix) = if let Some(id) = address.strip_prefix("code-review:") {
            (id, "code-review")
        } else if let Some(id) = address.strip_suffix("/review") {
            (id, "code-review")
        } else if let Some(id) = address.strip_suffix("/verifier") {
            (id, "verifier")
        } else if let Some(id) = address.strip_prefix("verifier:") {
            (id, "verifier")
        } else {
            (address.strip_prefix("worker:").unwrap_or(address), "worker")
        };
        let task = plan
            .iter()
            .find(|task| task.spec["id"] == id || task.id == id)
            .cloned()
            .or(self.store.task_by_id(id)?);
        let Some(task) = task else {
            return Ok(None);
        };
        if self.store.plan_conversation(&task.plan_id)? != conversation {
            return Ok(None);
        }
        let role = format!("{prefix}:{}", task.id);
        Ok(Some((task, role)))
    }

    fn ready_mail(&self, conversation: &str) -> Result<Vec<store::MailTarget>> {
        let mut targets = self.store.pending_mail_targets(conversation)?;
        let failures = self.mail_failures.lock().expect("mail failures mutex");
        targets.retain(|target| {
            let supported = if let Some(id) = &target.task_id {
                // Pending workers consume their instructions in their task turn.
                self.store
                    .task_by_id(id)
                    .ok()
                    .flatten()
                    .is_some_and(|task| status::finished(&task.status))
            } else {
                matches!(
                    target.role.as_str(),
                    ORCHESTRATOR | COMMENT_ORCHESTRATOR | PLAN_VERIFIER | AUDITOR
                )
            };
            let direct_orchestrator = target.role == ORCHESTRATOR
                && self
                    .store
                    .pending_mail(conversation, &target.role)
                    .is_ok_and(|mail| {
                        mail.iter()
                            .all(|mail| mail.kind == "agent_work" || mail.kind == "copy")
                    });
            supported
                && !direct_orchestrator
                && failures
                    .get(&(conversation.to_owned(), target.role.clone()))
                    .is_none_or(|failed| target.newest > *failed)
        });
        Ok(targets)
    }

    pub(super) fn answer_in_background(self: &Arc<Self>, conversation: &str, _plan_id: &str) {
        let mut jobs = self.mail_jobs.lock().expect("mail jobs mutex");
        if jobs.contains_key(conversation)
            || self
                .ready_mail(conversation)
                .is_ok_and(|mail| mail.is_empty())
        {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        jobs.insert(conversation.to_owned(), cancel.clone());
        drop(jobs);
        let core = Arc::clone(self);
        let conversation = conversation.to_owned();
        std::thread::spawn(move || {
            let mut batches = 0;
            loop {
                batches += 1;
                let before = core
                    .ready_mail(&conversation)
                    .ok()
                    .and_then(|targets| targets.iter().map(|target| target.newest).max())
                    .unwrap_or(0);
                if let Err(error) = core.answer_waiting(&conversation, "", &cancel) {
                    core.notice(
                        &conversation,
                        &format!("mail delivery failed; messages remain queued: {error}"),
                    );
                }
                // A sender persists its message before taking this same lock.
                // Either the drain sees it here, or that sender starts a new job.
                let mut jobs = core.mail_jobs.lock().expect("mail jobs mutex");
                let newer = core
                    .ready_mail(&conversation)
                    .ok()
                    .is_some_and(|targets| targets.iter().any(|target| target.newest > before));
                if newer && batches < MAIL_BATCH_LIMIT && !cancel.load(Ordering::SeqCst) {
                    drop(jobs);
                    continue;
                }
                let mut external_arrival = false;
                if newer && batches >= MAIL_BATCH_LIMIT {
                    if let Ok(targets) = core.ready_mail(&conversation) {
                        let mut failures = core.mail_failures.lock().expect("mail failures mutex");
                        for target in targets {
                            let external = core
                                .store
                                .pending_mail(&conversation, &target.role)
                                .ok()
                                .is_some_and(|mail| {
                                    mail.iter().any(|mail| {
                                        mail.id > before
                                            && matches!(mail.sender.as_str(), "user" | RUNTIME)
                                    })
                                });
                            if external {
                                external_arrival = true;
                            } else {
                                failures.insert((conversation.clone(), target.role), target.newest);
                            }
                        }
                    }
                    core.notice(&conversation, "mail response limit reached; remaining agent replies stay queued until a new message or restart");
                }
                jobs.remove(&conversation);
                drop(jobs);
                if external_arrival && !cancel.load(Ordering::SeqCst) {
                    core.answer_in_background(&conversation, "");
                }
                break;
            }
        });
    }

    pub(super) fn answer_waiting(
        self: &Arc<Self>,
        conversation: &str,
        _plan_id: &str,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let drain = self.turn_lock(conversation, "__mail_drain");
        let Some(_drain) = lock_cancellable(&drain, cancel) else {
            return Ok(());
        };
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        for target in self.ready_mail(conversation)? {
            let task = target
                .task_id
                .as_deref()
                .map(|id| self.store.task_by_id(id))
                .transpose()?
                .flatten();
            let spec = task
                .as_ref()
                .map(|task| serde_json::from_value::<TaskSpec>(task.spec.clone()))
                .transpose()?;
            let (kind, role_kind, sender, title) = if let Some(spec) = &spec {
                let verifier =
                    target.role.starts_with("verifier:") || target.role.starts_with("code-review:");
                (
                    spec.kind,
                    if verifier {
                        RoleKind::Verifier
                    } else {
                        RoleKind::Executor
                    },
                    if target.role.starts_with("code-review:") {
                        format!("{}/review", spec.id)
                    } else if verifier {
                        format!("{}/verifier", spec.id)
                    } else {
                        spec.id.clone()
                    },
                    spec.title.clone(),
                )
            } else {
                (
                    WorkKind::Orchestration,
                    if matches!(target.role.as_str(), ORCHESTRATOR | COMMENT_ORCHESTRATOR) {
                        RoleKind::Orchestrator
                    } else {
                        RoleKind::Verifier
                    },
                    target.role.clone(),
                    target.role.clone(),
                )
            };
            for _ in 0..MAIL_TURNS {
                if cancel.load(Ordering::SeqCst)
                    || self
                        .store
                        .pending_mail(conversation, &target.role)?
                        .is_empty()
                {
                    break;
                }
                let mut completed = false;
                for attempt in 0..DELIVERY_ATTEMPTS {
                    let setup = TurnSetup {
                        conversation,
                        plan_id: target.plan_id.clone(),
                        role: target.role.clone(),
                        profile: self.config_snapshot().resolve(kind, role_kind),
                        cwd: project.clone(),
                        add_dirs: vec![],
                        access: agent::Access::ReadOnly,
                        upto_message: 0,
                    };
                    let answer = self.turn_inner(setup, cancel, |first| {
                        format!("Mailbox recipient: {}. This is a receipt-only turn; do not execute work, approve tasks or propose plans.\n{}",
                            target.role, tasks::answer_prompt(&title, first))
                    }, true)?;
                    if let Some(answer) = answer {
                        self.post(conversation, target.plan_id.as_deref(), &sender, &answer)?;
                        self.mail_failures
                            .lock()
                            .expect("mail failures mutex")
                            .remove(&(conversation.to_owned(), target.role.clone()));
                        completed = true;
                        break;
                    }
                    if cancel.load(Ordering::SeqCst)
                        || self
                            .store
                            .pending_mail(conversation, &target.role)?
                            .is_empty()
                    {
                        break;
                    }
                    let error = self
                        .store
                        .last_turn(conversation, &target.role)?
                        .and_then(|turn| turn.error)
                        .unwrap_or_else(|| "recipient turn ended without acknowledgement".into());
                    let retry = attempt + 1 < DELIVERY_ATTEMPTS && retryable(&error);
                    self.store.append_event(conversation, None, target.plan_id.as_deref(),
                        if retry { "mail_retry" } else { "mail_delivery_failed" },
                        &json!({"recipient": target.role, "attempt": attempt + 1, "reason": error, "state": "queued"}))?;
                    if !retry || !retry_delay(attempt, cancel) {
                        break;
                    }
                }
                if !completed {
                    self.mail_failures
                        .lock()
                        .expect("mail failures mutex")
                        .insert(
                            (conversation.to_owned(), target.role.clone()),
                            target.newest,
                        );
                    break;
                }
            }
        }
        Ok(())
    }
}
