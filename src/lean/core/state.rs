//! Core's in-memory state, grouped by concern. Nothing here is durable:
//! a restart rebuilds it from the store.

use super::super::*;
use std::collections::{BTreeSet, HashSet};
use std::sync::atomic::AtomicU64;

/// The live configuration and how it is replaced.
pub(in crate::lean) struct ConfigState {
    pub(in crate::lean) current: RwLock<Arc<Config>>,
    /// Serializes configuration updates.
    pub(in crate::lean) update: Mutex<()>,
    /// Modification time of the configuration file last loaded.
    pub(in crate::lean) modified: AtomicU64,
}

impl ConfigState {
    pub(in crate::lean) fn new(config: Config) -> Self {
        Self {
            current: RwLock::new(Arc::new(config)),
            update: Mutex::new(()),
            modified: AtomicU64::new(0),
        }
    }
}

/// Work running per conversation and the flags that cancel it.
#[derive(Default)]
pub(in crate::lean) struct Activity {
    /// Conversations with an orchestrator loop running, and its cancel flag.
    pub(in crate::lean) running: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Conversations with a plan running, and its cancel flag.
    pub(in crate::lean) plans: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Conversations with a code review the user asked for, and its flag.
    pub(in crate::lean) reviews: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub(in crate::lean) agent_reviews: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Active task processes can be cancelled independently.
    pub(in crate::lean) task_cancels: Mutex<HashMap<String, (String, Arc<AtomicBool>)>>,
    pub(in crate::lean) task_restarts: Mutex<HashMap<String, TaskRecoveryRequest>>,
    pub(in crate::lean) orchestrator_restarts: Mutex<HashSet<String>>,
    /// The blocked report last handed to each conversation's orchestrator,
    /// so that an unchanged block is told once.
    pub(in crate::lean) blocked_seen: Mutex<HashMap<String, String>>,
}

/// Plan revisions in flight.
#[derive(Default)]
pub(in crate::lean) struct RevisionState {
    pub(in crate::lean) revising: Mutex<BTreeSet<String>>,
    pub(in crate::lean) jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Per conversation, the plan and revision its orchestrator was last
    /// shown; `revise_plan` edits exactly that graph.
    pub(in crate::lean) views: Mutex<HashMap<String, Option<(String, i64)>>>,
}

/// Task worktrees in use and those waiting to be cleaned up.
#[derive(Default)]
pub(in crate::lean) struct WorktreeState {
    pub(in crate::lean) cleanup: Mutex<BTreeSet<String>>,
    pub(in crate::lean) cleanup_notes: Mutex<HashMap<String, String>>,
    pub(in crate::lean) leases:
        Mutex<HashMap<PathBuf, crate::lean::core::worktree::cleanup::WorktreeLeaseState>>,
    pub(in crate::lean) lease_changed: std::sync::Condvar,
}

/// Locks that serialize work across conversations.
#[derive(Default)]
pub(in crate::lean) struct Locks {
    /// Writers land on the user's branch one at a time.
    pub(in crate::lean) integrating: Mutex<()>,
    pub(in crate::lean) graph_updates: Mutex<()>,
    pub(in crate::lean) turns: Mutex<mail::TurnLocks>,
    /// Serializes starting an orchestrator with permanent session deletion.
    pub(in crate::lean) session_lifecycle: Mutex<()>,
}

fn has(map: &Mutex<HashMap<String, impl Sized>>, key: &str) -> bool {
    map.lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains_key(key)
}

impl Activity {
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
