use architect::{
    config::{Config, IsolationProfile, PinnedConfig, ProviderKind, RoleKind, WorkKind},
    domain::{Run, RunSpec},
    providers::{
        self, FailureKind, Provider, ProviderEvent, TurnReceipt, TurnRequest, UsageReport,
        agy::{self, AgyProvider},
    },
};
use std::{
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const ID: &str = "00000000-0000-4000-8000-000000000001";
const OTHER: &str = "00000000-0000-4000-8000-000000000002";
const SECRET: &str = "PRIVATE-PLANNER-AND-DIAGNOSTIC-SENTINEL";

struct Fixture {
    root: tempfile::TempDir,
    home: PathBuf,
    run: Run,
    provider: AgyProvider,
    launched: PathBuf,
}
impl Fixture {
    fn new(mode: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&workspace).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        let launched = root.path().join("launched.json");
        let binary = root.path().join("mock-agy");
        let script = format!(
            r#"#!/usr/bin/env python3
import sys,os,json,pathlib,time,subprocess
def fixture_exception(kind,value,tb):
 frames=[]
 while tb:
  frames.append([tb.tb_frame.f_code.co_name,tb.tb_lineno]);tb=tb.tb_next
 pathlib.Path({fixture_error:?}).write_text(json.dumps(dict(frames=frames,kind=kind.__name__,message=str(value))))
sys.excepthook=fixture_exception
for name in ['NODE_OPTIONS','AGY_SYSTEM_PROMPT','GOOGLE_GEMINI_BASE_URL','GOOGLE_APPLICATION_CREDENTIALS']:
 assert name not in os.environ
assert (pathlib.Path.cwd()/'.agents/agents/antislop-stdio.md').is_file()
if '--version' in sys.argv: print('1.3.5' if {mode:?}=='version_135' or {mode:?}.startswith('context') else '99.0.0' if {mode:?}=='unsupported_version' else '1.3.4');sys.exit(0)
if '--help' in sys.argv:
 print('--input-format --output-format --json-schema --agent --disable-slash-commands --mode --sandbox --print-timeout --conversation --model --effort',file=sys.stderr if {mode:?}=='stderr_help' else sys.stdout)
 if {mode:?}=='stderr_help':sys.stdout.write('X'*100000)
 sys.exit(0)
args=sys.argv[1:]
assert args[args.index('--agent')+1]=='antislop-stdio'
assert args[args.index('--mode')+1]=='accept-edits'
assert args[args.index('--print-timeout')+1]=='0'
assert '--dangerously-skip-permissions' not in args and '--continue' not in args
profile=(pathlib.Path.cwd()/'.agents/agents/antislop-stdio.md').read_text()
for required in ['tools: [finish]','excludeDefaultComponents: true','inheritCustomizations: false',('inheritMcp: true' if {mode:?}.startswith('context') else 'inheritMcp: false'),'subagent: false']:
 assert required in profile
pathlib.Path({launched:?}).write_text(json.dumps(dict(args=args,cwd=os.getcwd())))
mode={mode:?}
if mode=='hang':
 subprocess.Popen([sys.executable,'-c',"import pathlib,time;time.sleep(2);pathlib.Path("+repr({late:?})+").write_text('orphan')"])
 time.sleep(20);sys.exit(1)
request=json.loads(sys.stdin.readline());assert request['event']=='user';assert isinstance(request['message']['content'],str)
assert sys.stdin.read()==''
import hashlib
record=json.loads(pathlib.Path({launched:?}).read_text())
content=request['message']['content'].encode('utf-8')
record.update(prompt_bytes=len(content),prompt_hash=hashlib.sha256(content).hexdigest())
pathlib.Path({launched:?}).write_text(json.dumps(record))
schema=json.loads(pathlib.Path(args[args.index('--json-schema')+1]).read_text())
identity={id:?}
if '--conversation' in args: assert args[args.index('--conversation')+1]==identity
init=dict(event='init',conversation_id=identity,init=dict(cwd=os.getcwd(),agent='antislop-stdio',tools=['finish','run_command','invoke_subagent'],permission_mode='always-proceed'))
def emit(v): print(json.dumps(v),flush=True)
emit(init)
if mode.startswith('context'):
 import re
 config=json.loads((pathlib.Path.cwd()/'.agents/mcp_config.json').read_text())['mcpServers']['antislop-context']
 proxy=subprocess.Popen([config['command']]+config['args'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
 counter=0
 def rpc(method,params):
  global counter
  counter+=1;proxy.stdin.write(json.dumps(dict(jsonrpc='2.0',id=counter,method=method,params=params))+'\n');proxy.stdin.flush()
  response=proxy.stdout.readline();assert response,'Proxy EOF at '+method+' #'+str(counter)
  return json.loads(response)['result']
 rpc('initialize',dict(protocolVersion='2025-06-18',capabilities={{}},clientInfo=dict(name='native-fixture',version='1')))
 proxy.stdin.write(json.dumps(dict(jsonrpc='2.0',method='notifications/initialized'))+'\n');proxy.stdin.flush()
 tools=rpc('tools/list',{{}})['tools'];assert len(tools)==1 and tools[0]['name']=='read_context'
 bundle=tools[0]['inputSchema']['properties']['bundle_hash']['const']
 index=re.search(r'index document ([0-9a-f]{{64}})',tools[0]['description']).group(1)
 step_index=1
 def fetch(document,offset):
  global step_index
  step_index+=1
  arguments=dict(bundle_hash=bundle,document_hash=document,offset=offset,max_bytes=16384)
  parameters=dict(ServerName=('foreign' if mode=='context-foreign' else 'antislop-context'),ToolName='read_context',Arguments=arguments)
  info=dict(name='call_mcp_tool',parameters=parameters)
  emit(dict(event='step_update',step_update=dict(conversation_id=identity,step_index=step_index,state='ACTIVE',step_type='tool',tool_name='call_mcp_tool',tool_info=info)))
  if mode=='context-no-commit':
   emit(dict(event='step_update',step_update=dict(conversation_id=identity,step_index=step_index,state='DONE',step_type='tool',tool_name='call_mcp_tool',tool_info=info)))
   time.sleep(60);sys.exit(1)
  answer=rpc('tools/call',dict(name='read_context',arguments=arguments))
  page=json.loads(answer['content'][0]['text'])
  database=pathlib.Path({database:?})
  if database.exists():
   import sqlite3
   with sqlite3.connect('file:'+str(database)+'?mode=ro',uri=True) as connection:
    saved=json.loads(connection.execute('SELECT state FROM runs').fetchone()[0])
   assert saved['session_ledger']['turns']['turn-1']['context']['receipts'][-1]==page['receipt']
  if mode=='context-forged':page['content']='fabricated'
  info['output']=json.dumps(page)
  if mode in ('context-success','context-no-echo'):info.pop('output')
  event=dict(event='step_update',step_update=dict(conversation_id=identity,step_index=step_index,state='DONE',step_type='tool',tool_name='call_mcp_tool',tool_info=info))
  emit(event)
  if mode=='context-duplicate':emit(event)
  return page
 def document(digest,partial=False):
  offset=0;body=''
  while True:
   page=fetch(digest,offset);body+=page['content']
   if page['next_offset'] is None or partial:break
   offset=page['next_offset']
  return body
 manifest=json.loads(document(index))
 source=next(d for d in manifest['documents'] if 'prepared-prompt' in d['source_refs'])
 original=document(source['document_hash'],mode=='context-incomplete')
 if mode not in ('context-incomplete','context-forged'):assert hashlib.sha256(original.encode()).hexdigest()==source['document_hash']
 proxy.stdin.close();proxy.wait()
if mode=='duplicate_init':emit(init)
if mode in ('step_output_limit','step_output_limit_payload'):
 subprocess.Popen([sys.executable,'-c',"import pathlib,time;time.sleep(2);pathlib.Path("+repr({late:?})+").write_text('late')"])
 step=dict(conversation_id=identity,state='DONE',step_type='error_message')
 step['error_message' if mode.endswith('_payload') else 'error']=dict(message={secret:?}+' Your previous response was cut off because it exceeded the output token limit. Retries remaining: 3')
 emit(dict(event='step_update',step_update=step))
 time.sleep(60)
if mode=='tool':emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type='tool',tool_name='write_to_file')))
if mode=='private_tool':emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type='tool',tool_name={secret:?},tool_info=dict(name={secret:?},parameters=dict(command={secret:?})),text_delta={secret:?})))
if mode in ('unknown_step','system_step'):emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type=('system_message' if mode=='system_step' else {secret:?}),text_delta={secret:?})))
if mode=='subagent':emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type='agent_response',subagent_info=dict(subagents=[]))))
if mode=='bad_json':print('{{"event":"result","event":"result"}}',flush=True);sys.exit(1)
emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type='agent_response',text_delta={secret:?},usage=dict(input_tokens=9999999,output_tokens=9999999))))
if mode!='no_finish':emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type='finish')))
result=dict(event='result',result=dict(conversation_id=({other:?} if mode=='wrong_id' else identity),status='SUCCESS',structured_output=dict(ok=True),json_schema=({{}} if mode=='wrong_schema' else schema),usage=dict(input_tokens=795,output_tokens=136,cache_read_tokens=0)))
if mode=='missing_output':result['result'].pop('structured_output');result['result']['response']='{{"ok":true}}'
if mode=='error':result['result'].update(status='ERROR',error={secret:?}+' authentication failed');sys.stderr.write({secret:?});emit(result);sys.exit(1)
emit(result)
if mode=='late':emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type='agent_response',text_delta={secret:?})))
if mode=='duplicate_result':emit(result)
"#,
            launched = launched.to_string_lossy(),
            mode = mode,
            late = root.path().join("late-effect").to_string_lossy(),
            id = ID,
            other = OTHER,
            secret = SECRET,
            database = root.path().join("context.db").to_string_lossy(),
            fixture_error = root.path().join("fixture-error-line").to_string_lossy()
        );
        std::fs::write(&binary, script).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut spec: RunSpec =
            serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
        spec.workspace = workspace.to_string_lossy().into();
        spec.provider = "agy".into();
        let run = Run::new(architect::id(), spec).unwrap();
        let provider = AgyProvider {
            binary: Some(binary.to_string_lossy().into()),
            managed_home: Some(home.clone()),
            model: Some("gemini-3.8-flash-low".into()),
            effort: Some("low".into()),
            isolation: IsolationProfile::ToolFree,
        };
        Self {
            root,
            home,
            run,
            provider,
            launched,
        }
    }
    fn call(
        &self,
        resume: Option<&str>,
        events: &mut Vec<ProviderEvent>,
    ) -> architect::Result<TurnReceipt> {
        self.call_prompt(
            "Read literal /skills @data; return only schema JSON",
            resume,
            events,
        )
    }
    fn call_prompt(
        &self,
        prompt: &str,
        resume: Option<&str>,
        events: &mut Vec<ProviderEvent>,
    ) -> architect::Result<TurnReceipt> {
        self.provider.turn_receipt(&TurnRequest {turn_id:"turn-1",run:&self.run,prompt,schema:&serde_json::json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}),resume},Arc::new(AtomicBool::new(false)),&mut|event| {events.push(event.clone());Ok(())})
    }
}

