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
    if state
        .protocol_rejected_at
        .is_some_and(|at| at.elapsed() >= Duration::from_secs(60))
    {
        state.expected = None;
        return Err(crate::lean::native::invalid(format!(
            "result rejected: MCP protocol correction stalled for turn {turn} after 60 s; last rejected submission: {}",
            state
                .rejected_finish
                .as_ref()
                .map_or("unknown rejection", |(_, reason)| reason.as_str())
        )));
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
    crate::lean::native::StopCause::Timeout.diagnostic(format!(
        "no accepted result for turn {turn} within {} s{rejection}",
        timeout.as_secs()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lean::mcp::{dispatch::call, gateway::Shared};
    use serde_json::json;
    use std::sync::{Condvar, Mutex};

    fn shared() -> Shared {
        Shared {
            token: "synthetic".into(),
            state: Mutex::new(State {
                expected: Some("current-audit".into()),
                role: "auditor".into(),
                ..State::default()
            }),
            changed: Condvar::new(),
        }
    }

    #[test]
    fn stale_missing_and_wrong_role_results_can_be_corrected_without_acceptance() {
        for arguments in [
            json!({"turn":"stale", "report":"checked", "findings":[]}),
            json!({"report":"checked", "findings":[]}),
        ] {
            let shared = shared();
            let reply = call(&shared, &json!({"name":"review", "arguments":arguments}));
            assert_eq!(reply["isError"], true);
            let reason = reply["content"][0]["text"].as_str().unwrap();
            assert!(
                reason.contains("current-audit") && reason.contains("review"),
                "{reason}"
            );
            assert!(shared.state.lock().unwrap().finished.is_none());
            assert!(shared.state.lock().unwrap().protocol_rejected_at.is_some());
            let wrong_role = call(
                &shared,
                &json!({"name":"complete", "arguments":{
                    "turn":"current-audit", "summary":"done", "report":"done"
                }}),
            );
            assert_eq!(wrong_role["isError"], true);
            assert!(shared.state.lock().unwrap().finished.is_none());
            let right = call(
                &shared,
                &json!({"name":"review", "arguments":{
                    "turn":"current-audit", "report":"checked", "findings":[]
                }}),
            );
            assert!(right["isError"].is_null(), "{right}");
            let mut state = shared.state.lock().unwrap();
            assert!(state.protocol_rejected_at.is_none());
            assert!(
                poll_wait(
                    &mut state,
                    Instant::now(),
                    "current-audit",
                    &AtomicBool::new(false),
                    Duration::from_secs(1800)
                )
                .unwrap()
                .is_some()
            );
        }
    }

    #[test]
    fn repeated_stale_results_do_not_extend_the_correction_window_or_finish_the_turn() {
        let shared = shared();
        let stale = json!({"name":"review", "arguments":{
            "turn":"stale", "report":"checked", "findings":[]
        }});
        call(&shared, &stale);
        let first = Instant::now() - Duration::from_secs(61);
        shared.state.lock().unwrap().protocol_rejected_at = Some(first);
        call(&shared, &stale);
        let mut state = shared.state.lock().unwrap();
        assert_eq!(state.protocol_rejected_at, Some(first));
        let error = poll_wait(
            &mut state,
            Instant::now(),
            "current-audit",
            &AtomicBool::new(false),
            Duration::from_secs(1800),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("MCP protocol correction stalled") && error.contains("turn is not open"),
            "{error}"
        );
        assert!(state.expected.is_none() && state.finished.is_none());
        drop(state);
        assert_eq!(call(&shared, &stale)["isError"], true);
        assert!(shared.state.lock().unwrap().protocol_rejected_at.is_some());
    }

    #[test]
    fn another_authenticated_gateway_cannot_close_this_turn() {
        let current = shared();
        let foreign = shared();
        foreign.state.lock().unwrap().expected = Some("foreign-audit".into());
        let reply = call(
            &current,
            &json!({"name":"review", "arguments":{
                "turn":"foreign-audit", "report":"checked", "findings":[]
            }}),
        );
        assert_eq!(reply["isError"], true);
        assert!(current.state.lock().unwrap().finished.is_none());
        let state = foreign.state.lock().unwrap();
        assert_eq!(state.expected.as_deref(), Some("foreign-audit"));
        assert!(state.finished.is_none() && state.rejected_finish.is_none());
    }

    #[test]
    fn an_old_waiter_cannot_consume_or_close_the_replacement_turn() {
        let mut state = State {
            expected: Some("next".into()),
            finished: Some(Finish {
                text: "next result".into(),
            }),
            ..State::default()
        };
        let cancel = AtomicBool::new(false);
        let error = poll_wait(
            &mut state,
            Instant::now(),
            "previous",
            &cancel,
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(error.to_string().contains("replaced turn"));
        assert_eq!(state.expected.as_deref(), Some("next"));
        assert!(state.finished.is_some());
        let finish = poll_wait(
            &mut state,
            Instant::now(),
            "next",
            &cancel,
            Duration::from_secs(1),
        )
        .unwrap()
        .unwrap();
        assert_eq!(finish.text, "next result");
        assert!(state.expected.is_none());
    }
}
