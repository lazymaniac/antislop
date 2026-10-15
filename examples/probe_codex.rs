//! Real-provider contract smoke test; explicitly invoked, never run by cargo test.
use architect::{Result, domain::*, providers::codex::CodexProvider};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, atomic::AtomicBool},
};
fn main() -> Result<()> {
    let root = std::env::current_dir()?;
    let run = Run::new(
        architect::id(),
        RunSpec {
            team_template: None,
            config: None,
            task_family: architect::quality::TaskFamily::Software,
            objective: "Protocol smoke".into(),
            workspace: root.to_string_lossy().into(),
            provider: "codex".into(),
            model: None,
            requirements: vec![Requirement {
                id: "R1".into(),
                original_text: "Return structured arithmetic".into(),
                source_ref: "smoke".into(),
                criteria: BTreeMap::from([("result".into(), "Exact result".into())]),
            }],
            plan_items: vec![PlanItem {
                dependencies: BTreeSet::new(),
                id: "P1".into(),
                requirement_ids: BTreeSet::from(["R1".into()]),
                description: "Protocol smoke".into(),
            }],
            input_files: vec![],
            editable_files: vec![],
            check_command: vec![],
        },
    )?;
    let provider = CodexProvider::default();
    let schema = serde_json::json!({"type":"object","properties":{"answer":{"type":"integer"}},"required":["answer"],"additionalProperties":false});
    let (session, output) = provider.turn(
        &run,
        "Remember the number 37. Return answer=37 as JSON. Do not use tools.",
        &schema,
        None,
        Arc::new(AtomicBool::new(false)),
        &mut |_| Ok(()),
    )?;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output)?["answer"],
        37
    );
    let (resumed, output) = provider.turn(
        &run,
        "Return the remembered number plus 5 as JSON. Do not use tools.",
        &schema,
        Some(&session),
        Arc::new(AtomicBool::new(false)),
        &mut |_| Ok(()),
    )?;
    assert_eq!(session, resumed);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output)?["answer"],
        42
    );
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let result = provider.turn(
        &run,
        "Return answer=1 as JSON; no tools.",
        &schema,
        None,
        cancelled,
        &mut |_| {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        },
    );
    assert!(
        result.is_err(),
        "cancellation must interrupt a live attempt"
    );
    println!(
        "structured_output=observed; session_id=observed; exact_id_resume=observed; context_retained=observed; process_cancellation=observed"
    );
    std::fs::write("docs/m0/codex-session.json", serde_json::json!({"version":"0.162.1","structured_output":"observed","exact_id_resume":"observed","context_retained":"observed","process_cancellation":"observed","streaming":"observed","steering":"unsupported by selected transport"}).to_string())?;
    Ok(())
}