#[test]
fn oversized_utf8_input_is_rejected_before_probe_or_delivery() {
    let mut f = Fixture::new("success");
    // Invalid startup paths prove preflight precedes probing and home inspection.
    f.provider.binary = Some(f.root.path().join("missing-cli").to_string_lossy().into());
    f.provider.managed_home = Some(f.root.path().join("missing-home"));
    let prompt = format!("{}{}", "ą".repeat(agy::MAX_PROMPT_BYTES / 2), SECRET);
    assert!(prompt.chars().count() < agy::MAX_PROMPT_BYTES);
    let mut events = vec![];
    let architect::Error::Provider(error) = f.call_prompt(&prompt, None, &mut events).unwrap_err()
    else {
        panic!("expected typed input preflight failure");
    };
    assert_eq!(error.kind, FailureKind::Protocol);
    assert!(error.message.contains("input_too_large"));
    assert!(error.stopped && !error.partial_effects && error.session_id.is_none());
    assert!(!serde_json::to_string(&error).unwrap().contains(SECRET));
    assert!(events.is_empty() && !f.launched.exists());
}

#[test]
fn exact_adapter_byte_ceiling_reaches_cli_with_original_bytes() {
    let f = Fixture::new("success");
    let prompt = "ą".repeat(agy::MAX_PROMPT_BYTES / 2);
    assert_eq!(prompt.len(), agy::MAX_PROMPT_BYTES);
    let receipt = f.call_prompt(&prompt, None, &mut vec![]).unwrap();
    assert!(receipt.completed && receipt.delivered);
    let launched: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&f.launched).unwrap()).unwrap();
    assert_eq!(launched["prompt_bytes"], agy::MAX_PROMPT_BYTES);
    assert_eq!(launched["prompt_hash"], architect::hash(&prompt));
}

