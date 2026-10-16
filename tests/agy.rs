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
assert os.environ.get('AGY_CLI_OSC7501')=='off'
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
if mode=='invalid_catalog':init['init']['tools']=[dict(private={secret:?})]
if mode!='correction-unobserved':emit(init)
if mode.startswith('context'):
 import re
 assert 'For this CLI invoke call_mcp_tool with ServerName=antislop-context, ToolName=read_context' in profile
 assert 'Do not call list_resources' in profile
 assert request['message']['content'].startswith('Antislop immutable context transport agy-pinned-context-v2.')
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
 first=json.JSONDecoder().raw_decode(request['message']['content'].split('First index call: call_mcp_tool(',1)[1])[0]
 assert first==dict(ServerName='antislop-context',ToolName='read_context',Arguments=dict(bundle_hash=bundle,document_hash=index,offset=0,max_bytes=16384))
 step_index=1
 def fetch(document,offset):
  global step_index
  step_index+=1
  arguments=dict(bundle_hash=bundle,document_hash=document,offset=offset,max_bytes=16384)
  parameters=dict(ServerName=('foreign' if mode=='context-foreign' else 'antislop-context'),ToolName='read_context',Arguments=arguments)
  info=dict(name='call_mcp_tool',parameters=parameters)
  emit(dict(event='step_update',step_update=dict(conversation_id=identity,step_index=step_index,state='ACTIVE',step_type='tool',tool_name='call_mcp_tool',tool_info=info)))
  if mode=='context-correction-outstanding':emit(dict(event='step_update',step_update=dict(conversation_id=identity,step_index=step_index+1,state='DONE',step_type='error_message',error_message=dict(message='Malformed function call: '+{secret:?}))))
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
 original=document(source['document_hash'],mode in ('context-incomplete','context-tool-finish-incomplete','context-correction-incomplete'))
 if mode not in ('context-incomplete','context-tool-finish-incomplete','context-correction-incomplete','context-forged'):assert hashlib.sha256(original.encode()).hexdigest()==source['document_hash']
 if mode=='context-resources':emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type='unknown',step_index=step_index+1)))
 proxy.stdin.close();proxy.wait()
if mode=='duplicate_init':emit(init)
correction_index=None
if 'correction-' in mode:
 correction_index=step_index+1 if mode.startswith('context') else 4
 step=dict(conversation_id=identity,state='DONE',step_type='error_message',step_index=correction_index,error_message=dict(message='Native model response rejected. Malformed function call: '+{secret:?}))
 if mode=='correction-wrong-id':step['conversation_id']={other:?}
 if mode=='correction-active':step['state']='ACTIVE'
 if mode=='correction-unknown-code':step['error_message']['code']='AUTHENTICATION_ERROR'
 if mode=='correction-no-retry':step['error_message']['retryable']=False
 if mode=='correction-auth-prefix':step['error_message']['message']='Authentication failed. Malformed function call: '+{secret:?}
 if mode=='correction-mentioned':step['error_message']['message']='An explanation mentions malformed function call without a native marker: '+{secret:?}
 if mode=='correction-nested':step['error_message']=dict(details=dict(message='Malformed function call: '+{secret:?}))
 if mode.endswith('correction-text-delta'):
  step['text_delta']=step.pop('error_message');step['text_delta']=step['text_delta']['message']
 if mode=='correction-null-carrier':step['error']=step.pop('error_message');step['error_message']=None
 if mode=='correction-conflicting-carriers':step['error']=dict(code='AUTHENTICATION_ERROR',message={secret:?})
 if mode=='correction-bad-delta-type':step['text_delta']=step.pop('error_message')
 if 'correction-opaque' in mode:
  step.pop('error_message');step.update(duration_seconds=0.25,usage=dict(output_tokens=1))
  if mode=='correction-opaque-nulls':step.update(error_message=None,errorMessage=None,error=None,text_delta=None)
  if mode=='correction-opaque-extra':step['private_unrecognized_field']={secret:?}
  if mode=='correction-opaque-bad-duration':step['duration_seconds']={secret:?}
  if mode=='correction-opaque-bad-usage':step['usage']={secret:?}
  if mode=='correction-opaque-unindexed':step.pop('step_index')
  if mode=='correction-opaque-finished':emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type='finish',step_index=3)))
  if mode=='correction-opaque-wrong-id':step['conversation_id']={other:?}
  if mode=='correction-opaque-external':step['tool_name']='run_command'
 if mode=='correction-missing-index':step.pop('step_index')
 if mode=='correction-after-finish':emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type='finish',step_index=3)))
 if mode=='correction-old-rejection':emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type='agent_response',step_index=5)))
 if mode in ('correction-callback-fails','correction-opaque-callback-fails'):
  subprocess.Popen([sys.executable,'-c',"import pathlib,time;time.sleep(2);pathlib.Path("+repr({late:?})+").write_text('late')"])
 event=dict(event='step_update',step_update=step);emit(event)
 if mode=='correction-opaque-duplicate':emit(event)
 if mode=='correction-opaque-no-terminal':sys.exit(0)
 if mode=='correction-opaque-no-finish':mode='no_finish'
 if mode=='correction-opaque-fatal-result':mode='error'
 if mode=='correction-opaque-bad-schema':mode='wrong_schema'
 if mode=='correction-opaque-nonzero':mode='finish-exit-nonzero'
 if mode in ('correction-callback-fails','correction-opaque-callback-fails'):time.sleep(60)
 if mode=='correction-duplicate':emit(event)
 if mode=='correction-repeat':
  correction_index+=2;step['step_index']=correction_index;emit(event)
 if mode=='correction-no-terminal':sys.exit(0)
 if mode=='correction-no-new-finish':mode='no_finish'
 if mode=='correction-wrong-schema':mode='wrong_schema'
 if mode=='correction-nonzero':mode='finish-exit-nonzero'
 if mode=='correction-terminal-error':mode='error'
