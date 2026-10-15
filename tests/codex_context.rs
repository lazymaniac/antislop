use architect::{
    config::RoleKind,
    domain::{Run, RunSpec},
    providers::{Provider, ProviderEvent, TurnRequest, codex::CodexProvider},
    sessions::*,
    storage::Store,
};
use serde_json::json;
use std::{
    os::unix::fs::PermissionsExt,
    sync::{Arc, atomic::AtomicBool},
};

fn fixture(root: &std::path::Path, mode: &str) -> (Run, CodexProvider) {
    let binary = root.join("codex-fixture");
    let source = r#"#!/usr/bin/env python3
import sys,json,pathlib,subprocess,tomllib,re,hashlib,sqlite3
if '--version' in sys.argv: print('codex-cli 0.162.1');sys.exit(0)
root=pathlib.Path(__file__).parent
mode=MODE
prompt=sys.stdin.read()
assert len(prompt)<10000
assert '--ignore-user-config' in sys.argv and '--ignore-rules' in sys.argv
config={}
for i,arg in enumerate(sys.argv[:-1]):
 if arg=='-c': config.update(tomllib.loads(sys.argv[i+1]))
server=config['mcp_servers']['antislop-context']
assert server['enabled_tools']==['read_context'] and server['required']
assert server['tools']['read_context']['output_token_limit']>=30000
root.joinpath('launched').write_text(prompt)
def emit(v):print(json.dumps(v),flush=True)
emit(dict(type='thread.started',thread_id='00000000-0000-4000-8000-000000000001'))
emit(dict(type='turn.started'))
proxy=subprocess.Popen([server['command']]+server['args'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
counter=0
def rpc(method,params):
 global counter
 counter+=1
 proxy.stdin.write(json.dumps(dict(jsonrpc='2.0',id=counter,method=method,params=params))+'\n');proxy.stdin.flush()
 line=proxy.stdout.readline()
 assert line, 'context pipe closed'
 return json.loads(line)['result']
rpc('initialize',dict(protocolVersion='2025-06-18',capabilities={},clientInfo=dict(name='fixture',version='1')))
proxy.stdin.write(json.dumps(dict(jsonrpc='2.0',method='notifications/initialized'))+'\n');proxy.stdin.flush()
tool=rpc('tools/list',{})['tools'][0]
bundle=tool['inputSchema']['properties']['bundle_hash']['const']
index=re.search(r'index document ([0-9a-f]{64})',tool['description']).group(1)
def document(digest,partial=False):
 offset=0;body=''
 while True:
  args=dict(bundle_hash=bundle,document_hash=digest,offset=offset,max_bytes=16384)
  item=dict(id=str(counter),type='mcp_tool_call',server='foreign' if mode=='foreign' else 'antislop-context',tool='read_context',arguments=args,status='in_progress')
  emit(dict(type='item.started',item=item))
  if mode=='uncommitted':
   item.update(status='completed',result=dict(content=[dict(type='text',text='{}')]))
   emit(dict(type='item.completed',item=item));sys.exit(0)
  result=rpc('tools/call',dict(name='read_context',arguments=args))
  page=json.loads(result['content'][0]['text'])
  db=root/'context.db'
  if db.exists():
   with sqlite3.connect('file:'+str(db)+'?mode=ro',uri=True) as conn:
    saved=json.loads(conn.execute('SELECT state FROM runs').fetchone()[0])
   assert saved['session_ledger']['turns']['turn']['context']['receipts'][-1]==page['receipt']
  if mode=='forged':
   page['content']='fabricated';result['content'][0]['text']=json.dumps(page)
  if mode=='changed-arguments':item['arguments']=dict(args,offset=offset+1)
  if mode=='missing-body':result={}
  item.update(status='completed',result=result,error=None)
  event=dict(type='item.completed',item=item);emit(event)
  if mode=='duplicate':emit(event)
  body+=page['content']
  if page['next_offset'] is None or partial:break
  offset=page['next_offset']
 return body
manifest=json.loads(document(index))
doc=next(d for d in manifest['documents'] if 'prepared-prompt' in d['source_refs'])
original=document(doc['document_hash'],mode=='incomplete')
if mode=='success':assert hashlib.sha256(original.encode()).hexdigest()==doc['document_hash']
proxy.stdin.close();proxy.wait()
emit(dict(type='item.completed',item=dict(id='answer',type='agent_message',text='{"ok":true}')))
emit(dict(type='turn.completed'))
"#.replace("MODE", &serde_json::to_string(mode).unwrap());
    std::fs::write(&binary, source).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.to_string_lossy().into();
    (
        Run::new(architect::id(), spec).unwrap(),
        CodexProvider {
            binary: Some(binary.to_string_lossy().into()),
            pinned_context: true,
            ..Default::default()
        },
    )
}
fn request<'a>(run: &'a Run, prompt: &'a str, schema: &'a serde_json::Value) -> TurnRequest<'a> {
    TurnRequest {
        turn_id: "turn",
        run,
        prompt,
        schema,
        resume: None,
    }
}