#[test]
fn scoped_context_bridge_commits_before_reply_and_authenticates_complete_native_delivery_after_sqlite_restart()
 {
    use architect::{sessions::*, storage::Store};
    let mut f = Fixture::new("context-success");
    f.provider.isolation = IsolationProfile::PinnedContext;
    let prompt = format!(
        "Complete original instructions and Unicode sources 🦀. {}",
        "original context; no invented data\n".repeat(7000)
    );
    assert!(prompt.len() > agy::MAX_PROMPT_BYTES);
    let schema = serde_json::json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false});
    let session = SessionRecord::new(
        "logical".into(),
        SessionScope::for_run(
            &f.run,
            "executor".into(),
            None,
            RoleKind::Executor,
            "agy".into(),
            "1.3.5".into(),
        )
        .unwrap(),
    )
    .unwrap();
    let turn = TurnRecord::new(
        "turn-1".into(),
        session.id.clone(),
        architect::hash(&prompt),
        architect::hash(serde_json::to_vec(&schema).unwrap()),
        None,
    )
    .unwrap();
    let mut ledger = SessionLedger::default();
    let database = f.root.path().join("context.db");
    let mut store = Store::open(&database).unwrap();
    let mut persist = |next: &SessionLedger, kind: &str| {
        let mut snapshot = f.run.clone();
        snapshot.session_ledger = next.clone();
        store.save(&snapshot, kind, None)
    };
    ledger.reserve(session, turn, &mut persist).unwrap();
    let result = f.provider.turn_receipt(
        &TurnRequest {
            turn_id: "turn-1",
            run: &f.run,
            prompt: &prompt,
            schema: &schema,
            resume: None,
        },
        Arc::new(AtomicBool::new(false)),
        &mut |event| ledger.record_provider_event("turn-1", event, &mut persist),
    );
    if result.is_err() {
        eprintln!(
            "Mock diagnostic: {}",
            std::fs::read_to_string(f.root.path().join("fixture-error-line")).unwrap_or_default()
        );
    }
    let receipt = result.unwrap();
    ledger
        .complete(
            "turn-1",
            &receipt,
            OutputRef {
                document: "turn:turn-1".into(),
                hash: architect::hash(&receipt.output),
            },
            &mut persist,
        )
        .unwrap();
    drop(store);
    let reopened = Store::open(&database).unwrap();
    let saved = reopened.get(&f.run.id).unwrap();
    saved.session_ledger.validate().unwrap();
    let context = saved.session_ledger.turns["turn-1"]
        .context
        .as_ref()
        .unwrap();
    context.complete().unwrap();
    let (covered, total) = context.progress().unwrap();
    assert_eq!(covered, total);
    assert!(covered > prompt.len());
    assert!(context.receipts.len() > 2);
    assert_eq!(context.receipts.len(), context.observed.len());
    assert_eq!(
        context.bundle.documents[&architect::hash(&prompt)].utf8,
        prompt
    );
    let sent: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&f.launched).unwrap()).unwrap();
    assert_eq!(sent["prompt_hash"], context.native_prompt_hash);
    assert!(sent["prompt_bytes"].as_u64().unwrap() < agy::MAX_PROMPT_BYTES as u64);
    assert!(!std::path::Path::new(sent["cwd"].as_str().unwrap()).exists());
    assert!(
        saved
            .session_ledger
            .validate_for_run("foreign-run")
            .is_err()
    );
    for change in [
        "source",
        "native-input",
        "missing-page",
        "unobserved",
        "scope",
    ] {
        let mut ledger = saved.session_ledger.clone();
        let c = ledger
            .turns
            .get_mut("turn-1")
            .unwrap()
            .context
            .as_mut()
            .unwrap();
        match change {
            "source" => c
                .bundle
                .documents
                .values_mut()
                .next()
                .unwrap()
                .utf8
                .push('!'),
            "native-input" => c.native_prompt_hash = architect::hash("changed"),
            "missing-page" => {
                c.receipts.pop();
            }
            "unobserved" => c.observed.clear(),
            "scope" => c.bundle.scope.input_hash = architect::hash("foreign"),
            _ => unreachable!(),
        }
        assert!(ledger.validate().is_err(), "{change}");
    }
}