if mode in ('step_output_limit','step_output_limit_payload','step_malformed_call'):
 subprocess.Popen([sys.executable,'-c',"import pathlib,time;time.sleep(2);pathlib.Path("+repr({late:?})+").write_text('late')"])
 step=dict(conversation_id=identity,state='DONE',step_type='error_message')
 step['error_message' if mode.endswith('_payload') else 'error']=dict(message={secret:?} + (' Malformed function call: sensitive attempted parameters' if mode=='step_malformed_call' else ' Your previous response was cut off because it exceeded the output token limit. Retries remaining: 3'))
 emit(dict(event='step_update',step_update=step))
 time.sleep(60)
if mode=='invalid_step_state':emit(dict(event='step_update',step_update=dict(conversation_id=identity,state={secret:?},step_type='agent_response',text_delta={secret:?})))
if mode=='tool':emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type='tool',tool_name='write_to_file')))
if mode=='private_tool':emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type='tool',tool_name={secret:?},tool_info=dict(name={secret:?},parameters=dict(command={secret:?})),text_delta={secret:?})))
if mode in ('unknown_step','system_step'):emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type=('system_message' if mode=='system_step' else {secret:?}),text_delta={secret:?})))
if mode in ('unmapped_step','unmapped_active'):emit(dict(event='step_update',step_update=dict(conversation_id=identity,state=('ACTIVE' if mode=='unmapped_active' else 'DONE'),step_type='unknown',step_index=4)))
if mode=='subagent':emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type='agent_response',subagent_info=dict(subagents=[]))))
if mode=='bad_json':print('{{"event":"result","event":"result"}}',flush=True);sys.exit(1)
emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type='agent_response',step_index=(correction_index+1 if correction_index is not None else 1000),text_delta={secret:?},usage=dict(input_tokens=9999999,output_tokens=9999999))))
if mode=='tool-finish' or mode.startswith('finish-') or mode.startswith('context-tool-finish'):
 info=dict(name=('write_to_file' if mode=='finish-name-mismatch' else 'finish'),parameters=dict(ok=True,toolSummary={secret:?},toolAction='Return structured output'))
 if mode=='finish-error':info['error']=dict(message={secret:?})
 if mode=='finish-bad-parameters':info['parameters']='invalid'
 step=dict(conversation_id=({other:?} if mode=='finish-wrong-id' else identity),state='ACTIVE',step_type='tool',tool_name=('write_to_file' if mode=='finish-tool-mismatch' else 'finish'),tool_info=info)
 if mode=='finish-subagent':step['subagent_info']=dict(subagents=[])
 emit(dict(event='step_update',step_update=step))
 if mode!='finish-active-only':
  step['state']='DONE';emit(dict(event='step_update',step_update=step))
elif mode!='no_finish':
 step=dict(conversation_id=identity,state='DONE',step_type='finish',step_index=(correction_index+2 if correction_index is not None else 1001))
 if mode=='correction-old-finish':step['step_index']=correction_index
 if mode=='correction-unindexed-finish':step.pop('step_index')
 emit(dict(event='step_update',step_update=step))