#[test]
fn codex_reader_commits_before_delivery_and_preserves_large_context_on_restart() {
    let root = tempfile::tempdir().unwrap();
    let (run, provider) = fixture(root.path(), "success");
    let prompt = "Original immutable instructions źródło 🦀\n".repeat(27000);
    assert!(prompt.chars().count() > 1_048_576);
    let schema = json!({"type":"object"});
    let mut ledger = SessionLedger::default();
    let database = root.path().join("context.db");
    let mut store = Store::open(&database).unwrap();
    let mut persist = |next: &SessionLedger, kind: &str| {
        let mut saved = run.clone();
        saved.session_ledger = next.clone();
        store.save(&saved, kind, None)
    };
    let scope = SessionScope::for_run(
        &run,
        "orchestrator".into(),
        None,
        RoleKind::Orchestrator,
        "codex".into(),
        "codex-cli 0.162.1".into(),
    )
    .unwrap();
    let session = SessionRecord::new("logical".into(), scope).unwrap();
    let turn = TurnRecord::new(
        "turn".into(),
        session.id.clone(),
        architect::hash(&prompt),
        architect::hash(serde_json::to_vec(&schema).unwrap()),
        None,
    )
    .unwrap();
    ledger.reserve(session, turn, &mut persist).unwrap();
    let receipt = provider
        .turn_receipt(
            &request(&run, &prompt, &schema),
            Arc::new(AtomicBool::new(false)),
            &mut |e| ledger.record_provider_event("turn", e, &mut persist),
        )
        .unwrap();
    assert!(receipt.completed);
    let context = ledger.turns["turn"].context.as_ref().unwrap();
    context.complete().unwrap();
    assert!(context.receipts.len() > 70);
    assert_eq!(context.receipts.len(), context.observed.len());
    assert_eq!(
        context.bundle.documents[&architect::hash(&prompt)].utf8,
        prompt
    );
    let saved = Store::open(&database).unwrap().get(&run.id).unwrap();
    saved.session_ledger.validate_for_run(&run.id).unwrap();
    assert_eq!(
        saved.session_ledger.turns["turn"].context,
        ledger.turns["turn"].context
    );
    let mut tampered = saved.session_ledger.clone();
    tampered.sessions.get_mut("logical").unwrap().scope.provider = "agy".into();
    assert!(tampered.validate().is_err());
}

#[test]
fn codex_refuses_incomplete_forged_foreign_replayed_or_uncommitted_context() {
    for mode in [
        "incomplete",
        "forged",
        "foreign",
        "duplicate",
        "uncommitted",
        "changed-arguments",
        "missing-body",
    ] {
        let root = tempfile::tempdir().unwrap();
        let (run, provider) = fixture(root.path(), mode);
        let mut events = vec![];
        let result = provider.turn_receipt(
            &request(&run, &"original source\n".repeat(3000), &json!({})),
            Arc::new(AtomicBool::new(false)),
            &mut |e| {
                events.push(e.clone());
                Ok(())
            },
        );
        assert!(result.is_err(), "{mode}");
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ProviderEvent::Completed { .. })),
            "{mode}"
        );
        if mode == "incomplete" {
            assert!(events.iter().any(
                |e| matches!(e,ProviderEvent::OutputRejected{output,..} if output=="{\"ok\":true}")
            ));
        }
    }
}

#[test]
fn codex_reader_stops_on_failed_commit_and_prelaunch_cancellation() {
    for fail_prepared in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let (run, provider) = fixture(root.path(), "success");
        let mut observed = false;
        assert!(
            provider
                .turn_receipt(
                    &request(&run, "source", &json!({})),
                    Arc::new(AtomicBool::new(false)),
                    &mut |e| {
                        observed |= matches!(e, ProviderEvent::ContextObserved { .. });
                        if matches!(e, ProviderEvent::ContextPrepared { .. }) && fail_prepared
                            || matches!(e, ProviderEvent::ContextRead { .. }) && !fail_prepared
                        {
                            return Err(architect::Error::Invalid(
                                "simulated persistence failure".into(),
                            ));
                        }
                        Ok(())
                    }
                )
                .is_err()
        );
        assert!(!observed);
        assert_eq!(root.path().join("launched").exists(), !fail_prepared);
    }
    let root = tempfile::tempdir().unwrap();
    let (run, provider) = fixture(root.path(), "success");
    let cancellation = Arc::new(AtomicBool::new(false));
    assert!(
        provider
            .turn_receipt(
                &request(&run, "source", &json!({})),
                cancellation.clone(),
                &mut |e| {
                    if matches!(e, ProviderEvent::ContextPrepared { .. }) {
                        cancellation.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    Ok(())
                }
            )
            .is_err()
    );
    assert!(!root.path().join("launched").exists());
}

#[test]
fn codex_reader_refuses_unvalidated_exact_resume_before_startup() {
    let root = tempfile::tempdir().unwrap();
    let (run, provider) = fixture(root.path(), "success");
    let schema = json!({});
    let mut request = request(&run, "source", &schema);
    request.resume = Some("00000000-0000-4000-8000-000000000001");
    let result = provider.turn_receipt(&request, Arc::new(AtomicBool::new(false)), &mut |_| {
        panic!("must not deliver")
    });
    assert!(
        matches!(result,Err(architect::Error::Provider(error)) if error.kind==architect::providers::FailureKind::Unsupported && error.stopped && !error.partial_effects && error.session_id.is_none())
    );
    assert!(!root.path().join("launched").exists());
}
