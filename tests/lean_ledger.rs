mod support;
use antislop::lean::{
    ledger::*,
    store::{Store, TurnStatus},
    tasks::TaskSpec,
};
use serde_json::json;

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    conversation: String,
    run: Run,
    source: Source,
    text: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("core.db")).unwrap();
        let conversation = store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        let text = "Keep all qualifiers, except offline. Never send secrets. 👩‍💻e\u{301}".to_owned();
        let message = store.add_message(&conversation, "user", &text).unwrap();
        let run = store.create_run(&conversation, "deliver scope").unwrap();
        let source = store
            .register_message_source(&conversation, &run.id, message.seq)
            .unwrap();
        Self {
            dir,
            store,
            conversation,
            run,
            source,
            text,
        }
    }
    fn entry(&self, id: &str) -> EntrySpec {
        serde_json::from_value(json!({"id":id,"kind":"requirement","source":{"source_id":self.source.id,"start":0,"end":self.text.len()},"interpretation":format!("Deliver {id} with all qualifiers"),"qualifiers":["except offline","never send secrets"],"criteria":[{"id":format!("{id}.accept"),"text":"independently reproduce the acceptance check","mandatory":true}],"task_ids":[]})).unwrap()
    }
    fn change(&self, revision: i64, entries: Vec<EntrySpec>) -> LedgerChange {
        LedgerChange {
            conversation: self.conversation.clone(),
            run_id: self.run.id.clone(),
            expected_spec_revision: revision,
            operation_id: antislop::id(),
            reason: "user changes the scope".into(),
            entries,
        }
    }
    fn apply(&self, change: &LedgerChange) -> ChangeResult {
        self.store
            .apply_ledger_change(change, self.dir.path(), true)
            .unwrap()
    }
    fn task(
        &self,
        id: &str,
        requirements: &[&str],
        plan_items: &[&str],
        depends: &[&str],
    ) -> serde_json::Value {
        json!({"id":id,"title":id,"goal":"implement and check scope","kind":"implementation","verifier":{"focus":"actual behavior"},"requirement_ids":requirements,"criterion_ids":requirements.iter().map(|id|format!("{id}.accept")).collect::<Vec<_>>(),"plan_item_ids":plan_items,"depends_on":depends})
    }
    fn plan(&self, specs: Vec<serde_json::Value>) -> Vec<antislop::lean::store::Task> {
        let tasks = self.store.add_plan(&self.conversation, &specs).unwrap();
        self.store
            .link_plan_run(&self.conversation, &tasks[0].plan_id, &self.run.id)
            .unwrap();
        self.store
            .bind_task_ledger(&self.conversation, &self.run.id, &tasks[0].plan_id)
            .unwrap();
        tasks
    }
    fn turn(&self, phase: &str, verifier: bool) -> String {
        let role = format!(
            "requirements-{phase}-{}",
            if verifier { "verifier" } else { "analyst" }
        );
        let turn = self
            .store
            .start_turn(&self.conversation, &role, "mock", 0, None)
            .unwrap();
        self.store.set_session(&turn.id, &antislop::id()).unwrap();
        self.store
            .finish_turn(&turn.id, TurnStatus::Done, None)
            .unwrap();
        let revision = self
            .store
            .run(&self.conversation, &self.run.id)
            .unwrap()
            .spec_revision;
        let sources = self
            .store
            .ledger_sources(&self.conversation, &self.run.id)
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>();
        self.store
            .bind_ledger_turn(
                &self.conversation,
                &self.run.id,
                &turn.id,
                revision,
                phase,
                &sources,
            )
            .unwrap();
        turn.id
    }
    fn review(&self, phase: &str, fp: &str, approved: bool) {
        let producer = self.turn(phase, false);
        let verifier = self.turn(phase, true);
        let revision = self
            .store
            .run(&self.conversation, &self.run.id)
            .unwrap()
            .spec_revision;
        self.store
            .record_ledger_review(
                &self.conversation,
                &self.run.id,
                revision,
                phase,
                &producer,
                &verifier,
                fp,
                approved,
                "compared every source and reproduced checks",
            )
            .unwrap();
    }
    fn assessment(&self, task: &antislop::lean::store::Task, id: &str, fp: &str) -> Assessment {
        Assessment {
            evidence: EvidenceProposal {
                entry_id: id.into(),
                entry_version: 1,
                criterion_id: format!("{id}.accept"),
                task_id: task.id.clone(),
                generation: task.input_generation,
                attempt: task.round,
                producer_turn: self.turn("audit", false),
                result: "cargo test acceptance: 7 passed, 0 failed; inspected integrated files"
                    .into(),
                artifact: None,
                integrated_fingerprint: fp.into(),
            },
            approved: true,
            note: "reproduced test and inspected all outputs".into(),
        }
    }
    fn approved(
        &self,
        mut tasks: Vec<antislop::lean::store::Task>,
    ) -> Vec<antislop::lean::store::Task> {
        for t in &mut tasks {
            t.status = "approved".into();
            t.round = 1;
            self.store.update_task(t).unwrap();
        }
        tasks
    }
}
#[test]
fn immutable_sources_grapheme_spans_and_document_tampering() {
    let f = Fixture::new();
    let valid = f.entry("r1");
    f.apply(&f.change(0, vec![valid.clone()]));
    let entry = f
        .store
        .ledger_entries(&f.conversation, &f.run.id, None)
        .unwrap()
        .remove(0);
    assert_eq!(entry.original, f.text);
    let mut split = valid.clone();
    split.source.end = f.text.len() - 1;
    assert!(
        f.store
            .preview_ledger_change(&f.change(1, vec![split]), f.dir.path())
            .is_err()
    );
    let doc = f
        .store
        .register_document_source(
            &f.conversation,
            &f.run.id,
            "middle obligation",
            f.dir.path(),
        )
        .unwrap();
    assert_eq!(
        f.store
            .ledger_source_text(&f.conversation, &f.run.id, &doc.id, f.dir.path())
            .unwrap(),
        "middle obligation"
    );
    std::fs::write(f.dir.path().join(doc.locator.unwrap()), "changed").unwrap();
    assert!(
        f.store
            .ledger_source_text(&f.conversation, &f.run.id, &doc.id, f.dir.path())
            .is_err()
    );
}
#[test]
fn revision_conflict_idempotency_history_and_user_authority() {
    let f = Fixture::new();
    let first = f.change(0, vec![f.entry("r1")]);
    let saved = f.apply(&first);
    assert_eq!(f.apply(&first), saved);
    let mut collision = first.clone();
    collision.reason = "different".into();
    assert!(
        f.store
            .apply_ledger_change(&collision, f.dir.path(), true)
            .is_err()
    );
    let mut edited = f.entry("r1");
    edited.mandatory = false;
    let change = f.change(1, vec![edited.clone()]);
    assert!(
        f.store
            .apply_ledger_change(&change, f.dir.path(), false)
            .is_err()
    );
    let (id, diff) = f
        .store
        .propose_ledger_change(&change, f.dir.path())
        .unwrap();
    let hash = diff_hash(&diff).unwrap();
    f.store.check_ledger_proposal(&id, &hash, &change).unwrap();
    let mut wrong = change.clone();
    wrong.entries[0].interpretation = "silent weakening".into();
    assert!(f.store.check_ledger_proposal(&id, &hash, &wrong).is_err());
    f.apply(&change);
    assert!(
        f.store
            .apply_ledger_change(&f.change(1, vec![edited]), f.dir.path(), true)
            .is_err()
    );
    let history = f
        .store
        .ledger_entry_history(&f.conversation, &f.run.id, "r1")
        .unwrap();
    assert_eq!(history.len(), 2);
    assert!(history[0].spec.mandatory);
    assert!(!history[1].spec.mandatory);
    drop(f.store);
    let reopened = Store::open(&f.dir.path().join("core.db")).unwrap();
    assert_eq!(
        reopened
            .apply_ledger_change(&change, f.dir.path(), true)
            .unwrap()
            .spec_revision,
        2
    );
}
#[test]
fn semantic_edit_invalidates_dependency_closure_but_keeps_independent_tasks() {
    let f = Fixture::new();
    f.apply(&f.change(0, vec![f.entry("r1"), f.entry("r2")]));
    let tasks = f.plan(vec![
        f.task("a", &["r1"], &[], &[]),
        f.task("b", &["r2"], &[], &["a"]),
        f.task("c", &["r2"], &[], &[]),
    ]);
    let mut edited = f.entry("r1");
    edited.qualifiers.push("new qualifier".into());
    let result = f.apply(&f.change(1, vec![edited]));
    assert_eq!(result.affected_tasks.len(), 2);
    assert!(result.affected_tasks.contains(&tasks[0].id));
    assert!(result.affected_tasks.contains(&tasks[1].id));
    assert!(!result.affected_tasks.contains(&tasks[2].id));
    assert!(f.store.update_task(&tasks[0]).is_err());
    assert!(f.store.assert_task_inputs(&tasks[0]).is_err());
    f.store.update_task(&tasks[2]).unwrap();
    f.store.assert_task_inputs(&tasks[2]).unwrap();
    drop(f.store);
    let store = Store::open(&f.dir.path().join("core.db")).unwrap();
    assert_eq!(
        store.task_by_id(&tasks[0].id).unwrap().unwrap().status,
        "stale"
    );
}
#[test]
fn parent_requires_children_and_its_own_integration_evidence() {
    let f = Fixture::new();
    let mut parent = f.entry("parent");
    parent.criteria[0].integration = true;
    let mut child = f.entry("child");
    child.parent = Some("parent".into());
    f.apply(&f.change(0, vec![parent, child]));
    let tasks = f.approved(f.plan(vec![f.task("a", &["parent", "child"], &[], &[])]));
    let verifier = f.turn("audit", true);
    f.store
        .add_ledger_evidence(
            &f.conversation,
            &f.run.id,
            &verifier,
            &f.assessment(&tasks[0], "parent", "integrated"),
        )
        .unwrap();
    let status = f
        .store
        .ledger_statuses(&f.conversation, &f.run.id, None, "integrated")
        .unwrap();
    assert_ne!(
        status
            .iter()
            .find(|e| e.spec.id == "parent")
            .unwrap()
            .status,
        "verified"
    );
    f.store
        .add_ledger_evidence(
            &f.conversation,
            &f.run.id,
            &verifier,
            &f.assessment(&tasks[0], "child", "integrated"),
        )
        .unwrap();
    assert!(
        f.store
            .ledger_statuses(&f.conversation, &f.run.id, None, "integrated")
            .unwrap()
            .iter()
            .all(|e| e.status == "verified")
    );
    assert!(
        f.store
            .ledger_statuses(&f.conversation, &f.run.id, None, "changed")
            .unwrap()
            .iter()
            .all(|e| e.status != "verified")
    );
}
#[test]
fn evidence_rejects_self_verification_empty_claims_and_stale_inputs() {
    let f = Fixture::new();
    f.apply(&f.change(0, vec![f.entry("r1")]));
    let tasks = f.approved(f.plan(vec![f.task("a", &["r1"], &[], &[])]));
    let verifier = f.turn("audit", true);
    let mut a = f.assessment(&tasks[0], "r1", "tree");
    assert!(
        f.store
            .add_ledger_evidence(&f.conversation, &f.run.id, &a.evidence.producer_turn, &a)
            .is_err()
    );
    a.evidence.result = "done".into();
    assert!(
        f.store
            .add_ledger_evidence(&f.conversation, &f.run.id, &verifier, &a)
            .is_err()
    );
    a.evidence.result = "test checks passed with measured outputs".into();
    a.evidence.generation = 99;
    assert!(
        f.store
            .add_ledger_evidence(&f.conversation, &f.run.id, &verifier, &a)
            .is_err()
    );
    a.evidence.generation = 0;
    a.evidence.entry_version = 0;
    assert!(
        f.store
            .add_ledger_evidence(&f.conversation, &f.run.id, &verifier, &a)
            .is_err()
    );
}
#[test]
fn completion_gate_requires_source_coverage_current_audit_and_integrated_evidence() {
    let f = Fixture::new();
    f.apply(&f.change(0, vec![f.entry("r1")]));
    let tasks = f.approved(f.plan(vec![f.task("a", &["r1"], &[], &[])]));
    assert!(
        !f.store
            .requirements_gaps(&f.conversation, &f.run.id, "tree")
            .unwrap()
            .is_empty()
    );
    f.review("extraction", "", true);
    f.review("audit", "tree", true);
    let verifier = f.turn("audit", true);
    f.store
        .add_ledger_evidence(
            &f.conversation,
            &f.run.id,
            &verifier,
            &f.assessment(&tasks[0], "r1", "tree"),
        )
        .unwrap();
    assert!(
        f.store
            .requirements_gaps(&f.conversation, &f.run.id, "tree")
            .unwrap()
            .is_empty()
    );
    f.review("audit", "tree", false);
    assert!(
        !f.store
            .requirements_gaps(&f.conversation, &f.run.id, "tree")
            .unwrap()
            .is_empty()
    );
    let message = f
        .store
        .add_message(&f.conversation, "user", "Obligation hidden in a new source")
        .unwrap();
    f.store
        .register_message_source(&f.conversation, &f.run.id, message.seq)
        .unwrap();
    assert!(
        !f.store
            .extraction_current(&f.conversation, &f.run.id)
            .unwrap()
    );
}
#[test]
fn technical_steps_need_goal_checks_tasks_and_cannot_be_silently_withdrawn() {
    let f = Fixture::new();
    let mut step = f.entry("step");
    step.kind = LedgerKind::PlanItem;
    step.task_ids = vec!["task-step".into()];
    step.technical_goal = Some("verify build assumptions".into());
    step.check = Some("inspect compiler output".into());
    let mut invalid = step.clone();
    invalid.check = None;
    assert!(
        f.store
            .preview_ledger_change(&f.change(0, vec![invalid]), f.dir.path())
            .is_err()
    );
    f.apply(&f.change(0, vec![step.clone()]));
    let specs = f.task("task-step", &[], &["step"], &[]);
    let _: TaskSpec = serde_json::from_value(specs.clone()).unwrap();
    f.plan(vec![specs]);
    step.disposition = Disposition::WithdrawnByUser;
    let mut change = f.change(1, vec![step]);
    change.reason.clear();
    assert!(
        f.store
            .apply_ledger_change(&change, f.dir.path(), true)
            .is_err()
    );
    change.reason = "user no longer requires this technical step".into();
    f.apply(&change);
    assert_eq!(
        f.store
            .ledger_statuses(&f.conversation, &f.run.id, None, "")
            .unwrap()[0]
            .status,
        "withdrawn_by_user"
    );
}
#[test]
fn schema_six_migrates_and_preserves_legacy_history_without_verification() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.db");
    let store = Store::open(&path).unwrap();
    let conversation = store.create_conversation("project").unwrap();
    let tasks = store
        .add_plan(
            &conversation.id,
            &[json!({"id":"old","title":"Historical","kind":"analysis"})],
        )
        .unwrap();
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.pragma_update(None, "foreign_keys", false).unwrap();
    let tables=db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND (name LIKE 'ledger_%' OR name IN ('runs','plan_runs','task_inputs','task_ledger_inputs'))").unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<std::result::Result<Vec<_>,_>>().unwrap();
    for table in tables {
        db.execute_batch(&format!("DROP TABLE {table}")).unwrap();
    }
    db.pragma_update(None, "user_version", 6).unwrap();
    drop(db);
    let store = Store::open(&path).unwrap();
    assert!(store.plan_run(&tasks[0].plan_id).unwrap().is_none());
    assert_eq!(store.plan(&tasks[0].plan_id).unwrap(), tasks);
    let version = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
        .unwrap();
    assert!(
        version >= 7,
        "ledger migration must be installed without downgrading newer migrations"
    );
}