if mode=='finish-no-terminal':sys.exit(0)
result=dict(event='result',result=dict(conversation_id=({other:?} if mode=='wrong_id' else identity),status='SUCCESS',structured_output=dict(ok=True),json_schema=({{}} if mode=='wrong_schema' else schema),usage=dict(input_tokens=795,output_tokens=136,cache_read_tokens=0)))
if mode in ('missing_output','finish-missing-output'):result['result'].pop('structured_output');result['result']['response']='{{"ok":true}}'
if mode=='finish-wrong-schema':result['result']['json_schema']={{}}
if mode=='error':result['result'].update(status='ERROR',error={secret:?}+' authentication failed');sys.stderr.write({secret:?});emit(result);sys.exit(1)
emit(result)
if mode=='late':emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type='agent_response',text_delta={secret:?})))
if mode=='duplicate_result':emit(result)
if mode=='finish-exit-nonzero':sys.exit(1)
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
    fn with_version(mode: &str, version: &str) -> Self {
        let fixture = Self::new(mode);
        let binary = fixture.provider.binary.as_ref().unwrap();
        let script = std::fs::read_to_string(binary).unwrap();
        let mut replaced = 0;
        let rewritten = script
            .lines()
            .map(|line| {
                if line.starts_with("if '--version' in sys.argv:") {
                    replaced += 1;
                    format!("if '--version' in sys.argv: print({version:?});sys.exit(0)")
                } else {
                    line.into()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(replaced, 1);
        std::fs::write(binary, rewritten).unwrap();
        fixture
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
fn native_format_correction_keeps_one_process_session_and_all_terminal_gates() {
    for mode in [
        "correction-success",
        "correction-repeat",
        "context-correction-success",
        "correction-text-delta",
        "context-correction-text-delta",
        "correction-null-carrier",
    ] {
        let mut fixture = Fixture::with_version(mode, "1.4.0");
        if mode.starts_with("context") {
            fixture.provider.isolation = IsolationProfile::PinnedContext;
        }
        let mut events = vec![];
        let receipt = fixture.call(None, &mut events).unwrap();
        assert!(receipt.completed && receipt.stopped && receipt.delivered);
        assert_eq!(receipt.session_id, ID);
        assert_eq!(receipt.output, "{\"ok\":true}");
        let rejections: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                ProviderEvent::NativeOutputRejected {
                    session_id,
                    step_index,
                    reason,
                    ..
                } => {
                    assert_eq!(session_id, ID);
                    assert_eq!(
                        *reason,
                        providers::NativeOutputRejection::MalformedFunctionCall
                    );
                    Some(*step_index)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            rejections.len(),
            if mode == "correction-repeat" { 2 } else { 1 }
        );
        assert!(rejections.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, ProviderEvent::SessionObserved { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, ProviderEvent::DeliverySent { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, ProviderEvent::Completed { .. }))
                .count(),
            1
        );
        assert!(!serde_json::to_string(&events).unwrap().contains(SECRET));
    }
    for mode in [
        "correction-wrong-id",
        "correction-unobserved",
        "correction-active",
        "correction-unknown-code",
        "correction-no-retry",
        "correction-auth-prefix",
        "correction-mentioned",
        "correction-nested",
        "correction-conflicting-carriers",
        "correction-bad-delta-type",
        "correction-missing-index",
        "correction-after-finish",
        "correction-old-rejection",
        "correction-duplicate",
        "correction-no-terminal",
        "correction-no-new-finish",
        "correction-wrong-schema",
        "correction-nonzero",
        "correction-terminal-error",
        "correction-old-finish",
        "correction-unindexed-finish",
        "context-correction-incomplete",
        "context-correction-outstanding",
    ] {
        let mut fixture = Fixture::with_version(mode, "1.4.0");
        if mode.starts_with("context") {
            fixture.provider.isolation = IsolationProfile::PinnedContext;
        }
        let mut events = vec![];
        let error = fixture
            .call_prompt(&"Original required input. ".repeat(1800), None, &mut events)
            .unwrap_err();
        assert!(!error.to_string().contains(SECRET), "{mode}");
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ProviderEvent::Completed { .. })),
            "{mode}"
        );
    }
    for version in ["1.3.4", "1.3.5"] {
        let fixture = Fixture::with_version("correction-success", version);
        let mut events = vec![];
        assert!(fixture.call(None, &mut events).is_err());
        assert!(!events.iter().any(|event| matches!(
            event,
            ProviderEvent::NativeOutputRejected { .. } | ProviderEvent::Completed { .. }
        )));
    }
}

#[test]
fn a_reasonless_native_step_is_observed_until_the_actual_terminal_result() {
    for mode in [
        "correction-opaque-success",
        "correction-opaque-nulls",
        "context-correction-opaque-success",
    ] {
        let mut fixture = Fixture::with_version(mode, "1.4.0");
        if mode.starts_with("context") {
            fixture.provider.isolation = IsolationProfile::PinnedContext;
        }
        let mut events = vec![];
        let receipt = fixture.call(None, &mut events).unwrap();
        assert!(receipt.completed && receipt.delivered && receipt.stopped);
        assert_eq!(receipt.output, "{\"ok\":true}");
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    ProviderEvent::NativeOutputRejected {
                        reason: providers::NativeOutputRejection::UnclassifiedNativeStep,
                        ..
                    }
                ))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, ProviderEvent::Completed { .. }))
                .count(),
            1
        );
        assert!(!serde_json::to_string(&events).unwrap().contains(SECRET));
    }
    for mode in [
        "correction-opaque-extra",
        "correction-opaque-bad-duration",
        "correction-opaque-bad-usage",
        "correction-opaque-unindexed",
        "correction-opaque-finished",
        "correction-opaque-wrong-id",
        "correction-opaque-external",
        "correction-opaque-duplicate",
        "correction-opaque-no-terminal",
        "correction-opaque-no-finish",
        "correction-opaque-fatal-result",
        "correction-opaque-bad-schema",
        "correction-opaque-nonzero",
    ] {
        let fixture = Fixture::with_version(mode, "1.4.0");
        let mut events = vec![];
        let error = fixture.call(None, &mut events).unwrap_err();
        assert!(!error.to_string().contains(SECRET), "{mode}");
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ProviderEvent::Completed { .. })),
            "{mode}"
        );
        if mode == "correction-opaque-fatal-result" {
            assert!(
                matches!(error,architect::Error::Provider(ref failure) if failure.kind == FailureKind::Authentication)
            );
        }
    }
    for version in ["1.3.4", "1.3.5"] {
        let fixture = Fixture::with_version("correction-opaque-success", version);
        let mut events = vec![];
        assert!(fixture.call(None, &mut events).is_err());
        assert!(!events.iter().any(|e| matches!(
            e,
            ProviderEvent::NativeOutputRejected { .. } | ProviderEvent::Completed { .. }
        )));
    }
}