#[test]
fn context_output_requires_all_observed_pages_and_refuses_forged_duplicate_or_foreign_tool_events()
{
    for mode in [
        "context-incomplete",
        "context-forged",
        "context-duplicate",
        "context-foreign",
        "context-no-commit",
    ] {
        let mut f = Fixture::new(mode);
        f.provider.isolation = IsolationProfile::PinnedContext;
        let prompt = "Original pinned source 🦀\n".repeat(3000);
        let mut events = vec![];
        let error = f.call_prompt(&prompt, None, &mut events).unwrap_err();
        assert!(
            matches!(error, architect::Error::Provider(_)),
            "{mode}: {error}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ProviderEvent::Completed { .. })),
            "{mode}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ProviderEvent::ContextPrepared { .. })),
            "{mode}"
        );
        if mode == "context-incomplete" {
            assert!(events.iter().any(
                |e| matches!(e,ProviderEvent::OutputRejected {output,..} if output=="{\"ok\":true}")
            ));
        }
    }
}

#[test]
fn context_preparation_or_page_commit_failure_stops_before_unauthorized_delivery() {
    for fail_page in [false, true] {
        let mut f = Fixture::new("context-success");
        f.provider.isolation = IsolationProfile::PinnedContext;
        let mut observed = vec![];
        let result=f.provider.turn_receipt(&TurnRequest {turn_id:"turn-1",run:&f.run,prompt:"Pinned source",schema:&serde_json::json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}),resume:None},Arc::new(AtomicBool::new(false)),&mut |event| {
            observed.push(event.clone());
            if matches!(event,ProviderEvent::ContextRead {..}) && fail_page || matches!(event,ProviderEvent::ContextPrepared {..}) && !fail_page {return Err(architect::Error::Invalid("simulated failed durable context commit".into()));}Ok(())
        });
        assert!(result.is_err());
        assert!(!observed.iter().any(|e| matches!(
            e,
            ProviderEvent::ContextObserved { .. } | ProviderEvent::Completed { .. }
        )));
        if !fail_page {
            assert!(!f.launched.exists());
        }
    }
    let mut f = Fixture::new("success");
    f.provider.isolation = IsolationProfile::PinnedContext;
    let error = f.call(None, &mut vec![]).unwrap_err();
    assert!(error.to_string().contains("1.3.5"));
    assert!(!f.launched.exists());
}

