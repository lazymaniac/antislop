//! Core's in-memory state, grouped by concern. Nothing here is durable:
//! a restart rebuilds it from the store.

use super::super::*;
use std::collections::{BTreeSet, HashSet};
use std::sync::MutexGuard;
use std::sync::atomic::AtomicU64;

/// The live configuration and how it is replaced.
pub(in crate::lean) struct ConfigState {
    pub(in crate::lean::core) current: RwLock<Arc<Config>>,
    /// Serializes configuration updates.
    pub(in crate::lean::core) update: Mutex<()>,
    /// Modification time of the configuration file last loaded.
    pub(in crate::lean::core) modified: AtomicU64,
}

impl ConfigState {
    pub(in crate::lean) fn new(config: Config) -> Self {
        Self {
            current: RwLock::new(Arc::new(config)),
            update: Mutex::new(()),
            modified: AtomicU64::new(0),
        }
    }

    /// Held while the configuration file is read or replaced.
    pub(in crate::lean) fn updating(&self) -> MutexGuard<'_, ()> {
        self.update
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// Modification time of the configuration file last loaded.
    pub(in crate::lean) fn modified(&self) -> u64 {
        self.modified.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(in crate::lean) fn set_modified(&self, modified: u64) {
        self.modified
            .store(modified, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Work running per conversation and the flags that cancel it.
#[derive(Default)]
pub(in crate::lean) struct Activity {
    /// Conversations with an orchestrator loop running, and its cancel flag.
    pub(in crate::lean::core) running: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Scoped preparation/recovery work participates in handoff and deletion.
    pub(in crate::lean::core) operations: Mutex<HashMap<String, ScopedOperation>>,
    /// Conversations with a plan running, and its cancel flag.
    pub(in crate::lean::core) plans: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Conversations with a code review the user asked for, and its flag.
    pub(in crate::lean::core) reviews: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub(in crate::lean::core) agent_reviews: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Active task processes can be cancelled independently.
    pub(in crate::lean::core) task_cancels: Mutex<HashMap<String, (String, Arc<AtomicBool>)>>,
    pub(in crate::lean::core) task_restarts: Mutex<HashMap<String, TaskRecoveryRequest>>,
    pub(in crate::lean::core) orchestrator_restarts: Mutex<HashSet<String>>,
    /// The blocked report last handed to each conversation's orchestrator,
    /// so that an unchanged block is told once.
    pub(in crate::lean::core) blocked_seen: Mutex<HashMap<String, String>>,
}

/// Plan revisions in flight.
#[derive(Default)]
pub(in crate::lean) struct RevisionState {
    pub(in crate::lean::core) revising: Mutex<BTreeSet<String>>,
    pub(in crate::lean::core) jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Per conversation, the plan and revision its orchestrator was last
    /// shown; `revise_plan` edits exactly that graph.
    pub(in crate::lean::core) views: Mutex<HashMap<String, Option<(String, i64)>>>,
}

/// Task worktrees in use and those waiting to be cleaned up.
#[derive(Default)]
pub(in crate::lean) struct WorktreeState {
    pub(in crate::lean::core) cleanup: Mutex<BTreeSet<String>>,
    pub(in crate::lean::core) cleanup_notes: Mutex<HashMap<String, String>>,
    pub(in crate::lean::core) leases:
        Mutex<HashMap<PathBuf, crate::lean::core::worktree::cleanup::WorktreeLeaseState>>,
    pub(in crate::lean::core) lease_changed: std::sync::Condvar,
}

/// Locks that serialize work across conversations.
#[derive(Default)]
pub(in crate::lean) struct Locks {
    /// Writers land on the user's branch one at a time.
    pub(in crate::lean::core) integrating: Mutex<()>,
    pub(in crate::lean::core) graph_updates: Mutex<()>,
    pub(in crate::lean::core) turns: Mutex<mail::TurnLocks>,
    /// Serializes message commits and orchestrator starts with session deletion
    /// and the final runtime handoff.
    pub(in crate::lean) session_lifecycle: Mutex<()>,
}

fn has(map: &Mutex<HashMap<String, impl Sized>>, key: &str) -> bool {
    map.lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains_key(key)
}

pub(in crate::lean::core) struct ScopedOperation {
    pub conversation: String,
    pub task: Option<String>,
    pub cancel: Arc<AtomicBool>,
}

pub(in crate::lean) struct OperationLease<'a> {
    core: &'a Core,
    id: String,
    pub(in crate::lean) cancel: Arc<AtomicBool>,
}
impl OperationLease<'_> {
    pub(in crate::lean) fn bind_task(&self, task: &str) {
        if let Some(operation) = self
            .core
            .activity
            .operations
            .lock()
            .expect("operation leases")
            .get_mut(&self.id)
        {
            operation.task = Some(task.into());
        }
    }
}
impl Drop for OperationLease<'_> {
    fn drop(&mut self) {
        self.core
            .activity
            .operations
            .lock()
            .expect("operation leases")
            .remove(&self.id);
    }
}
impl Core {
    /// Work registration callers hold the lifecycle admission fence.
    pub(in crate::lean) fn require_accepting(&self) -> Result<()> {
        require(
            !daemon::activation::pending(self) && !self.state.join("stopped").exists(),
            "runtime is switching or stopping; retry after readiness is confirmed",
        )
    }
    pub(in crate::lean) fn cancel_task_operations(&self, conversation: &str, task: &str) -> usize {
        self.activity
            .operations
            .lock()
            .expect("operation leases")
            .values()
            .filter(|operation| {
                if operation.conversation == conversation && operation.task.as_deref() == Some(task)
                {
                    operation.cancel.store(true, Ordering::SeqCst);
                    true
                } else {
                    false
                }
            })
            .count()
    }
    pub(in crate::lean) fn cancel_operations(&self) {
        for operation in self
            .activity
            .operations
            .lock()
            .expect("operation leases")
            .values()
        {
            operation.cancel.store(true, Ordering::SeqCst);
        }
    }
    pub(in crate::lean) fn operation(&self, conversation: &str) -> Result<OperationLease<'_>> {
        let _lifecycle = self
            .locks
            .session_lifecycle
            .lock()
            .expect("session lifecycle mutex");
        self.require_accepting()?;
        let id = crate::id();
        let cancel = Arc::new(AtomicBool::new(false));
        self.activity
            .operations
            .lock()
            .expect("operation leases")
            .insert(
                id.clone(),
                ScopedOperation {
                    conversation: conversation.into(),
                    task: None,
                    cancel: cancel.clone(),
                },
            );
        Ok(OperationLease {
            core: self,
            id,
            cancel,
        })
    }
}