#[test]
fn failed_native_rejection_commit_stops_the_owned_process_and_descendants() {
    for mode in [
        "correction-callback-fails",
        "correction-opaque-callback-fails",
    ] {
        let fixture = Fixture::with_version(mode, "1.4.0");
        let mut observed = vec![];
        let schema = serde_json::json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false});
        let started = Instant::now();
        let error = fixture
            .provider
            .turn_receipt(
                &TurnRequest {
                    turn_id: "turn-1",
                    run: &fixture.run,
                    prompt: "Return the schema output",
                    schema: &schema,
                    resume: None,
                },
                Arc::new(AtomicBool::new(false)),
                &mut |event| {
                    if matches!(event, ProviderEvent::NativeOutputRejected { .. }) {
                        return Err(architect::Error::Invalid(SECRET.into()));
                    }
                    observed.push(event.clone());
                    Ok(())
                },
            )
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(10));
        let architect::Error::Provider(error) = error else {
            panic!("expected scoped failure")
        };
        assert_eq!(error.session_id.as_deref(), Some(ID));
        assert!(error.stopped && error.partial_effects);
        assert!(error.message.contains("native_rejection_commit"));
        assert!(!error.message.contains(SECRET));
        assert!(
            !observed
                .iter()
                .any(|event| matches!(event, ProviderEvent::Completed { .. }))
        );
        std::thread::sleep(Duration::from_millis(2200));
        assert!(!fixture.root.path().join("late-effect").exists());
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
            "source" => Arc::make_mut(&mut c.bundle)
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
            "scope" => Arc::make_mut(&mut c.bundle).scope.input_hash = architect::hash("foreign"),
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
        "context-tool-finish-incomplete",
        "context-forged",
        "context-duplicate",
        "context-foreign",
        "context-no-commit",
        "context-resources",
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
        if mode == "context-incomplete" || mode == "context-tool-finish-incomplete" {
            assert!(events.iter().any(
                |e| matches!(e,ProviderEvent::OutputRejected {output,..} if output=="{\"ok\":true}")
            ));
        }
        if mode == "context-resources" {
            let architect::Error::Provider(error) = &error else {
                unreachable!()
            };
            assert_eq!(error.kind, FailureKind::Protocol);
            assert!(error.message.contains("step=unknown"));
        }
        if mode == "context-duplicate" || mode == "context-foreign" {
            let architect::Error::Provider(error) = &error else {
                unreachable!()
            };
            assert!(error.message.contains(if mode == "context-duplicate" {
                "boundary=context_step_replayed"
            } else {
                "boundary=context_tool_scope"
            }));
            assert!(error.message.contains("phase=context_tool"));
        }
    }
}