#[test]
fn cancellation_committed_during_context_preparation_does_not_launch_native_delivery() {
    let mut f = Fixture::new("context-success");
    f.provider.isolation = IsolationProfile::PinnedContext;
    let token = Arc::new(AtomicBool::new(false));
    let mut events = vec![];
    let result = f.provider.turn_receipt(
        &TurnRequest {
            turn_id: "cancel-before-delivery",
            run: &f.run,
            prompt: "Pinned source",
            schema: &serde_json::json!({"type":"object"}),
            resume: None,
        },
        token.clone(),
        &mut |event| {
            events.push(event.clone());
            if matches!(event, ProviderEvent::ContextPrepared { .. }) {
                token.store(true, Ordering::SeqCst);
            }
            Ok(())
        },
    );
    let architect::Error::Provider(error) = result.unwrap_err() else {
        panic!("expected typed cancellation");
    };
    assert_eq!(error.kind, FailureKind::Cancelled);
    assert!(error.stopped && !error.partial_effects && error.session_id.is_none());
    assert!(!f.launched.exists());
    assert!(!events.iter().any(|e| matches!(
        e,
        ProviderEvent::DeliveryPendingUnknown { .. }
            | ProviderEvent::DeliverySent { .. }
            | ProviderEvent::Completed { .. }
    )));
}

