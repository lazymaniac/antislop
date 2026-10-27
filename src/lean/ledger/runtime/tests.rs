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
    for (repairs, reject_review) in [(true, false), (false, false), (true, true)] {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("analyst");
        std::fs::write(&script, r#"#!/usr/bin/env python3
import json,sys,uuid,os
prompt=sys.argv[-1]
lines=prompt.rsplit('Run and ledger inputs (stable identities; keep their versions):\n',1)[1].splitlines()
run=json.loads(lines[0])
assert 'selected option details' in prompt
if prompt.startswith('Independently compare'):
    assert 'may legitimately be absent' in prompt
    answer='```verdict\n'+json.dumps({'approved':not os.path.exists('reject-review'),'findings':[{'issue':'missing qualifier'}] if os.path.exists('reject-review') else []})+'\n```'
else:
    assert 'parent references another requirement' in prompt
    assert 'ORCHESTRATOR_ROUTE_ONLY' not in prompt
    n=int(open('attempts').read()) if os.path.exists('attempts') else 0
    open('attempts','w').write(str(n+1))
    if n == 1:
        assert 'entry p has unknown parent missing' in prompt, prompt
        assert 'Previous response:' in prompt
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
        if reject_review {
            std::fs::write(dir.path().join("reject-review"), "").unwrap();
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
            "2"
        );
        let run = core.store.latest_run(&conversation).unwrap().unwrap();
        let entries = core
            .store
            .ledger_entries(&conversation, &run.id, None)
            .unwrap();
        if reject_review {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("missing qualifier")
            );
            assert_eq!(entries.len(), 2);
            assert_eq!(
                core.store
                    .count_since_user(&conversation, "requirements-reviewer")
                    .unwrap(),
                1
            );
            let findings = core
                .store
                .requirements_review_findings(&conversation, &run.id)
                .unwrap();
            let findings: serde_json::Value = serde_json::from_str(&findings).unwrap();
            assert_eq!(findings[0]["issue"], "missing qualifier");
            let repeated = core
                .ensure_requirements(&conversation, &AtomicBool::new(false))
                .unwrap_err();
            assert!(
                repeated.to_string().contains("missing qualifier"),
                "unchanged entries must not create a scope approval: {repeated}"
            );
            assert_eq!(
                core.store
                    .count_since_user(&conversation, "requirements-reviewer")
                    .unwrap(),
                2
            );
            assert_eq!(
                std::fs::read_to_string(dir.path().join("attempts")).unwrap(),
                "3"
            );
            for _ in 0..tasks::FORMAT_REPAIR_ATTEMPTS + 1 {
                assert!(
                    core.ensure_requirements(&conversation, &AtomicBool::new(false))
                        .is_err()
                );
            }
            assert_eq!(
                core.store
                    .count_since_requirements_reset(&conversation, "requirements-reviewer")
                    .unwrap(),
                tasks::FORMAT_REPAIR_ATTEMPTS as usize + 3
            );
            let (sessions, turns): (i64, i64) = core.store.db().query_row(
                "SELECT count(DISTINCT session_id),count(*) FROM turns WHERE conversation_id=?1 AND role='requirements-extraction-analyst'",
                [&conversation], |row| Ok((row.get(0)?, row.get(1)?)),
            ).unwrap();
            assert!(turns > 1);
            assert_eq!(
                sessions, turns,
                "each analyst attempt must receive a fresh provider context"
            );
        } else if repairs {
            result.unwrap();
            assert_eq!(entries.len(), 2);
            assert_eq!(run.spec_revision, 1);
            assert!(
                core.store
                    .extraction_current(&conversation, &run.id)
                    .unwrap()
            );
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