#[test]
fn selected_finish_tool_is_data_only_and_requires_the_terminal_result_in_both_profiles() {
    for mode in ["tool-finish", "context-tool-finish"] {
        let mut f = Fixture::new(mode);
        if mode.starts_with("context") {
            f.provider.isolation = IsolationProfile::PinnedContext;
        }
        let mut events = vec![];
        let receipt = f.call(None, &mut events).unwrap();
        assert!(receipt.completed && receipt.stopped);
        assert_eq!(receipt.output, "{\"ok\":true}");
        assert!(!serde_json::to_string(&events).unwrap().contains(SECRET));
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, ProviderEvent::Completed { .. }))
                .count(),
            1
        );
    }
    for mode in [
        "finish-name-mismatch",
        "finish-tool-mismatch",
        "finish-subagent",
        "finish-error",
        "finish-bad-parameters",
        "finish-active-only",
        "finish-wrong-id",
        "finish-missing-output",
        "finish-wrong-schema",
        "finish-exit-nonzero",
        "finish-no-terminal",
    ] {
        let f = Fixture::new(mode);
        let mut events = vec![];
        let architect::Error::Provider(error) = f.call(None, &mut events).unwrap_err() else {
            panic!("missing scoped native failure for {mode}")
        };
        assert_eq!(error.session_id.as_deref(), Some(ID), "{mode}");
        assert!(error.stopped && error.partial_effects, "{mode}");
        let expected_boundary = match mode {
            "finish-error" | "finish-bad-parameters" => Some("finish_component"),
            "finish-active-only" => Some("result_identity_or_finish"),
            "finish-wrong-id" => Some("step_identity"),
            "finish-wrong-schema" => Some("result_schema"),
            _ => None,
        };
        if let Some(boundary) = expected_boundary {
            assert!(
                error.message.contains(&format!("boundary={boundary}")),
                "{mode}"
            );
        }
        assert!(!serde_json::to_string(&error).unwrap().contains(SECRET));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ProviderEvent::Completed { .. })),
            "{mode}"
        );
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
            if matches!(event,ProviderEvent::ContextRead {..}) && fail_page || matches!(event,ProviderEvent::ContextPrepared {..}) && !fail_page {return Err(architect::Error::Invalid(if fail_page {SECRET.into()} else {"simulated failed durable context commit".into()}));}Ok(())
        });
        assert!(result.is_err());
        if fail_page {
            let error = result.as_ref().unwrap_err().to_string();
            assert!(error.contains("phase=context_commit"));
            assert!(error.contains("boundary=unclassified"));
            assert!(!error.contains(SECRET));
        }
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
fn invalid_native_initialization_catalog_has_a_fixed_private_boundary() {
    let f = Fixture::new("invalid_catalog");
    let mut events = vec![];
    let architect::Error::Provider(error) = f.call(None, &mut events).unwrap_err() else {
        panic!("expected scoped native failure")
    };
    assert!(
        error
            .message
            .contains("phase=normalization; boundary=initialization_catalog")
    );
    assert!(error.session_id.is_none());
    assert!(error.stopped && error.partial_effects);
    assert!(!serde_json::to_string(&error).unwrap().contains(SECRET));
    assert!(!events.iter().any(|e| matches!(
        e,
        ProviderEvent::SessionObserved { .. } | ProviderEvent::Completed { .. }
    )));
}

