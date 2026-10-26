//! Durable mailbox dispatch. Receipt turns never own the plan scheduler slot.

use super::*;

use std::sync::{MutexGuard, TryLockError, Weak};
use std::time::Duration;

pub(super) type TurnLocks = HashMap<(String, String), Weak<Mutex<()>>>;
fn parse_mail_address(address: &str) -> (&str, &str) {
    if let Some(id) = address.strip_prefix("code-review:") {
        (id, CODE_REVIEW)
    } else if let Some(id) = address.strip_suffix("/review") {
        (id, CODE_REVIEW)
    } else if let Some(id) = address.strip_suffix("/verifier") {
        (id, "verifier")
    } else if let Some(id) = address.strip_prefix("verifier:") {
        (id, "verifier")
    } else {
        (address.strip_prefix("worker:").unwrap_or(address), "worker")
    }
}

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
}

impl Core {
    pub(in crate::lean) fn mail_address_task(
        &self,
        conversation: &str,
        plan: &[Task],
        address: &str,
    ) -> Result<Option<(Task, String)>> {
        let (id, prefix) = parse_mail_address(address);
        let task = plan
            .iter()
            .find(|task| task.spec["id"] == id || task.id == id)
            .cloned()
            .or(self.store.task_by_id(id)?);
        let Some(task) = task else { return Ok(None) };
        if self.store.plan_conversation(&task.plan_id)? != conversation {
            return Ok(None);
        };
        let role = format!("{prefix}:{}", task.id);
        Ok(Some((task, role)))
    }
}
