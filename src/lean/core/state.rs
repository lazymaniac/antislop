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
    /// Serializes starting an orchestrator with permanent session deletion.
    pub(in crate::lean::core) session_lifecycle: Mutex<()>,
}

fn has(map: &Mutex<HashMap<String, impl Sized>>, key: &str) -> bool {
    map.lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains_key(key)
}

impl Locks {
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
