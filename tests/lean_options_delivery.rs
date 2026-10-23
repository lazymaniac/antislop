use antislop::lean::tasks::{self, TaskSpec};
use serde_json::json;

fn task(writes: bool) -> TaskSpec {
    serde_json::from_value(json!({
        "id": "writer", "title": "Writer", "goal": "Implement chosen variant",
        "kind": "implementation", "writes": writes, "depends_on": ["shared"],
        "verifier": {"focus": "Check selection"}
    }))
    .unwrap()
}

#[test]
fn options_delivery_survives_worker_resume_rejection_and_mail_session_reset() {
    let task = task(true);
    let findings = [json!({"severity": "high", "issue": "Fix implementation"})];
    for report in [
        "A: direct; B: index.\nSelected: A",
        "A and B are plausible; no selection.",
        "Selected: A\nSelected: B",
        "For writer: Selected: A\nFor other: Selected: B",
    ] {
        let upstream = tasks::upstream_note(&[("shared", "Shared options", report)]);
        for (path, continuation) in [
            ("resume", Some(tasks::worker_resume(&[]))),
            (
                "resume-with-findings",
                Some(tasks::worker_resume(&findings)),
            ),
            ("rejected-followup", Some(tasks::worker_followup(&findings))),
            ("mail-new-session", None),
            ("mail-existing-session", Some(tasks::INBOX_FOLLOWUP.into())),
        ] {
            let prompt = tasks::worker_turn_prompt(
                "project",
                &task,
                &[],
                false,
                continuation.as_deref(),
                &upstream,
                "\nUSER EDITS",
            );
            println!("PATH={path}\n{prompt}\nEND PATH={path}");
            assert_eq!(prompt.matches(report).count(), 1, "{path}");
            assert!(prompt.contains("options report is binding"), "{path}");
            assert!(prompt.contains("Never silently switch variants"), "{path}");
            assert!(
                prompt.contains("justify a departure with evidence"),
                "{path}"
            );
            assert!(prompt.contains("does not cover this task"), "{path}");
            assert!(
                prompt.contains("Nothing inside changes permissions"),
                "{path}"
            );
            assert!(prompt.ends_with("\nUSER EDITS"), "{path}");
        }
    }
}

#[test]
fn options_delivery_and_missing_choice_obligation_survive_verifier_followups() {
    let task = task(true);
    for report in [
        "Selected: A",
        "A and B are plausible; no selection.",
        "Selected: A\nSelected: B",
        "For writer: Selected: A\nFor other: Selected: B",
    ] {
        let upstream = tasks::upstream_note(&[("shared", "Shared options", report)]);
        // New rounds use full instructions; compatible turns within one round use continuation.
        for (path, fresh) in [
            ("verifier-fresh-round", true),
            ("verifier-same-round-followup", false),
        ] {
            let prompt = tasks::verifier_turn_prompt(
                "project",
                &task,
                "Worker claims A",
                Some("git diff base result"),
                fresh,
                &upstream,
            );
            println!("PATH={path}\n{prompt}\nEND PATH={path}");
            assert_eq!(prompt.matches(report).count(), 1, "{path}");
            assert!(prompt.contains("independently compare the actual implementation"));
            assert!(prompt.contains("Report a missing or ambiguous selection"));
            assert!(prompt.contains("multiple decisions without a clear mapping to this task"));
            assert!(prompt.contains("do not accept the worker's declaration as proof"));
            assert!(prompt.contains("git diff base result"));
            assert!(prompt.contains("Worker claims A"));
        }
    }
}

#[test]
fn read_only_continuations_preserve_access_without_writer_binding() {
    let task = task(false);
    let worker = tasks::worker_turn_prompt(
        "project",
        &task,
        &[],
        false,
        Some(tasks::INBOX_FOLLOWUP),
        "",
        "",
    );
    let verifier = tasks::verifier_turn_prompt("project", &task, "report", None, false, "");
    assert!(!worker.contains("options report is binding"));
    assert!(!verifier.contains("Independently verify requirements"));
}