#[test]
fn success_uses_native_finish_output_once_and_resume_is_explicitly_unsupported() {
    let f = Fixture::new("success");
    {
        let mut events = Vec::new();
        let receipt = f.call(None, &mut events).unwrap();
        assert_eq!(receipt.provider, "agy");
        assert_eq!(receipt.session_id, ID);
        assert_eq!(receipt.output, "{\"ok\":true}");
        assert!(receipt.completed && receipt.delivered && receipt.stopped);
        assert!(matches!(
            receipt.usage,
            UsageReport::Reported {
                input_tokens: Some(795),
                output_tokens: Some(136),
                cumulative: true,
                ..
            }
        ));
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, ProviderEvent::Usage { .. }))
                .count(),
            1
        );
        assert!(matches!(
            events.last(),
            Some(ProviderEvent::Completed { .. })
        ));
        assert!(!serde_json::to_string(&events).unwrap().contains(SECRET));
        assert!(!serde_json::to_string(&receipt).unwrap().contains(SECRET));
    }
    let before = std::fs::read(&f.launched).unwrap();
    assert!(f.call(Some("latest"), &mut vec![]).is_err());
    let architect::Error::Provider(error) = f.call(Some(ID), &mut vec![]).unwrap_err() else {
        panic!("expected unsupported resume")
    };
    assert_eq!(error.kind, FailureKind::Unsupported);
    assert_eq!(std::fs::read(&f.launched).unwrap(), before);
}

#[test]
fn protocol_and_policy_faults_preserve_observed_identity_and_private_diagnostics() {
    for mode in [
        "duplicate_init",
        "tool",
        "private_tool",
        "unknown_step",
        "system_step",
        "subagent",
        "bad_json",
        "no_finish",
        "wrong_id",
        "wrong_schema",
        "missing_output",
        "late",
        "duplicate_result",
        "error",
    ] {
        let f = Fixture::new(mode);
        let mut events = vec![];
        let architect::Error::Provider(error) = f.call(None, &mut events).unwrap_err() else {
            panic!("expected typed failure for {mode}")
        };
        assert_eq!(error.session_id.as_deref(), Some(ID), "{mode}");
        assert!(error.stopped && error.partial_effects, "{mode}");
        if mode == "tool" || mode == "private_tool" || mode == "subagent" {
            assert_eq!(error.kind, FailureKind::Policy);
            assert!(error.message.contains("selected profile"));
            if mode == "tool" {
                assert!(error.message.contains("tool=write_to_file"));
            }
            if mode == "private_tool" {
                assert!(error.message.contains("tool=redacted"));
            }
        }
        if mode == "error" {
            assert_eq!(error.kind, FailureKind::Authentication);
        }
        if mode == "unknown_step" || mode == "system_step" {
            assert_eq!(error.kind, FailureKind::Protocol);
            assert!(error.message.contains("type_sha256="));
            assert!(error.message.contains(if mode == "system_step" {
                "step=system_message"
            } else {
                "step=redacted"
            }));
        }
        assert!(
            !serde_json::to_string(&error).unwrap().contains(SECRET),
            "{mode}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ProviderEvent::Completed { .. })),
            "{mode}"
        );
    }
}

