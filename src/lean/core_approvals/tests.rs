use super::*;
use crate::config::{Config, ProviderKind};

const COMMAND: &str = include_str!("../../../tests/fixtures/approvals/claude/command.txt");
const TRUST: &str = include_str!("../../../tests/fixtures/approvals/claude/trust.txt");

fn dialog(fixture: &str) -> Approval {
    approvals::detect(ProviderKind::Claude, fixture.split_once('\n').unwrap().1).unwrap()
}

fn setup() -> (tempfile::TempDir, Arc<Core>, String) {
    let directory = tempfile::tempdir().unwrap();
    let config = Config::from_toml(include_str!("../../../examples/antislop.toml")).unwrap();
    let core = Core::open(&directory.path().join("state"), config).unwrap();
    let conversation = core
        .store
        .create_conversation(directory.path().to_str().unwrap())
        .unwrap();
    (directory, core, conversation.id)
}

fn seat(core: &Core, conversation: &str, role: &str) -> ApprovalSeat {
    let turn = core
        .store
        .start_turn(conversation, role, "claude", 0, None)
        .unwrap();
    ApprovalSeat {
        conversation: conversation.into(),
        role: role.into(),
        turn: turn.id,
        provider: "claude".into(),
        directories: vec![],
    }
}

/// Settles `approval` on another thread, as a session's wait loop would.
fn settle(
    core: &Arc<Core>,
    seat: &ApprovalSeat,
    approval: Approval,
    shown: bool,
) -> std::sync::mpsc::Receiver<(Result<Verdict>, Arc<AtomicBool>)> {
    let (send, receive) = std::sync::mpsc::channel();
    let (core, seat) = (core.clone(), seat.clone());
    let cancel = Arc::new(AtomicBool::new(false));
    std::thread::spawn(move || {
        let verdict = core.settle_approval(&seat, &approval, &cancel, &|| shown);
        let _ = send.send((verdict, cancel));
    });
    receive
}