impl Locks {
    pub(in crate::lean) fn primary(&self, project: &Path) -> Result<Arc<Mutex<()>>> {
        Ok(self.turn(&project.canonicalize()?.to_string_lossy(), "primary-tree"))
    }
    pub(in crate::lean) fn source(&self, conversation: &str) -> Arc<Mutex<()>> {
        self.turn(conversation, "source-scope")
    }

    /// Held while a change lands on the user's branch or the scope it
    /// lands into is read.
    pub(in crate::lean) fn integration(&self) -> MutexGuard<'_, ()> {
        self.integrating.lock().expect("integration mutex")
    }

    /// The lock one role's turns in `conversation` take, shared while
    /// anyone holds it.
    pub(in crate::lean) fn turn(&self, conversation: &str, role: &str) -> Arc<Mutex<()>> {
        let mut locks = self.turns.lock().unwrap_or_else(|error| error.into_inner());
        // Weak entries must not accumulate with completed historical sessions.
        locks.retain(|_, lock| lock.strong_count() > 0);
        let key = (conversation.to_owned(), role.to_owned());
        if let Some(lock) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }

    /// Held while the task graph changes.
    pub(in crate::lean) fn graph(&self) -> MutexGuard<'_, ()> {
        self.graph_updates.lock().expect("graph update mutex")
    }
}

