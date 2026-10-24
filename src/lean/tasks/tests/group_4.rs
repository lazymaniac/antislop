use super::*;

#[test]
fn plans_that_write_get_the_required_quality_gates() {
    let answer = "```plan\n{\"tasks\":[\
            {\"id\":\"t1\",\"title\":\"A\",\"goal\":\"g\",\"scope\":[\"src/a.rs\"],\"writes\":true,\"kind\":\"implementation\",\"verifier\":{\"focus\":\"f\"}},\
            {\"id\":\"t2\",\"title\":\"T\",\"goal\":\"g\",\"scope\":[\"tests/\"],\"writes\":true,\"depends_on\":[\"t1\"],\"kind\":\"testing\",\"verifier\":{\"focus\":\"f\"}}\
        ]}\n```";
    let tasks = parse_plan(answer).1.unwrap().unwrap();
    let ids: Vec<_> = missing_quality_agents(&tasks)
        .iter()
        .map(|a| a.id)
        .collect();
    assert_eq!(
        ids,
        vec![
            "quality-refactoring",
            "static-analyzer",
            "security-auditor",
            "performance-auditor",
            "integration-gate"
        ]
    );
    let full = with_options(with_required_agents(tasks));
    assert_eq!(full.len(), 12);
    validate(&full).unwrap();
    let gate = |kind| full.iter().find(|task| task.kind == kind).unwrap();
    let refactor = gate(WorkKind::Refactoring);
    assert_eq!(
        refactor.depends_on,
        vec!["t1", "t2", "quality-refactoring-options"]
    );
    let lint = gate(WorkKind::StaticAnalysis);
    assert_eq!(
        lint.depends_on,
        vec!["t1", "t2", "quality-refactoring", "static-analyzer-options"]
    );
    assert_eq!(lint.scope, vec!["src/a.rs", "tests/"]);
    assert!(!gate(WorkKind::Security).writes);
    assert_eq!(
        gate(WorkKind::Integration).depends_on,
        vec![
            "t1",
            "t2",
            "quality-refactoring",
            "static-analyzer",
            "security-auditor",
            "performance-auditor",
            "integration-gate-options"
        ]
    );
    assert!(missing_quality_agents(&full).is_empty());

    // A read-only plan needs no gates.
    let read_only = "```plan\n{\"tasks\":[{\"id\":\"t1\",\"title\":\"a\",\"goal\":\"b\",\
            \"kind\":\"analysis\",\"verifier\":{\"focus\":\"c\"}}]}\n```";
    let tasks = parse_plan(read_only).1.unwrap().unwrap();
    assert_eq!(with_required_agents(tasks).len(), 1);
}

#[test]
fn plans_with_unknown_dependencies_are_rejected() {
    let answer = "```plan\n{\"tasks\":[{\"id\":\"t1\",\"title\":\"a\",\"goal\":\"b\",\
            \"depends_on\":[\"t9\"],\"kind\":\"analysis\",\"verifier\":{\"focus\":\"c\"}}]}\n```";
    let (_, plan) = parse_plan(answer);
    assert!(plan.unwrap().unwrap_err().to_string().contains("t9"));
    assert!(parse_plan("no plan here").1.is_none());
    let looped = "```plan\n{\"tasks\":[{\"id\":\"t1\",\"title\":\"a\",\"goal\":\"b\",\
            \"depends_on\":[\"t1\"],\"kind\":\"analysis\",\"verifier\":{\"focus\":\"c\"}}]}\n```";
    assert!(parse_plan(looped).1.unwrap().is_err());
}

#[test]
fn message_blocks_are_taken_out_of_the_answer() {
    let answer = "Report.\n```message\n{\"to\": \"t2\", \"kind\": \"finding\", \"text\": \"UTF-16\"}\n```\n\
                      More.\n```message\n{\"to\": \"all\", \"text\": \"heads up\"}\n```\n\
                      ```message\nnot json\n```";
    let (text, messages) = parse_messages(answer);
    assert_eq!(
        messages
            .iter()
            .map(|message| (
                message.to.as_str(),
                message.kind.as_str(),
                message.text.as_str()
            ))
            .collect::<Vec<_>>(),
        [("t2", "finding", "UTF-16"), ("all", "status", "heads up")]
    );
    assert_eq!(text, "Report.\n\nMore.\n\n```message\nnot json\n```");
}

#[test]
fn backticks_inside_a_message_do_not_cut_it() {
    let answer =
        "Report.\n```message\n{\"to\": \"t14\", \"text\": \"last ```effect fence wins\"}\n```";
    let (text, messages, unreadable) = parse_messages_checked(answer);
    assert!(unreadable.is_empty());
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].text, "last ```effect fence wins");
    assert_eq!(text, "Report.");
    let (_, none, bad) = parse_messages_checked("```message\nnot json\n```");
    assert!(none.is_empty());
    assert_eq!(bad, ["not json"]);
}

#[test]
fn login_and_limit_errors_wait_for_the_user() {
    assert!(needs_user_action("Error: Not logged in. Run `agy login`."));
    assert!(needs_user_action("429 Too Many Requests"));
    assert!(needs_user_action("You've hit your usage limit"));
    assert!(!needs_user_action("exit status 1: panicked at src/main.rs"));
}