#[test]
fn live_scope_edit_cancels_only_affected_process_and_rejects_late_task_updates() {
    use antislop::{config::Config, lean::Core};
    use std::time::{Duration, Instant};
    let f = Fixture::new();
    f.apply(&f.change(0, vec![f.entry("r1"), f.entry("r2")]));
    let tasks = f.plan(vec![
        f.task("a", &["r1"], &[], &[]),
        f.task("b", &["r2"], &[], &[]),
    ]);
    f.review("extraction", "", true);
    let script = f.dir.path().join("fixture-codex");
    std::fs::write(&script,r#"#!/usr/bin/env python3
import json,sys,uuid,os,time,re
prompt=sys.argv[-1]
if 'Task id:' in prompt and 'You verify' not in prompt:
    task='a' if 'Task id: a\n' in prompt else 'b'
    open('entered-'+task,'w').close()
    while not os.path.exists('release-'+task):time.sleep(0.01)
    answer='Concrete report for '+task
elif 'You verify' in prompt or 'You audit' in prompt:
    ids=re.findall(r'^- ([RCV][0-9]+): ',prompt,re.M)
    answer='```verdict\n'+json.dumps({'approved':True,'findings':[],'coverage':{i:{'status':'ok','note':'independently checked'} for i in ids}})+'\n```'
else:answer='Scope remains open; request executor work for evidence.'
session=str(uuid.uuid4())
print(json.dumps({'type':'thread.started','thread_id':session}))
print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
print(json.dumps({'type':'turn.completed','usage':{}}))
"#).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/antislop.toml"
    ))
    .unwrap()
    .replace(
        "binary = \"codex\"",
        &format!("binary = {:?}", script.to_str().unwrap()),
    )
    .replace("provider = \"agy\"", "provider = \"codex\"")
    .replace("required_gates = true", "required_gates = false");
    std::fs::set_permissions(f.dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let core = Core::open(f.dir.path(), Config::from_toml(&config).unwrap()).unwrap();
    core.accept_plan(&f.conversation, &tasks[0].plan_id)
        .unwrap();
    let wait = |done: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for processes");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    wait(&|| f.dir.path().join("entered-a").exists() && f.dir.path().join("entered-b").exists());
    let mut edited = f.entry("r1");
    edited.interpretation.push_str(" with a new qualifier");
    let change = f.change(1, vec![edited]);
    let (proposal, diff) = core
        .store
        .propose_ledger_change(&change, f.dir.path())
        .unwrap();
    let hash = diff_hash(&diff).unwrap();
    let result = core
        .apply_user_ledger_change(&proposal, &hash, &change)
        .unwrap();
    assert_eq!(result.affected_tasks, vec![tasks[0].id.clone()]);
    std::fs::write(f.dir.path().join("release-b"), "").unwrap();
    wait(&|| core.is_idle(&f.conversation));
    let after = core.store.plan(&tasks[0].plan_id).unwrap();
    assert_eq!(after[0].status, "stale");
    assert_eq!(after[1].status, "approved");
    assert!(core.store.assert_task_inputs(&tasks[0]).is_err());
    assert!(core.store.assert_task_inputs(&after[1]).is_ok());
    assert_eq!(
        core.store
            .last_turn(&f.conversation, &format!("worker:{}", tasks[0].id))
            .unwrap()
            .unwrap()
            .status,
        TurnStatus::Cancelled
    );
    // Lost response: retrying the exact operation returns the same revision.
    assert_eq!(
        core.apply_user_ledger_change(&proposal, &hash, &change)
            .unwrap(),
        result
    );
}