impl Activity {
    /// Registers an orchestrator loop for `conversation` and returns its
    /// cancel flag, or `None` when one already runs.
    pub(in crate::lean) fn start_orchestrator(
        &self,
        conversation: &str,
    ) -> Option<Arc<AtomicBool>> {
        let mut running = self.running.lock().expect("running mutex");
        if running.contains_key(conversation) {
            return None;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        running.insert(conversation.to_owned(), cancel.clone());
        Some(cancel)
    }

    /// Asks the processes of `tasks` that run to stop.
    pub(in crate::lean) fn cancel_tasks<'a>(&self, tasks: impl IntoIterator<Item = &'a String>) {
        let flags = self.task_cancels.lock().expect("task cancel mutex");
        for task in tasks {
            if let Some((_, flag)) = flags.get(task) {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }

    /// Whether a plan is running in `conversation`.
    pub(in crate::lean) fn plan_running(&self, conversation: &str) -> bool {
        has(&self.plans, conversation)
    }

    /// Whether a code review the user asked for runs in `conversation`.
    pub(in crate::lean) fn review_running(&self, conversation: &str) -> bool {
        has(&self.reviews, conversation)
    }

    /// Whether an agent review runs under `key`.
    pub(in crate::lean) fn agent_review_running(&self, key: &str) -> bool {
        has(&self.agent_reviews, key)
    }

    /// Whether task `task` has a process running.
    pub(in crate::lean) fn task_running(&self, task: &str) -> bool {
        has(&self.task_cancels, task)
    }
}

impl RevisionState {
    /// The plan and revision a revision in `conversation` is reviewed
    /// against, while one is.
    pub(in crate::lean) fn view(&self, conversation: &str) -> Option<(String, i64)> {
        self.views
            .lock()
            .expect("revision views mutex")
            .get(conversation)
            .cloned()
            .flatten()
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;

    #[test]
    fn a_reserved_operation_fences_deletion_and_accepts_manual_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        let operation = core.operation(&conversation).unwrap();
        assert!(core.busy());
        assert!(!core.is_idle(&conversation));
        assert!(
            core.delete_session(&conversation)
                .unwrap_err()
                .to_string()
                .contains("stop this session")
        );
        core.cancel(&conversation);
        assert!(operation.cancel.load(Ordering::SeqCst));
        drop(operation);
        assert!(!core.busy());
        assert!(core.is_idle(&conversation));
        core.delete_session(&conversation).unwrap();
        std::fs::write(core.state.join("stopped"), []).unwrap();
        assert!(core.operation("new-operation").is_err());
    }

    #[test]
    fn waiting_for_a_conversation_source_scope_holds_no_global_lock() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        let task=core.store.add_plan(&conversation,&[json!({"id":"writer","title":"Writer","goal":"Create result","change_kind":"code","kind":"implementation"})]).unwrap().remove(0);
        let spec: TaskContract = serde_json::from_value(task.spec.clone()).unwrap();
        let source = core.locks.source(&conversation);
        let held = source.lock().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = core.clone();
        let root = dir.path().to_path_buf();
        std::thread::spawn(move || {
            let _ = tx.send(worker.writer_base(&task.plan_id, &spec, &root));
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while core.activity.operations.lock().unwrap().is_empty() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(core.busy());
        assert!(!core.is_idle(&conversation));
        assert!(core.locks.graph_updates.try_lock().is_ok());
        assert!(core.locks.integrating.try_lock().is_ok());
        assert!(core.activity.plans.try_lock().is_ok());
        assert!(core.locks.source("another-conversation").try_lock().is_ok());
        core.cancel(&conversation);
        drop(held);
        let error = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("writer allocation cancelled"));
    }
}