fn open_request(core: &Core, conversation: &str) -> (String, String) {
    for _ in 0..200 {
        let found = core
            .store
            .db()
            .query_row(
                "SELECT id,decider FROM approvals WHERE conversation_id=?1 AND state='open'",
                [conversation],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        if let Some(found) = found {
            return found;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("no approval was opened");
}

#[test]
fn a_supervised_approval_waits_for_the_user_and_presses_their_choice() {
    let (_directory, core, conversation) = setup();
    let seat = seat(&core, &conversation, "worker:t1");
    let done = settle(&core, &seat, dialog(COMMAND), true);
    let (request, decider) = open_request(&core, &conversation);
    assert_eq!(decider, "user");
    assert!(done.try_recv().is_err(), "nobody answered yet");
    let error = core
        .answer_approval(&conversation, &request, "9")
        .unwrap_err()
        .to_string();
    assert!(error.contains("choice ids"), "{error}");
    core.answer_approval(&conversation, &request, "2").unwrap();
    // The same answer twice is accepted, another one is not.
    core.answer_approval(&conversation, &request, "2").unwrap();
    assert!(core.answer_approval(&conversation, &request, "1").is_err());
    let (verdict, _) = done.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(verdict.unwrap(), Verdict::Choose("2".into()));
}

#[test]
fn an_autonomous_approval_goes_to_the_orchestrator_who_may_escalate() {
    let (_directory, core, conversation) = setup();
    core.set_approval_mode(&conversation, "autonomous").unwrap();
    let seat = seat(&core, &conversation, "worker:t1");
    let done = settle(&core, &seat, dialog(COMMAND), true);
    let (request, decider) = open_request(&core, &conversation);
    assert_eq!(decider, "orchestrator");
    let mut told = None;
    for _ in 0..200 {
        told = core
            .store
            .messages_after(&conversation, 0)
            .unwrap()
            .into_iter()
            .find(|message| message.text.contains(&request));
        if told.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let message = told.expect("the orchestrator is told");
    assert_eq!(message.author, "runtime");
    assert!(message.text.contains("decide_approval"), "{}", message.text);
    core.decide_approval_tool(
        &conversation,
        &json!({"request":request,"choice":"escalate_to_user","explanation":"writes outside the project"}),
    )
    .unwrap();
    assert_eq!(core.store.approval_state(&request).unwrap().2, "user");
    let stale = core
        .decide_approval_tool(&conversation, &json!({"request":request,"choice":"1"}))
        .unwrap_err()
        .to_string();
    assert!(stale.contains("owned by another decider"), "{stale}");
    let stale_at_store = core
        .store
        .answer_approval(&conversation, &request, "1", "orchestrator")
        .unwrap_err()
        .to_string();
    assert!(
        stale_at_store.contains("owned by another decider"),
        "{stale_at_store}"
    );
    core.answer_approval(&conversation, &request, "4").unwrap();
    let (verdict, _) = done.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(verdict.unwrap(), Verdict::Choose("4".into()));

    // The orchestrator's own decision answers the agent.
    let seat = self::seat(&core, &conversation, "worker:t2");
    let done = settle(&core, &seat, dialog(COMMAND), true);
    let (request, _) = open_request(&core, &conversation);
    core.decide_approval_tool(&conversation, &json!({"request":request,"choice":"1"}))
        .unwrap();
    let (verdict, _) = done.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(verdict.unwrap(), Verdict::Choose("1".into()));
}

#[test]
fn the_deciding_roles_are_never_left_to_the_orchestrator() {
    let (_directory, core, conversation) = setup();
    core.set_approval_mode(&conversation, "autonomous").unwrap();
    let seat = seat(&core, &conversation, crate::lean::PLANNER);
    let _done = settle(&core, &seat, dialog(COMMAND), true);
    assert_eq!(open_request(&core, &conversation).1, "user");
}

#[test]
fn a_dialog_that_left_the_screen_closes_the_approval_without_keys() {
    let (_directory, core, conversation) = setup();
    let seat = seat(&core, &conversation, "worker:t1");
    let done = settle(&core, &seat, dialog(COMMAND), false);
    let (verdict, _) = done.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(verdict.unwrap(), Verdict::AnsweredInTerminal);
    let states: Vec<String> = core
        .store
        .db()
        .prepare("SELECT state FROM approvals")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    assert_eq!(states, ["closed"]);
}

#[test]
fn cancelling_the_turn_stops_the_wait_and_closes_the_approval() {
    let (_directory, core, conversation) = setup();
    let seat = seat(&core, &conversation, "worker:t1");
    let cancel = Arc::new(AtomicBool::new(false));
    let (worker_core, worker_cancel) = (core.clone(), cancel.clone());
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let verdict =
            worker_core.settle_approval(&seat, &dialog(COMMAND), &worker_cancel, &|| true);
        let _ = send.send(verdict);
    });
    let (request, _) = open_request(&core, &conversation);
    cancel.store(true, Ordering::SeqCst);
    // Nothing is pressed; the session's wait loop then sees the cancel.
    let verdict = receive.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(verdict.unwrap(), Verdict::AnsweredInTerminal);
    assert_eq!(core.store.approval_state(&request).unwrap().0, "closed");
}

#[test]
fn trusting_a_directory_the_runtime_allocated_is_answered_without_asking() {
    let (_directory, core, conversation) = setup();
    let mut seat = seat(&core, &conversation, "worker:t1");
    let approval = dialog(TRUST);
    assert_eq!(approval.kind, Kind::Trust);
    // The dialog wraps the long path at the terminal width.
    seat.directories = vec![PathBuf::from(
        "/private/tmp/claude-501/-Users-sebastian-workspace-architect/e7c1fb6e-229f-4b4d-a487-d28e922fb074/scratchpad/work/claude",
    )];
    let verdict = core
        .settle_approval(&seat, &approval, &AtomicBool::new(false), &|| true)
        .unwrap();
    let trust = approval
        .choices
        .iter()
        .find(|choice| choice.decision == Decision::Allow)
        .unwrap();
    assert_eq!(verdict, Verdict::Choose(trust.id.clone()));
    let asked: i64 = core
        .store
        .db()
        .query_row("SELECT count(*) FROM approvals", [], |row| row.get(0))
        .unwrap();
    assert_eq!(asked, 0);
    // Any other directory is a decision for the user.
    seat.directories = vec![PathBuf::from("/somewhere/else")];
    let done = settle(&core, &seat, approval, true);
    open_request(&core, &conversation);
    drop(done);
}

#[test]
fn trust_approval_does_not_match_a_sibling_prefix_or_ambiguous_target() {
    let (_directory, core, conversation) = setup();
    let mut seat = seat(&core, &conversation, "worker:t1");
    let mut approval = dialog(TRUST);
    let allocated = PathBuf::from("/runtime/work/project");
    seat.directories = vec![allocated.clone()];

    approval.summary = "/runtime/work/project-evil".into();
    assert!(automatic_choice(&seat, &approval).is_none());
    let done = settle(&core, &seat, approval.clone(), true);
    let (request, decider) = open_request(&core, &conversation);
    assert_eq!(decider, "user");
    core.answer_approval(&conversation, &request, "1").unwrap();
    let (verdict, _) = done.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(verdict.unwrap(), Verdict::Choose("1".into()));

    approval.summary = format!("{}\n/runtime/work/project-evil", allocated.display());
    assert!(automatic_choice(&seat, &approval).is_none());

    approval.summary = allocated.to_string_lossy().into_owned();
    assert!(automatic_choice(&seat, &approval).is_some());
}

#[test]
fn a_restart_closes_the_approvals_nobody_waits_for_any_more() {
    let (_directory, core, conversation) = setup();
    let seat = seat(&core, &conversation, "worker:t1");
    let _done = settle(&core, &seat, dialog(COMMAND), true);
    let (request, _) = open_request(&core, &conversation);
    core.store.interrupt_running_turns().unwrap();
    assert_eq!(core.store.approval_state(&request).unwrap().0, "closed");
    let resolved = core
        .store
        .events_since(&conversation, 0, 100)
        .unwrap()
        .into_iter()
        .filter(|event| event.kind == "approval_resolved")
        .count();
    assert_eq!(resolved, 1);
}