#[test]
fn compliance_sessions_verify_integrated_files_and_reopen_after_file_changes() {
    use antislop::{config::Config, lean::Core};
    use std::{os::unix::fs::PermissionsExt, process::Command, sync::atomic::AtomicBool};
    let f = Fixture::new();
    f.apply(&f.change(0, vec![f.entry("r1")]));
    let tasks = f.approved(f.plan(vec![f.task("a", &["r1"], &[], &[])]));
    f.review("extraction", "", true);
    std::fs::write(f.dir.path().join(".gitignore"), "*\n").unwrap();
    std::fs::write(f.dir.path().join("observed-result"), "delivered").unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(f.dir.path())
            .args([
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@example.invalid",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    };
    git(&["init", "-b", "main"]);
    git(&["add", "-f", "observed-result", ".gitignore"]);
    git(&["commit", "-m", "baseline"]);
    let script = f.dir.path().join("fixture-audit");
    std::fs::write(&script,r#"#!/usr/bin/env python3
import json,sys,uuid,re,os
prompt=sys.argv[-1]
def block(tag,data):return '```'+tag+'\n'+json.dumps(data)+'\n```'
if prompt.startswith('Audit the integrated project'):
    assert open('observed-result').read()=='delivered'
    fingerprint=re.search(r'Integrated fingerprint: ([a-f0-9]+)',prompt).group(1)
    tasks,_=json.JSONDecoder().raw_decode(prompt.split('Task inputs and reports: ',1)[1])
    lines=prompt.rsplit('Run and ledger inputs (stable identities; keep their versions):\n',1)[1].splitlines()
    entries=json.loads(lines[2].removeprefix('Entries: '))
    evidence=[]
    for entry in entries:
        spec=entry['spec']
        task=next(t for t in tasks if spec['id'] in t['spec']['requirement_ids']+t['spec']['plan_item_ids'])
        for criterion in spec['criteria']:
            evidence.append({'entry_id':spec['id'],'entry_version':entry['version'],'criterion_id':criterion['id'],'task_id':task['id'],'generation':task['input_generation'],'attempt':task['round'],'producer_turn':'','result':'Read observed-result in integrated project: exact expected content delivered','artifact':None,'integrated_fingerprint':fingerprint})
    answer=block('compliance',{'approved':not os.path.exists('audit-gap'),'findings':[{'issue':'obligation omitted during extraction'}] if os.path.exists('audit-gap') else [],'evidence':evidence})
elif prompt.startswith('Verify this audit independently'):
    if os.path.exists('hold-audit-verifier'):
        import time
        open('audit-verifier-entered','w').close()
        while not os.path.exists('release-audit-verifier'):time.sleep(0.01)
    assert open('observed-result').read()=='delivered'
    proposals,_=json.JSONDecoder().raw_decode(prompt.split('Proposed evidence: ',1)[1])
    answer=block('compliance_verdict',{'approved':not os.path.exists('audit-gap'),'note':'Read all sources and reproduced every integrated observation','assessments':[{'evidence':e,'approved':True,'note':'Independently read integrated observed-result; content matches delivered'} for e in proposals]})
else:answer=block('verdict',{'approved':True,'findings':[]})
session=str(uuid.uuid4())
print(json.dumps({'type':'thread.started','thread_id':session}))
print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
print(json.dumps({'type':'turn.completed','usage':{}}))
"#).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(f.dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
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
    let core = Core::open(f.dir.path(), Config::from_toml(&config).unwrap()).unwrap();
    let cancel = AtomicBool::new(false);
    let report = core
        .plan_report(&f.conversation, &tasks[0].plan_id, &cancel)
        .unwrap();
    let fp = core.ledger_fingerprint(&f.conversation).unwrap();
    let gaps = core
        .store
        .requirements_gaps(&f.conversation, &f.run.id, &fp)
        .unwrap();
    assert!(gaps.is_empty(), "{gaps:?}\n{report}");
    assert!(report.contains("fully delivered"));
    assert_eq!(core.conversations().unwrap()[0].status, "done");
    std::fs::write(f.dir.path().join("audit-gap"), "omitted obligation").unwrap();
    let report = core
        .plan_report(&f.conversation, &tasks[0].plan_id, &cancel)
        .unwrap();
    assert!(report.contains("gate is CLOSED"));
    assert_eq!(
        core.conversations().unwrap()[0].status,
        "requirements_pending"
    );
    // Scope publication can win while the auditor's verifier is still running.
    std::fs::write(f.dir.path().join("hold-audit-verifier"), "").unwrap();
    let auditing = core.clone();
    let conversation = f.conversation.clone();
    let plan = tasks[0].plan_id.clone();
    let running = std::thread::spawn(move || {
        auditing
            .plan_report(&conversation, &plan, &AtomicBool::new(false))
            .unwrap()
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !f.dir.path().join("audit-verifier-entered").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mut edited = f.entry("r1");
    edited
        .qualifiers
        .push("scope published during audit".into());
    let change = f.change(1, vec![edited]);
    let (proposal, diff) = core
        .store
        .propose_ledger_change(&change, f.dir.path())
        .unwrap();
    core.apply_user_ledger_change(&proposal, &diff_hash(&diff).unwrap(), &change)
        .unwrap();
    std::fs::write(f.dir.path().join("release-audit-verifier"), "").unwrap();
    assert!(running.join().unwrap().contains("gate is CLOSED"));
    assert!(
        core.store
            .ledger_statuses(&f.conversation, &f.run.id, None, &fp)
            .unwrap()
            .iter()
            .all(|e| e.status != "verified")
    );
    std::fs::write(f.dir.path().join("observed-result"), "changed after audit").unwrap();
    assert_ne!(core.ledger_fingerprint(&f.conversation).unwrap(), fp);
    assert!(
        core.store
            .ledger_statuses(
                &f.conversation,
                &f.run.id,
                None,
                &core.ledger_fingerprint(&f.conversation).unwrap()
            )
            .unwrap()
            .iter()
            .all(|e| e.status != "verified")
    );
}

#[test]
fn long_document_preserves_middle_negative_obligation_without_copying_document_into_sqlite() {
    let f = Fixture::new();
    let prefix = "Background context without obligations.\n".repeat(4000);
    let obligation = "Never upload secrets, including offline caches, except a user-approved redacted sample. 👩‍💻";
    let text = format!(
        "{prefix}{obligation}\n{}",
        "Trailing context.\n".repeat(4000)
    );
    let source = f
        .store
        .register_document_source(&f.conversation, &f.run.id, &text, f.dir.path())
        .unwrap();
    let mut entry = f.entry("middle");
    entry.source = SourceSpan {
        source_id: source.id,
        start: prefix.len(),
        end: prefix.len() + obligation.len(),
    };
    entry.qualifiers = vec![
        "including offline caches".into(),
        "except a user-approved redacted sample".into(),
    ];
    f.apply(&f.change(0, vec![entry]));
    let saved = f
        .store
        .ledger_entries(&f.conversation, &f.run.id, None)
        .unwrap()
        .remove(0);
    assert_eq!(saved.original, obligation);
    assert_eq!(saved.spec.qualifiers.len(), 2);
    let db = rusqlite::Connection::open(f.dir.path().join("core.db")).unwrap();
    let duplicate: String = db
        .query_row(
            "SELECT original FROM ledger_versions WHERE entry_id='middle'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(duplicate.is_empty());
    assert_eq!(saved.spec.source.lines(&text).unwrap().0, 4001);
}
#[test]
fn concurrent_clients_cannot_overwrite_the_same_expected_revision() {
    use std::sync::{Arc, Barrier};
    let f = Fixture::new();
    f.apply(&f.change(0, vec![f.entry("r1")]));
    let mut a = f.entry("r1");
    a.interpretation = "client A".into();
    let mut b = a.clone();
    b.interpretation = "client B".into();
    let changes = [f.change(1, vec![a]), f.change(1, vec![b])];
    let store = Arc::new(Store::open(&f.dir.path().join("core.db")).unwrap());
    let barrier = Arc::new(Barrier::new(2));
    let results = std::thread::scope(|scope| {
        let handles = changes
            .iter()
            .map(|change| {
                let barrier = barrier.clone();
                let store = store.clone();
                let state = f.dir.path();
                scope.spawn(move || {
                    barrier.wait();
                    store.apply_ledger_change(change, state, true)
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        store.run(&f.conversation, &f.run.id).unwrap().spec_revision,
        2
    );
    assert_eq!(
        store
            .ledger_entry_history(&f.conversation, &f.run.id, "r1")
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn analyst_authority_is_bound_to_a_turn_and_proposals_are_idempotent() {
    let f = Fixture::new();
    f.apply(&f.change(0, vec![f.entry("r1")]));
    let change = f.change(1, vec![f.entry("r2")]);
    let impostor = f.turn("audit", false);
    assert!(
        f.store
            .apply_analyst_ledger_change(&change, f.dir.path(), &impostor)
            .is_err()
    );
    let analyst = f.turn("extraction", false);
    f.store
        .apply_analyst_ledger_change(&change, f.dir.path(), &analyst)
        .unwrap();
    assert!(
        !f.store
            .extraction_current(&f.conversation, &f.run.id)
            .unwrap()
    );
    let mut weakened = f.entry("r1");
    weakened.mandatory = false;
    let change = f.change(2, vec![weakened]);
    let analyst = f.turn("extraction", false);
    assert!(
        f.store
            .apply_analyst_ledger_change(&change, f.dir.path(), &analyst)
            .is_err()
    );
    let first = f
        .store
        .propose_agent_ledger_change(&change, f.dir.path(), &analyst)
        .unwrap();
    let repeated = f
        .store
        .propose_agent_ledger_change(&change, f.dir.path(), &analyst)
        .unwrap();
    assert_eq!(first, repeated);
    let saved = f
        .store
        .latest_ledger_proposal(&f.conversation, &f.run.id)
        .unwrap();
    assert_eq!(saved["proposal"]["diff_hash"], diff_hash(&first.1).unwrap());
    f.apply(&change);
    assert_eq!(
        f.store
            .propose_agent_ledger_change(&change, f.dir.path(), &analyst)
            .unwrap(),
        first
    );
}

#[test]
fn continuation_reuses_current_approved_inputs_and_task_filter_follows_real_links() {
    let f = Fixture::new();
    f.apply(&f.change(0, vec![f.entry("r1"), f.entry("r2")]));
    f.approved(f.plan(vec![
        f.task("first", &["r1"], &[], &[]),
        f.task("second", &["r2"], &[], &[]),
    ]));
    let next = f.plan(vec![f.task("repair", &["r2"], &[], &[])]);
    let statuses = f
        .store
        .ledger_statuses(&f.conversation, &f.run.id, None, "")
        .unwrap();
    assert_eq!(
        statuses.iter().find(|e| e.spec.id == "r1").unwrap().status,
        "implemented"
    );
    assert_eq!(
        statuses.iter().find(|e| e.spec.id == "r2").unwrap().status,
        "planned"
    );
    let page = f
        .store
        .list_ledger(
            &LedgerQuery {
                conversation: f.conversation.clone(),
                run_id: f.run.id.clone(),
                task: Some(next[0].id.clone()),
                limit: Some(1),
                ..Default::default()
            },
            LedgerKind::Requirement,
            "",
        )
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.entries[0].spec.id, "r2");
}

#[test]
fn a_positive_assessment_cannot_hide_a_conflicting_result_on_the_same_inputs() {
    let f = Fixture::new();
    f.apply(&f.change(0, vec![f.entry("r1")]));
    let mut task = f
        .approved(f.plan(vec![f.task("a", &["r1"], &[], &[])]))
        .remove(0);
    let verifier = f.turn("audit", true);
    let mut failed = f.assessment(&task, "r1", "tree");
    failed.approved = false;
    failed.note = "independent check reproduced a failure".into();
    f.store
        .add_ledger_evidence(&f.conversation, &f.run.id, &verifier, &failed)
        .unwrap();
    let passing = f.assessment(&task, "r1", "tree");
    f.store
        .add_ledger_evidence(&f.conversation, &f.run.id, &verifier, &passing)
        .unwrap();
    assert_ne!(
        f.store
            .ledger_statuses(&f.conversation, &f.run.id, None, "tree")
            .unwrap()[0]
            .status,
        "verified"
    );
    task.round += 1;
    f.store.update_task(&task).unwrap();
    let passing = f.assessment(&task, "r1", "tree");
    f.store
        .add_ledger_evidence(&f.conversation, &f.run.id, &verifier, &passing)
        .unwrap();
    assert_eq!(
        f.store
            .ledger_statuses(&f.conversation, &f.run.id, None, "tree")
            .unwrap()[0]
            .status,
        "verified"
    );
}

#[test]
fn optional_criteria_alone_cannot_verify_an_obligation() {
    let f = Fixture::new();
    let mut entry = f.entry("r1");
    entry.criteria[0].mandatory = false;
    f.apply(&f.change(0, vec![entry]));
    f.review("extraction", "", true);
    f.review("audit", "tree", true);
    let entries = f
        .store
        .ledger_statuses(&f.conversation, &f.run.id, None, "tree")
        .unwrap();
    assert_ne!(entries[0].status, "verified");
    assert!(
        f.store
            .requirements_gaps(&f.conversation, &f.run.id, "tree")
            .unwrap()
            .iter()
            .any(|gap| gap.starts_with("r1:"))
    );
}

#[test]
fn source_line_ranges_respect_exclusive_end_and_graphemes() {
    let source = "first\r\n👩‍💻e\u{301}\nlast";
    let span = |start, end| SourceSpan {
        source_id: "source".into(),
        start,
        end,
    };
    let second = source.find('👩').unwrap();
    let last = source.find("last").unwrap();
    assert_eq!(span(0, second).lines(source).unwrap(), (1, 1));
    assert_eq!(span(second, last).lines(source).unwrap(), (2, 2));
    assert_eq!(span(second, source.len()).lines(source).unwrap(), (2, 3));
    assert_eq!(span(last - 1, last).lines(source).unwrap(), (2, 2));
    assert!(span(second + 4, last).lines(source).is_err());
}

#[test]
fn approving_requirements_wakes_planning_uses_the_approved_set_and_compares_options() {
    use antislop::{config::Config, lean::Core};
    use std::{
        os::unix::fs::PermissionsExt,
        time::{Duration, Instant},
    };
    for (reject_options, reject_plan) in [(false, false), (true, false), (true, true)] {
        let f = Fixture::new();
        f.apply(&f.change(0, vec![f.entry("r1")]));
        let analyst = f.turn("extraction", false);
        let mut entry = f.entry("r1");
        entry.interpretation = "Approved wording with preserved qualifiers".into();
        let change = f.change(1, vec![entry]);
        let (proposal, diff) = f
            .store
            .propose_agent_ledger_change(&change, f.dir.path(), &analyst)
            .unwrap();
        let empty = f
            .store
            .create_run(&f.conversation, "Manually maintained requirements")
            .unwrap();
        assert_eq!(
            f.store.planning_run(&f.conversation).unwrap().unwrap().id,
            f.run.id
        );
        let script = f.dir.path().join("codex");
        std::fs::write(&script,r#"#!/usr/bin/env python3
import sys,json,uuid
prompt = sys.argv[-1]
assert not prompt.startswith('Act as requirements analyst'), 'approved requirements must go directly to verification'
answer='```plan\n'+json.dumps({'tasks':[{'id':'inspect','title':'Inspect','goal':'Inspect the project','kind':'analysis','scope':['.'],'verifier':{'focus':'Check the observations'}}]})+'\n```'
if 'You review a plan' in prompt or 'new plan:' in prompt:
    import os
    if os.path.exists('reject-options'):
        assert 'Choice lacks adequate evidence' in prompt
        assert 'resolves every review finding' in prompt
    answer='```verdict\n'+json.dumps({'approved':not os.path.exists('reject-final-plan'),'findings':[{'issue':'Plan does not resolve the options review'}] if os.path.exists('reject-final-plan') else []})+'\n```'
print(json.dumps({'type':'thread.started','thread_id':str(uuid.uuid4())}))
print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
print(json.dumps({'type':'turn.completed','usage':{}}))
"#).unwrap();
        support::install_ledger_protocol(&script);
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = include_str!("../examples/antislop.toml")
            .replace("binary = \"codex\"", &format!("binary = {:?}", script))
            .replace("provider = \"agy\"", "provider = \"codex\"")
            .replace("required_gates = true", "required_gates = false");
        std::fs::set_permissions(f.dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = config.replace(
            "[routing.options]\nexecutor = \"reasoning\"",
            "[routing.options]\nexecutor = \"options_test\"",
        ) + "\n[profiles.options_test]\nprovider = \"codex\"\nmodel = \"fixture-options\"\neffort = \"low\"\n";
        if reject_options {
            std::fs::write(f.dir.path().join("reject-options"), "").unwrap();
        }
        if reject_plan {
            std::fs::write(f.dir.path().join("reject-final-plan"), "").unwrap();
        }
        let core = Core::open(f.dir.path(), Config::from_toml(&config).unwrap()).unwrap();
        let result = core
            .apply_user_ledger_change(&proposal, &diff_hash(&diff).unwrap(), &change)
            .unwrap();
        assert_eq!(result.spec_revision, 2);
        let deadline = Instant::now() + Duration::from_secs(20);
        while !core.is_idle(&f.conversation) {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        let plan = core.store.latest_plan(&f.conversation).unwrap();
        let option_turns = core
            .store
            .events_since(&f.conversation, 0, 1000)
            .unwrap()
            .into_iter()
            .filter(|e| {
                e.kind == "turn_started" && e.payload["role"] == "requirements-options-analyst"
            })
            .collect::<Vec<_>>();
        assert!(
            option_turns
                .iter()
                .all(|event| event.payload["model"] == "fixture-options")
        );
        if reject_options {
            let reviewed = core
                .store
                .solution_choice(&f.conversation, &f.run.id)
                .unwrap()
                .unwrap();
            assert!(
                reviewed
                    .review_findings
                    .iter()
                    .any(|finding| finding.contains("Choice lacks adequate evidence"))
            );
        }
        assert_eq!(option_turns.len(), 1);
        if reject_plan {
            let events = core.store.events_since(&f.conversation, 0, 2000).unwrap();
            let visible = events
                .iter()
                .rev()
                .find(|event| event.kind == "plan_proposed")
                .expect("a reviewed plan remains visible to the user");
            assert_eq!(visible.payload["review"]["approved"], false);
            assert_eq!(visible.payload["automatic"], false);
            assert!(plan.iter().all(|task| task.round == 0));
        }
        assert_eq!(
            core.store
                .count_since_user(&f.conversation, "options-plan-repair")
                .unwrap(),
            0
        );
        assert_eq!(
            core.store
                .count_since_user(&f.conversation, "requirements-planner")
                .unwrap(),
            0
        );

        assert!(
            !plan.is_empty(),
            "{:?}",
            core.store
                .events_since(&f.conversation, 0, 500)
                .unwrap()
                .iter()
                .filter(|e| matches!(
                    e.kind.as_str(),
                    "notice" | "plan_invalid" | "requirements_failed" | "ledger_plan_rejected"
                ))
                .map(|e| &e.payload)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            core.store.plan_run(&plan[0].plan_id).unwrap(),
            Some(f.run.id.clone())
        );
        assert_eq!(
            core.store
                .run(&f.conversation, &empty.id)
                .unwrap()
                .spec_revision,
            0
        );
        assert_eq!(
            core.store
                .last_turn(&f.conversation, "requirements-extraction-analyst")
                .unwrap()
                .unwrap()
                .id,
            analyst
        );
        let choice = core
            .store
            .solution_choice(&f.conversation, &f.run.id)
            .unwrap()
            .unwrap();
        assert_eq!(choice.spec_revision, 2);
        assert_eq!(choice.alternatives.len(), 2);
        assert!(
            core.store
                .last_turn(&f.conversation, "requirements-options-analyst")
                .unwrap()
                .is_some()
        );
        assert!(
            core.store
                .last_turn(&f.conversation, "requirements-options-verifier")
                .unwrap()
                .is_some()
        );
        let approval_count = core
            .store
            .messages_by(&f.conversation, "requirements-approval", 0)
            .unwrap()
            .len();
        core.apply_user_ledger_change(&proposal, &diff_hash(&diff).unwrap(), &change)
            .unwrap();
        assert_eq!(
            core.store
                .messages_by(&f.conversation, "requirements-approval", 0)
                .unwrap()
                .len(),
            approval_count
        );
        let mut amended = f.entry("r1");
        amended.interpretation.push_str(" updated");
        core.store
            .apply_ledger_change(&f.change(2, vec![amended]), f.dir.path(), true)
            .unwrap();
        assert!(
            core.store
                .solution_choice(&f.conversation, &f.run.id)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn accepted_requirements_resume_durably_after_restart_and_ignore_empty_manual_sets() {
    let f = Fixture::new();
    f.store.begin_interview(&f.conversation).unwrap();
    let user = f.store.messages_by(&f.conversation, "user", 0).unwrap()[0].seq;
    f.store
        .interview_summary(&f.conversation, "Keep scope", user)
        .unwrap();
    f.store.finish_interview(&f.conversation).unwrap();
    f.apply(&f.change(0, vec![f.entry("r1")]));
    let analyst = f.turn("extraction", false);
    let mut edited = f.entry("r1");
    edited.interpretation.push_str(" clarified");
    let change = f.change(1, vec![edited]);
    f.store
        .propose_agent_ledger_change(&change, f.dir.path(), &analyst)
        .unwrap();
    f.store
        .apply_ledger_change(&change, f.dir.path(), true)
        .unwrap();
    // Reproduce an approval saved by the old release, before it had a durable wake-up.
    let db = rusqlite::Connection::open(f.dir.path().join("core.db")).unwrap();
    db.execute("DELETE FROM requirements_resumptions", [])
        .unwrap();
    db.execute(
        "DELETE FROM messages WHERE author='requirements-approval'",
        [],
    )
    .unwrap();
    f.store
        .create_run(&f.conversation, "Manually maintained requirements")
        .unwrap();
    f.store.recover_approved_requirements().unwrap();
    f.store.recover_approved_requirements().unwrap();
    assert!(
        f.store
            .has_pending_requirements_approval(&f.conversation)
            .unwrap()
    );
    assert_eq!(
        f.store
            .messages_by(&f.conversation, "requirements-approval", 0)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        f.store.planning_run(&f.conversation).unwrap().unwrap().id,
        f.run.id
    );
    let reopened = Store::open(&f.dir.path().join("core.db")).unwrap();
    assert!(
        reopened
            .has_pending_requirements_approval(&f.conversation)
            .unwrap()
    );
}

#[test]
fn old_options_rejection_recovers_as_plan_feedback_without_repeating_agents() {
    let f = Fixture::new();
    f.store.begin_interview(&f.conversation).unwrap();
    let user = f.store.messages_by(&f.conversation, "user", 0).unwrap()[0].seq;
    f.store
        .interview_summary(&f.conversation, "Keep scope", user)
        .unwrap();
    f.store.finish_interview(&f.conversation).unwrap();
    f.store
        .select_planning_run(&f.conversation, &f.run.id)
        .unwrap();
    f.apply(&f.change(0, vec![f.entry("r1")]));
    f.review("extraction", "", true);
    let producer = f.turn("options", false);
    let verifier = f.turn("options", true);
    let comparison = json!({"run_id":f.run.id,"spec_revision":1,"criteria":["Evidence"],"alternatives":[{"id":"diagnose","approach":"Measure before selecting an optimization","evaluations":["Establishes evidence"],"risks":"Measurement may be inconclusive","validation":"Check a reproducible baseline"},{"id":"rewrite","approach":"Rewrite based on old measurements","evaluations":["Uses stale evidence"],"risks":"May optimize the wrong path","validation":"Compare against a baseline"}],"selected":"diagnose","rationale":{"why":"Fresh evidence is needed","tradeoffs":["Defers optimization until after measurement"],"reopen":{"when":"The baseline identifies another bottleneck"}}});
    f.store
        .append_event(
            &f.conversation,
            Some(&producer),
            None,
            "agent_final",
            &json!({"text":format!("```options\n{}\n```",comparison)}),
        )
        .unwrap();
    f.store
        .record_ledger_review(
            &f.conversation,
            &f.run.id,
            1,
            "options",
            &producer,
            &verifier,
            "",
            false,
            "The complete plan must specify the decision criteria and independent oracle.",
        )
        .unwrap();
    f.store.recover_options_comparisons().unwrap();
    f.store.recover_options_comparisons().unwrap();
    let choice = f
        .store
        .solution_choice(&f.conversation, &f.run.id)
        .unwrap()
        .unwrap();
    assert!(choice.rationale.contains("Fresh evidence is needed"));
    assert!(choice.rationale.contains("Defers optimization"));
    assert!(choice.rationale.contains("another bottleneck"));
    assert!(choice.review_findings[0].contains("independent oracle"));
    assert_eq!(
        f.store
            .messages_by(&f.conversation, "options-planner", 0)
            .unwrap()
            .len(),
        1
    );
    assert!(
        f.store
            .has_pending_requirements_approval(&f.conversation)
            .unwrap()
    );
}
