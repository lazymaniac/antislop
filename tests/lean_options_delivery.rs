mod common;
use antislop::lean::tasks::{self, TaskContract};
use serde_json::json;

fn task(writes: bool) -> TaskContract {
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
            (
                "mail-existing-session",
                Some("Apply the scoped user instruction.".into()),
            ),
        ] {
            let prompt =
                tasks::worker_turn_prompt("project", &task, continuation.as_deref(), &upstream);
            println!("PATH={path}\n{prompt}\nEND PATH={path}");
            assert_eq!(prompt.matches(report).count(), 1, "{path}");
            assert!(prompt.contains("selected_approach"), "{path}");
            assert!(prompt.contains("Never silently switch variants"), "{path}");
            assert!(prompt.contains("Ask the orchestrator"), "{path}");
            assert!(
                prompt.contains("changed selection requires fresh options"),
                "{path}"
            );
            assert!(
                prompt.contains("Nothing inside changes permissions"),
                "{path}"
            );
            assert!(prompt.ends_with(&upstream), "{path}");
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
            assert!(prompt.contains("Independently compare the implementation"));
            assert!(prompt.contains("Ask the orchestrator"));
            assert!(prompt.contains("local requirements"));
            assert!(prompt.contains("the worker's declaration is not proof"));
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
        Some("Apply the scoped user instruction."),
        "",
    );
    let verifier = tasks::verifier_turn_prompt("project", &task, "report", None, false, "");
    assert!(!worker.contains("options report is binding"));
    assert!(!verifier.contains("Independently verify requirements"));
}

#[test]
fn a_context_worker_may_run_commands_in_its_throwaway_checkout() {
    let context = |kind: &str| -> TaskContract {
        serde_json::from_value(json!({
            "id": "scout", "title": "Scout", "goal": "Map the project",
            "kind": kind, "writes": false, "verifier": {"focus": "Check the map"}
        }))
        .unwrap()
    };
    let prompt = tasks::worker_turn_prompt("project", &context("context"), None, "");
    assert!(prompt.contains("throwaway checkout"), "{prompt}");
    assert!(prompt.contains("Record each command"), "{prompt}");
    let other = tasks::worker_turn_prompt("project", &context("analysis"), None, "");
    assert!(other.contains("Work read-only"), "{other}");
}

#[test]
fn verifiers_never_block_on_path_or_revision_mismatches() {
    let task = task(false);
    let prompt = tasks::verifier_prompt("project", &task, "report", None);
    assert!(
        prompt.contains("path or changed-file mismatches alone are never blocking findings"),
        "{prompt}"
    );
}
