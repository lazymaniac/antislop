use super::{CANCELLED, Finish, State};
use crate::{Error, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(super) fn take_finish(state: &mut State) -> Option<Finish> {
    let finish = state.finished.take();
    if finish.is_some() {
        state.expected = None;
    }
    finish
}

pub(super) fn poll_wait(
    state: &mut State,
    started: Instant,
    turn: &str,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Option<Finish>> {
    if let Some(finish) = take_finish(state) {
        return Ok(Some(finish));
    }
    if cancel.load(Ordering::SeqCst) {
        state.expected = None;
        return Err(Error::Invalid(CANCELLED.into()));
    }
    if wait_expired(started, timeout, state) {
        return Err(timeout_error(turn, timeout, state));
    }
    Ok(None)
}

fn wait_expired(started: Instant, timeout: Duration, state: &State) -> bool {
    let waiting = state.waited + state.waiting.map_or(Duration::ZERO, |at| at.elapsed());
    started.elapsed().saturating_sub(waiting) >= timeout
}

fn timeout_error(turn: &str, timeout: Duration, state: &mut State) -> Error {
    state.expected = None;
    let rejection = state
        .rejected_finish
        .as_ref()
        .map_or(String::new(), |(_, error)| {
            format!("; last rejected submission: {error}")
        });
    Error::Invalid(format!(
        "no accepted result for turn {turn} within {} s{rejection}",
        timeout.as_secs()
    ))
}
