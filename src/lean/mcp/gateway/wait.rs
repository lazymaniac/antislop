use super::{CANCELLED, Finish, State};
use crate::{Error, Result};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn take_finish(state: &mut State) -> Option<Finish> {
    let finish = state.finished.take();
    if finish.is_some() {
        state.expected = None;
    }
    finish
}

pub(super) fn poll_wait(
    state: &mut State,
    turn: &str,
    cancel: &AtomicBool,
) -> Result<Option<Finish>> {
    crate::require(
        state
            .expected
            .as_deref()
            .is_none_or(|expected| expected == turn),
        "this wait belongs to a replaced turn; the active turn is unchanged",
    )?;
    if let Some(finish) = take_finish(state) {
        return Ok(Some(finish));
    }
    if cancel.load(Ordering::SeqCst) {
        state.expected = None;
        return Err(Error::Invalid(CANCELLED.into()));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_cancellation_closes_only_its_own_turn() {
        let mut state = State {
            expected: Some("next".into()),
            ..State::default()
        };
        let cancel = AtomicBool::new(true);
        assert!(
            poll_wait(&mut state, "old", &cancel)
                .unwrap_err()
                .to_string()
                .contains("replaced turn")
        );
        assert_eq!(state.expected.as_deref(), Some("next"));
        assert_eq!(
            poll_wait(&mut state, "next", &cancel)
                .unwrap_err()
                .to_string(),
            CANCELLED
        );
        assert!(state.expected.is_none());
    }
    #[test]
    fn rejected_submissions_have_no_correction_deadline() {
        let mut state = State {
            expected: Some("current".into()),
            protocol_rejected_at: Some(
                std::time::Instant::now() - std::time::Duration::from_secs(10000),
            ),
            ..State::default()
        };
        assert!(
            poll_wait(&mut state, "current", &AtomicBool::new(false))
                .unwrap()
                .is_none()
        );
        assert_eq!(state.expected.as_deref(), Some("current"));
    }
}
