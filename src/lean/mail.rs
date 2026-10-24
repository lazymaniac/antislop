//! Durable mailbox dispatch. Receipt turns never own the plan scheduler slot.

use super::*;

#[path = "mail/background.rs"]
mod background;
#[path = "mail/delivery.rs"]
mod delivery;
#[path = "mail/recipient.rs"]
mod recipient;
#[path = "mail/routing.rs"]
mod routing;
use std::sync::{MutexGuard, TryLockError, Weak};
use std::time::{Duration, Instant};

pub(super) type TurnLocks = HashMap<(String, String), Weak<Mutex<()>>>;
const DELIVERY_ATTEMPTS: u32 = 3;
const MAIL_BATCH_LIMIT: usize = 4;

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

fn retryable(error: &str) -> bool {
    if error.contains(super::native::EXECUTION_FAILED) {
        return false;
    }
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
}

#[cfg(test)]
mod native_retry_tests {
    #[test]
    fn an_accepted_native_turn_is_never_automatically_retried() {
        assert!(super::retryable("connection reset"));
        assert!(!super::retryable(&format!(
            "{}connection reset after prompt submission",
            crate::lean::native::EXECUTION_FAILED
        )));
        assert!(!super::retryable(&format!(
            "{}timed out",
            crate::lean::native::EXECUTION_FAILED
        )));
    }
}