#[test]
fn native_step_output_failure_stops_internal_retries_and_descendants_immediately() {
    for mode in ["step_output_limit", "step_output_limit_payload"] {
        let f = Fixture::new(mode);
        let start = Instant::now();
        let mut events = vec![];
        let architect::Error::Provider(error) = f.call(None, &mut events).unwrap_err() else {
            panic!("expected typed native output-limit failure")
        };
        assert!(start.elapsed() < Duration::from_secs(8));
        assert_eq!(error.kind, FailureKind::Protocol);
        assert_eq!(error.session_id.as_deref(), Some(ID));
        assert!(error.stopped && error.partial_effects);
        assert!(error.message.contains("output_token_limit_exceeded"));
        assert!(!error.message.contains(SECRET));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ProviderEvent::Completed { .. }))
        );
        std::thread::sleep(Duration::from_millis(2200));
        assert!(!f.root.path().join("late-effect").exists());
    }
}

#[test]
fn startup_hook_and_changed_owned_profile_fail_before_delivery_or_cli_turn() {
    let f = Fixture::new("success");
    let config = f.home.join(".gemini/config");
    std::fs::create_dir_all(&config).unwrap();
    let hook = config.join("hooks.json");
    std::fs::write(&hook, "{\"hook\":{\"SessionStart\":[]}}").unwrap();
    let original = std::fs::read(&hook).unwrap();
    let mut events = vec![];
    assert!(f.call(None, &mut events).is_err());
    assert!(events.is_empty());
    assert!(!f.launched.exists());
    assert_eq!(std::fs::read(&hook).unwrap(), original);
    std::fs::remove_file(hook).unwrap();
    f.call(None, &mut events).unwrap();
    let info: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&f.launched).unwrap()).unwrap();
    let profile =
        PathBuf::from(info["cwd"].as_str().unwrap()).join(".agents/agents/antislop-stdio.md");
    std::fs::write(&profile, "changed").unwrap();
    let before = std::fs::read(&f.launched).unwrap();
    events.clear();
    assert!(f.call(None, &mut events).is_err());
    assert!(events.is_empty());
    assert_eq!(std::fs::read(&f.launched).unwrap(), before);
    assert_eq!(std::fs::read_to_string(profile).unwrap(), "changed");
}

#[test]
fn cancellation_stops_mock_and_same_group_descendant() {
    let f = Fixture::new("hang");
    let cancelled = Arc::new(AtomicBool::new(false));
    let token = cancelled.clone();
    let provider = f.provider;
    let run = f.run;
    let worker = std::thread::spawn(move || {
        provider.turn_receipt(
            &TurnRequest {
                turn_id: "cancel",
                run: &run,
                prompt: "x",
                schema: &serde_json::json!({}),
                resume: None,
            },
            token,
            &mut |_| Ok(()),
        )
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    while !f.launched.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    cancelled.store(true, Ordering::SeqCst);
    let architect::Error::Provider(error) = worker.join().unwrap().unwrap_err() else {
        panic!("expected typed cancellation")
    };
    assert_eq!(error.kind, FailureKind::Cancelled);
    assert!(error.stopped && error.partial_effects);
    std::thread::sleep(Duration::from_millis(2200));
    assert!(!f.root.path().join("late-effect").exists());
}

#[test]
fn canonical_agy_configuration_routes_native_model_and_effort() {
    let f = Fixture::new("success");
    let mut config = Config::from_toml(include_str!("../examples/architect.toml")).unwrap();
    let mut native = config.providers[&ProviderKind::Codex].clone();
    native.binary = PathBuf::from(f.provider.binary.as_ref().unwrap());
    native.managed_home = Some(f.home.clone());
    native.isolation = IsolationProfile::ToolFree;
    config.providers.insert(ProviderKind::Agy, native);
    for profile in config.profiles.values_mut() {
        profile.provider = ProviderKind::Agy;
        profile.model = Some("gemini-3.8-flash-low".into());
    }
    config.validate().unwrap();
    let resolved = config
        .resolve(WorkKind::Context, RoleKind::Executor)
        .unwrap();
    let mut run = f.run.clone();
    run.spec.config = Some(PinnedConfig {
        content_hash: architect::hash(serde_json::to_vec(&config).unwrap()),
        config,
    });
    let provider = providers::build_for(&run, RoleKind::Executor).unwrap();
    provider
        .turn_receipt(
            &TurnRequest {
                turn_id: "configured",
                run: &run,
                prompt: "test",
                schema: &serde_json::json!({}),
                resume: None,
            },
            Arc::new(AtomicBool::new(false)),
            &mut |_| Ok(()),
        )
        .unwrap();
    let info: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&f.launched).unwrap()).unwrap();
    let args = info["args"].as_array().unwrap();
    let value = |flag: &str| {
        args[args.iter().position(|v| v == flag).unwrap() + 1]
            .as_str()
            .unwrap()
    };
    assert_eq!(value("--model"), resolved.model.as_ref().unwrap());
    assert_eq!(value("--effort"), resolved.native_effort.as_ref().unwrap());
    assert_eq!(
        serde_json::to_string(&ProviderKind::Agy).unwrap(),
        "\"agy\""
    );
}

