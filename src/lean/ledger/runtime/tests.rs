use super::*;
use crate::config::Config;
use std::os::unix::fs::PermissionsExt;

#[test]
fn full_sources_keep_source_question_and_summary_order() {
    let dir = tempfile::tempdir().unwrap();
    let config = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/antislop.toml"
    ))
    .unwrap();
    let core = Core::open(
        &dir.path().join("state"),
        Config::from_toml(&config).unwrap(),
    )
    .unwrap();
    let conversation = core
        .store
        .create_conversation(dir.path().to_str().unwrap())
        .unwrap()
        .id;
    let question = core
        .store
        .add_message(
            &conversation,
            "orchestrator",
            "Which option should be used?",
        )
        .unwrap();
    let answer = core
        .store
        .add_message(
            &conversation,
            "user",
            "Interview answer (focus)\nUse option B",
        )
        .unwrap();
    let run = core
        .store
        .create_run(&conversation, "Build the selected option")
        .unwrap();
    core.store
        .register_message_source(&conversation, &run.id, answer.seq)
        .unwrap();
    core.store
        .interview_summary(&conversation, "The user selected option B.", answer.seq)
        .unwrap();

    let rendered = core.full_ledger_sources(&conversation, &run.id).unwrap();
    let question_context = rendered
        .find("Interview question context for source")
        .unwrap();
    let source = rendered.find("Full user source").unwrap();
    let summary = rendered.find("Interview summary (derived context").unwrap();
    assert!(question_context < source && source < summary);
    assert!(rendered.contains(&question.text));
    assert!(rendered.contains("Use option B"));
    assert!(rendered.contains("The user selected option B."));
}

#[test]
fn extraction_repairs_invalid_parent_once_and_preserves_atomicity() {
    for repairs in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("analyst");
        std::fs::write(&script, r#"#!/usr/bin/env python3
import json,sys,uuid,os
prompt=sys.argv[-1]
lines=prompt.rsplit('Run and ledger inputs (stable identities; keep their versions):\n',1)[1].splitlines()
run=json.loads(lines[0])
assert 'selected option details' in prompt
if True:
    assert 'parent references another requirement' in prompt
    assert 'ORCHESTRATOR_ROUTE_ONLY' not in prompt
    n=int(open('attempts').read()) if os.path.exists('attempts') else 0
    open('attempts','w').write(str(n+1))
    if n >= 1:
        # submit_ledger refused the first submission within the turn.
        assert 'entry p has unknown parent missing' in prompt, prompt
    source=json.loads(lines[1].removeprefix('Sources: '))[0]['id']
    base={'source':{'source_id':source,'start':0,'end':4},'interpretation':'Keep scope','criteria':[{'id':'r.c','text':'Check scope','mandatory':True,'integration':True}]}
    r=dict(base,id='r')
    p=dict(base,id='p',parent='r' if n and os.path.exists('repair') else 'missing',criteria=[{'id':'p.c','text':'Check task','mandatory':True}])
    answer='```ledger\n'+json.dumps({'run_id':run['id'],'expected_spec_revision':run['spec_revision'],'entries':[r,p]})+'\n```'
print(json.dumps({'type':'thread.started','thread_id':str(uuid.uuid4())}))
print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
print(json.dumps({'type':'turn.completed','usage':{}}))
"#).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        crate::lean::native::fixture::interactive(&script);
        if repairs {
            std::fs::write(dir.path().join("repair"), "").unwrap();
        }
        let config = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/antislop.toml"
        ))
        .unwrap()
        .replace(
            "binary = \"codex\"",
            &format!("binary = {:?}", script.to_str().unwrap()),
        )
        .replace("provider = \"agy\"", "provider = \"codex\"");
        let core = Core::open(
            &dir.path().join("state"),
            Config::from_toml(&config).unwrap(),
        )
        .unwrap();
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        core.store
            .add_message(&conversation, "orchestrator", "selected option details")
            .unwrap();
        core.store
            .add_message(&conversation, "user", "Interview answer (focus)\nKeep")
            .unwrap();
        // Orchestration routing text must never become analyst instructions.
        core.store.add_message(&conversation, "requirements-approval",
                "ORCHESTRATOR_ROUTE_ONLY: Return the complete task plan; do not emit a ledger block.").unwrap();
        let result = core.ensure_requirements(&conversation, &AtomicBool::new(false));
        assert!(
            dir.path().join("attempts").exists(),
            "analyst did not run: {result:?}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("attempts")).unwrap(),
            if repairs { "2" } else { "3" }
        );
        let run = core.store.latest_run(&conversation).unwrap().unwrap();
        let entries = core
            .store
            .ledger_entries(&conversation, &run.id, None)
            .unwrap();
        if repairs {
            result.unwrap();
            assert_eq!(entries.len(), 2);
            assert_eq!(run.spec_revision, 1);
            assert!(
                core.store
                    .extraction_current(&conversation, &run.id)
                    .unwrap()
            );
            let verifiers: i64 = core
                .store
                .db()
                .query_row(
                    "SELECT count(*) FROM turns WHERE role='requirements-extraction-verifier'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(verifiers, 0, "no separate source-coverage review runs");
        } else {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("entry p has unknown parent missing")
            );
            assert!(entries.is_empty());
            assert_eq!(run.spec_revision, 0);
        }
    }
}

#[test]
fn an_answer_to_an_execution_decision_is_not_a_requirements_source() {
    let dir = tempfile::tempdir().unwrap();
    let config = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/antislop.toml"
    ))
    .unwrap();
    let core = Core::open(
        &dir.path().join("state"),
        Config::from_toml(&config).unwrap(),
    )
    .unwrap();
    let conversation = core
        .store
        .create_conversation(dir.path().to_str().unwrap())
        .unwrap()
        .id;
    let request = core
        .store
        .add_message(&conversation, "user", "Refactor the crate")
        .unwrap();
    let run = core.store.create_run(&conversation, "Refactor").unwrap();
    let question = crate::lean::interview::Question {
        id: "pick-source".into(),
        context: "Two sources exist.".into(),
        question: "Which?".into(),
        options: vec![
            crate::lean::interview::Choice {
                label: "A".into(),
                description: String::new(),
            },
            crate::lean::interview::Choice {
                label: "B".into(),
                description: String::new(),
            },
        ],
    };
    core.store
        .ask_user_decision(&conversation, &question)
        .unwrap();
    core.store
        .add_message(
            &conversation,
            "user",
            "Interview answer (pick-source)\nQuestion: Which?\nAnswer: A",
        )
        .unwrap();
    let other = core
        .store
        .add_message(&conversation, "user", "Also keep the CLI unchanged")
        .unwrap();
    let user = core.store.messages_by(&conversation, "user", 0).unwrap();
    assert!(
        core.register_missing_sources(&conversation, &run, &user)
            .unwrap()
    );
    let sources: Vec<_> = core
        .store
        .ledger_sources(&conversation, &run.id)
        .unwrap()
        .into_iter()
        .filter_map(|source| source.message)
        .collect();
    assert!(sources.contains(&request.seq) || run.start_message > request.seq);
    assert!(sources.contains(&other.seq));
    assert!(
        !sources.contains(&(other.seq - 1)),
        "the decision answer stays out of the sources: {sources:?}"
    );
}