#[test]
fn protocol_and_policy_faults_preserve_observed_identity_and_private_diagnostics() {
    for mode in [
        "invalid_step_state",
        "duplicate_init",
        "tool",
        "private_tool",
        "unknown_step",
        "system_step",
        "unmapped_step",
        "unmapped_active",
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
        let expected_boundary = match mode {
            "invalid_step_state" => Some("native_step_state"),
            "duplicate_init" => Some("duplicate_initialization"),
            "no_finish" | "wrong_id" => Some("result_identity_or_finish"),
            "wrong_schema" => Some("result_schema"),
            "late" | "duplicate_result" => Some("event_after_terminal"),
            _ => None,
        };
        if let Some(boundary) = expected_boundary {
            assert!(
                error.message.contains(&format!("boundary={boundary}")),
                "{mode}"
            );
        }
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
        if mode.starts_with("unmapped_") {
            assert_eq!(error.kind, FailureKind::Protocol);
            assert!(error.message.contains("step=unknown"));
            assert!(error.message.contains("step_index=Some(4)"));
            assert!(error.message.contains(
                "type_sha256=eb8bf0d80db323992f6b634aab492b1e6d9e96a8e87a511c2a0db75ab929452c"
            ));
            assert!(error.message.contains(if mode == "unmapped_active" {
                "state=ACTIVE"
            } else {
                "state=DONE"
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
    for mode in [
        "step_output_limit",
        "step_output_limit_payload",
        "step_malformed_call",
    ] {
        let f = Fixture::new(mode);
        let start = Instant::now();
        let mut events = vec![];
        let architect::Error::Provider(error) = f.call(None, &mut events).unwrap_err() else {
            panic!("expected typed native output failure")
        };
        assert!(start.elapsed() < Duration::from_secs(8));
        assert_eq!(error.kind, FailureKind::Protocol);
        assert_eq!(error.session_id.as_deref(), Some(ID));
        assert!(error.stopped && error.partial_effects);
        let diagnosis = if mode == "step_malformed_call" {
            "malformed_function_call"
        } else {
            "output_token_limit_exceeded"
        };
        assert!(error.message.contains(diagnosis));
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

#[test]
fn exact_140_contract_keeps_both_profiles_and_refuses_unvalidated_versions_and_replay() {
    for mode in ["success", "context-success", "context-tool-finish"] {
        let mut fixture = Fixture::with_version(mode, "1.4.0");
        if mode.starts_with("context") {
            fixture.provider.isolation = IsolationProfile::PinnedContext;
        }
        assert_eq!(fixture.provider.probe().unwrap().version, "1.4.0");
        assert!(fixture.call(None, &mut vec![]).unwrap().completed);
        let mut events = vec![];
        let error = fixture.call(Some(ID), &mut events).unwrap_err();
        assert!(
            matches!(error,architect::Error::Provider(e) if e.kind==FailureKind::Unsupported && !e.partial_effects)
        );
        assert!(events.is_empty());
    }
    for version in ["1.4.1", "1.4.0-beta", "2.0.0"] {
        let fixture = Fixture::with_version("success", version);
        assert!(fixture.call(None, &mut vec![]).is_err());
        assert!(!fixture.launched.exists());
    }
}

#[test]
fn exact_140_contract_does_not_admit_forged_context_or_invalid_terminal_results() {
    let prompt = "Exact 1.4.0 mandatory source 🦀\n".repeat(1200);
    for mode in [
        "context-no-commit",
        "context-forged",
        "context-duplicate",
        "context-foreign",
        "context-incomplete",
        "duplicate_init",
        "wrong_schema",
        "wrong_id",
        "no_finish",
        "finish-active-only",
        "finish-error",
        "finish-name-mismatch",
        "finish-subagent",
        "finish-no-terminal",
        "finish-exit-nonzero",
    ] {
        let mut fixture = Fixture::with_version(mode, "1.4.0");
        if mode.starts_with("context") {
            fixture.provider.isolation = IsolationProfile::PinnedContext;
        }
        let mut events = vec![];
        let error = match fixture.call_prompt(&prompt, None, &mut events) {
            Err(architect::Error::Provider(error)) => error,
            Err(_) => panic!("missing scoped native error for {mode}"),
            Ok(_) => panic!("unexpected completed turn for {mode}"),
        };
        assert_eq!(error.session_id.as_deref(), Some(ID), "{mode}");
        assert!(error.stopped && error.partial_effects, "{mode}");
        assert!(!serde_json::to_string(&error).unwrap().contains(SECRET));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ProviderEvent::Completed { .. })),
            "{mode}"
        );
    }
}