#[test]
fn sanitized_native_fixture_exposes_only_structured_output_and_terminal_cumulative_usage() {
    let frames: Vec<_> = include_str!("../fixtures/protocol/agy-success.jsonl")
        .lines()
        .map(|line| architect::transport::decode::<serde_json::Value>(line.as_bytes()).unwrap())
        .collect();
    assert!(frames.iter().all(|frame| agy::normalize(frame).is_ok()));
    assert!(matches!(
        agy::normalize(frames.last().unwrap()).unwrap(),
        agy::NativeEvent::Result {
            usage: UsageReport::Reported {
                input_tokens: Some(795),
                cumulative: true,
                ..
            },
            ..
        }
    ));
    let tool: serde_json::Value =
        architect::transport::decode(include_bytes!("../fixtures/protocol/agy-tool.jsonl"))
            .unwrap();
    assert!(agy::normalize(&tool).is_err());
}

#[test]
fn public_help_on_stderr_and_large_stdout_are_drained_without_affecting_turn_privacy() {
    let f = Fixture::new("stderr_help");
    let receipt = f.call(None, &mut vec![]).unwrap();
    assert!(receipt.completed);
    assert!(!receipt.output.contains(SECRET));
}

#[test]
fn empty_native_status_line_is_accepted_but_a_command_is_refused_without_modification() {
    let f = Fixture::new("success");
    let directory = f.home.join(".gemini/antigravity-cli");
    std::fs::create_dir_all(&directory).unwrap();
    let settings = directory.join("settings.json");
    std::fs::write(
        &settings,
        r#"{"statusLine":{"type":"","command":"","enabled":true}}"#,
    )
    .unwrap();
    f.call(None, &mut vec![]).unwrap();
    let before = std::fs::read(&f.launched).unwrap();
    let custom =
        r#"{"statusLine":{"type":"command","command":"touch forbidden-effect","enabled":true}}"#;
    std::fs::write(&settings, custom).unwrap();
    assert!(f.call(None, &mut vec![]).is_err());
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), custom);
    assert_eq!(std::fs::read(&f.launched).unwrap(), before);
}

#[test]
fn version_135_uses_the_same_restricted_transport_and_unknown_versions_still_fail() {
    let f = Fixture::new("version_135");
    assert_eq!(f.provider.probe().unwrap().version, "1.3.5");
    assert!(f.call(None, &mut vec![]).unwrap().completed);
    let f = Fixture::new("unsupported_version");
    assert!(f.call(None, &mut vec![]).is_err());
    assert!(!f.launched.exists());
}
