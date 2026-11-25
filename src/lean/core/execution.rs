//! User-selected serial execution across every role and conversation in a daemon.
//! A turn yields only while coordinating through MCP or awaiting a CLI approval.
use crate::{Result, invalid};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub(crate) struct Execution {
    state: Mutex<State>,
    changed: Condvar,
}

struct State {
    serial: bool,
    active: usize,
    turns: HashMap<String, Turn>,
    waiting: VecDeque<String>,
}

#[derive(Default)]
struct Turn {
    active: bool,
    pauses: usize,
}

pub(crate) struct Lease {
    execution: Arc<Execution>,
    turn: String,
}

pub(crate) struct Suspension {
    execution: Arc<Execution>,
    turn: String,
    pending: bool,
}

impl Execution {
    pub(crate) fn new(serial: bool) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                serial,
                active: 0,
                turns: HashMap::new(),
                waiting: VecDeque::new(),
            }),
            changed: Condvar::new(),
        })
    }

    pub(crate) fn configure(&self, serial: bool) {
        self.state.lock().expect("agent execution mutex").serial = serial;
        self.changed.notify_all();
    }

    pub(crate) fn enter(self: &Arc<Self>, turn: &str, cancel: &AtomicBool) -> Result<Lease> {
        {
            let mut state = self.state.lock().map_err(invalid)?;
            crate::require(
                !state.turns.contains_key(turn),
                "agent turn already scheduled",
            )?;
            state.turns.insert(turn.into(), Turn::default());
            state.waiting.push_back(turn.into());
        }
        let lease = Lease {
            execution: self.clone(),
            turn: turn.into(),
        };
        self.acquire(turn, || cancel.load(Ordering::SeqCst))?;
        Ok(lease)
    }

    fn acquire(&self, turn: &str, cancelled: impl Fn() -> bool) -> Result<()> {
        let mut state = self.state.lock().map_err(invalid)?;
        loop {
            if cancelled() || !state.turns.contains_key(turn) {
                return Err(invalid(crate::lean::mcp::CANCELLED));
            }
            if state.turns[turn].active {
                return Ok(());
            }
            if state.turns[turn].pauses == 0
                && (!state.serial
                    || state.active == 0
                        && state.waiting.front().is_some_and(|first| first == turn))
            {
                state.waiting.retain(|id| id != turn);
                state.turns.get_mut(turn).unwrap().active = true;
                state.active += 1;
                self.changed.notify_all();
                return Ok(());
            }
            // Poll cancellation, with no deadline on queued work.
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .map_err(invalid)?
                .0;
        }
    }

    pub(crate) fn suspend(self: &Arc<Self>, turn: &str) -> Option<Suspension> {
        let mut state = self.state.lock().ok()?;
        let entry = state.turns.get_mut(turn)?;
        entry.pauses += 1;
        if std::mem::take(&mut entry.active) {
            state.active -= 1;
        }
        state.waiting.retain(|id| id != turn);
        self.changed.notify_all();
        Some(Suspension {
            execution: self.clone(),
            turn: turn.into(),
            pending: true,
        })
    }

    pub(crate) fn active(&self, turn: &str) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.turns.get(turn).is_none_or(|entry| entry.active))
    }

    /// Keep reminders and steering from waking a turn that yielded its slot.
    pub(crate) fn while_active<T>(&self, turn: &str, action: impl FnOnce() -> T) -> Option<T> {
        let state = self.state.lock().ok()?;
        if state.turns.get(turn).is_some_and(|entry| !entry.active) {
            return None;
        }
        Some(action())
    }

    fn unpause(&self, turn: &str) {
        let mut state = self.state.lock().expect("agent execution mutex");
        if let Some(entry) = state.turns.get_mut(turn) {
            entry.pauses -= 1;
            if entry.pauses == 0 {
                state.waiting.push_back(turn.into());
            }
        }
        self.changed.notify_all();
    }
}

impl Suspension {
    pub(crate) fn resume(mut self, cancelled: impl Fn() -> bool) -> Result<()> {
        self.pending = false;
        self.execution.unpause(&self.turn);
        self.execution.acquire(&self.turn, cancelled)
    }
}

impl Drop for Suspension {
    fn drop(&mut self) {
        if self.pending {
            self.execution.unpause(&self.turn);
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut state = self.execution.state.lock().expect("agent execution mutex");
        if state
            .turns
            .remove(&self.turn)
            .is_some_and(|turn| turn.active)
        {
            state.active -= 1;
        }
        state.waiting.retain(|id| id != &self.turn);
        self.execution.changed.notify_all();
    }
}

#[cfg(test)]
mod tests;
