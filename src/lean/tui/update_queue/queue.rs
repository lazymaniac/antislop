use crate::lean::tui::Update;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

struct State {
    items: VecDeque<Update>,
    receiver_open: bool,
}

struct Shared {
    capacity: usize,
    state: Mutex<State>,
    ready: Condvar,
    space: Condvar,
}

/// A cloneable producer for the bounded TUI update FIFO.
#[derive(Clone)]
pub struct UpdateSender(Arc<Shared>);

/// The single consumer for the bounded TUI update FIFO.
pub struct UpdateReceiver(Arc<Shared>);

/// Construct an update channel with a fixed, nonzero capacity.
pub fn update_channel(capacity: usize) -> (UpdateSender, UpdateReceiver) {
    assert!(capacity > 0, "update queue capacity must be nonzero");
    let shared = Arc::new(Shared {
        capacity,
        state: Mutex::new(State {
            items: VecDeque::new(),
            receiver_open: true,
        }),
        ready: Condvar::new(),
        space: Condvar::new(),
    });
    (UpdateSender(shared.clone()), UpdateReceiver(shared))
}

impl UpdateSender {
    /// Enqueue in FIFO order, waiting for capacity or receiver shutdown.
    pub fn send(&self, update: Update) -> Result<(), QueueClosed> {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        while state.receiver_open && state.items.len() == self.0.capacity {
            state = self.0.space.wait(state).unwrap_or_else(|e| e.into_inner());
        }
        if !state.receiver_open {
            return Err(QueueClosed);
        }
        state.items.push_back(update);
        self.0.ready.notify_one();
        Ok(())
    }
}

impl UpdateReceiver {
    /// Remove the oldest pending update without waiting.
    pub fn try_recv(&self) -> Result<Update, QueueEmpty> {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        let item = state.items.pop_front();
        if item.is_some() {
            self.0.space.notify_one();
        }
        item.ok_or(QueueEmpty)
    }

    /// Wait up to `timeout` for the oldest pending update.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Update, QueueEmpty> {
        let deadline = Instant::now() + timeout;
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(item) = state.items.pop_front() {
                self.0.space.notify_one();
                return Ok(item);
            }
            if !state.receiver_open {
                return Err(QueueEmpty);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(QueueEmpty);
            }
            let (next, timed_out) = self
                .0
                .ready
                .wait_timeout(state, remaining)
                .unwrap_or_else(|error| error.into_inner());
            state = next;
            if timed_out.timed_out() && state.items.is_empty() {
                return Err(QueueEmpty);
            }
        }
    }
}

impl Drop for UpdateReceiver {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.receiver_open = false;
        self.0.space.notify_all();
        self.0.ready.notify_all();
    }
}

/// The queue currently has no pending update.
#[derive(Debug)]
pub struct QueueEmpty;

/// The consumer has shut down before this update could be queued.
#[derive(Debug)]
pub struct QueueClosed;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lean::tui::Update;
    use std::sync::mpsc;
    use std::time::Duration;

    fn item(n: usize) -> Update {
        Update::Notice(n.to_string())
    }

    #[test]
    fn bounded_fifo_applies_backpressure_until_consumer_drains() {
        let (tx, rx) = update_channel(2);
        assert!(tx.send(item(1)).is_ok());
        assert!(tx.send(item(2)).is_ok());
        let (done, completed) = mpsc::channel();
        let producer = tx.clone();
        let join = std::thread::spawn(move || {
            assert!(producer.send(item(3)).is_ok());
            done.send(()).unwrap();
        });
        assert!(completed.recv_timeout(Duration::from_millis(30)).is_err());
        assert!(matches!(rx.try_recv(), Ok(Update::Notice(value)) if value == "1"));
        completed.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(rx.try_recv(), Ok(Update::Notice(value)) if value == "2"));
        assert!(matches!(rx.try_recv(), Ok(Update::Notice(value)) if value == "3"));
        join.join().unwrap();
    }

    #[test]
    fn receiver_shutdown_wakes_blocked_producer() {
        let (tx, rx) = update_channel(1);
        assert!(tx.send(item(1)).is_ok());
        let (done, completed) = mpsc::channel();
        let join = std::thread::spawn(move || {
            let result = tx.send(item(2));
            done.send(result.is_err()).unwrap();
        });
        drop(rx);
        assert!(completed.recv_timeout(Duration::from_secs(1)).unwrap());
        join.join().unwrap();
    }
}
