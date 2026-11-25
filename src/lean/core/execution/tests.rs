use super::*;
use std::sync::mpsc;
use std::thread;

fn queued(execution: &Execution, turn: &str) {
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while !execution
        .state
        .lock()
        .unwrap()
        .waiting
        .iter()
        .any(|id| id == turn)
    {
        assert!(
            std::time::Instant::now() < until,
            "turn did not queue: {turn}"
        );
        thread::yield_now();
    }
}

#[test]
fn serial_turns_are_fifo_and_cancelled_waiters_do_not_block_successors() {
    let execution = Execution::new(true);
    let first = execution.enter("worker", &AtomicBool::new(false)).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let waiter = execution.clone();
    let cancelled = cancel.clone();
    let second = thread::spawn(move || waiter.enter("reviewer", &cancelled));
    queued(&execution, "reviewer");
    let waiter = execution.clone();
    let (send, receive) = mpsc::channel();
    let third = thread::spawn(move || {
        let lease = waiter
            .enter("orchestrator", &AtomicBool::new(false))
            .unwrap();
        send.send(()).unwrap();
        lease
    });
    queued(&execution, "orchestrator");
    assert!(receive.try_recv().is_err());
    cancel.store(true, Ordering::SeqCst);
    assert_eq!(
        second.join().unwrap().err().unwrap().to_string(),
        crate::lean::mcp::CANCELLED
    );
    assert!(receive.try_recv().is_err());
    drop(first);
    receive.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(third.join().unwrap());
    assert!(execution.state.lock().unwrap().turns.is_empty());
}

#[test]
fn coordination_yields_to_another_role_and_resumes_only_after_it_finishes() {
    let execution = Execution::new(true);
    let worker = execution.enter("worker", &AtomicBool::new(false)).unwrap();
    let waiter = execution.clone();
    let (send, receive) = mpsc::channel();
    let (finish, finished) = mpsc::channel();
    let orchestrator = thread::spawn(move || {
        let _lease = waiter
            .enter("orchestrator", &AtomicBool::new(false))
            .unwrap();
        send.send(()).unwrap();
        finished.recv().unwrap();
    });
    queued(&execution, "orchestrator");
    let question = execution.suspend("worker").unwrap();
    receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        execution
            .while_active("worker", || panic!("must not steer a suspended turn"))
            .is_none()
    );
    let (send, receive) = mpsc::channel();
    let resumed = thread::spawn(move || {
        question.resume(|| false).unwrap();
        send.send(()).unwrap();
        worker
    });
    queued(&execution, "worker");
    assert!(receive.try_recv().is_err());
    finish.send(()).unwrap();
    orchestrator.join().unwrap();
    receive.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(resumed.join().unwrap());
}

#[test]
fn live_policy_changes_drain_active_turns_or_release_waiters() {
    let execution = Execution::new(false);
    let first = execution.enter("first", &AtomicBool::new(false)).unwrap();
    let second = execution.enter("second", &AtomicBool::new(false)).unwrap();
    execution.configure(true);
    let waiter = execution.clone();
    let (send, receive) = mpsc::channel();
    let pending = thread::spawn(move || {
        let lease = waiter.enter("third", &AtomicBool::new(false)).unwrap();
        send.send(()).unwrap();
        lease
    });
    queued(&execution, "third");
    drop(first);
    assert!(receive.try_recv().is_err());
    execution.configure(false);
    receive.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(pending.join().unwrap());
    drop(second);
}

#[test]
fn cancellation_of_a_yielded_turn_unblocks_its_waiting_tool_response() {
    let execution = Execution::new(true);
    let worker = execution.enter("worker", &AtomicBool::new(false)).unwrap();
    let question = execution.suspend("worker").unwrap();
    let orchestrator = execution
        .enter("orchestrator", &AtomicBool::new(false))
        .unwrap();
    let resumed = thread::spawn(move || question.resume(|| false));
    queued(&execution, "worker");
    drop(worker);
    assert_eq!(
        resumed.join().unwrap().unwrap_err().to_string(),
        crate::lean::mcp::CANCELLED
    );
    drop(orchestrator);
    assert!(execution.state.lock().unwrap().turns.is_empty());
}

#[test]
fn concurrent_tool_waits_resume_the_agent_only_after_both_finish() {
    let execution = Execution::new(true);
    let worker = execution.enter("worker", &AtomicBool::new(false)).unwrap();
    let first = execution.suspend("worker").unwrap();
    let second = execution.suspend("worker").unwrap();
    let (send, receive) = mpsc::channel();
    let resumed = thread::spawn(move || {
        first.resume(|| false).unwrap();
        send.send(()).unwrap();
    });
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while execution.state.lock().unwrap().turns["worker"].pauses != 1 {
        assert!(std::time::Instant::now() < until);
        thread::yield_now();
    }
    assert!(receive.try_recv().is_err());
    assert!(!execution.active("worker"));
    second.resume(|| false).unwrap();
    receive.recv_timeout(Duration::from_secs(5)).unwrap();
    resumed.join().unwrap();
    assert_eq!(execution.state.lock().unwrap().active, 1);
    drop(worker);
}
