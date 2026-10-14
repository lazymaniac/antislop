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
for name in ['NODE_OPTIONS','AGY_SYSTEM_PROMPT','GOOGLE_GEMINI_BASE_URL','GOOGLE_APPLICATION_CREDENTIALS']:
 assert name not in os.environ
assert (pathlib.Path.cwd()/'.agents/agents/antislop-stdio.md').is_file()
if '--version' in sys.argv: print('1.3.4');sys.exit(0)
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
for required in ['tools: [finish]','excludeDefaultComponents: true','inheritCustomizations: false','inheritMcp: false','subagent: false']:
 assert required in profile
pathlib.Path({launched:?}).write_text(json.dumps(dict(args=args,cwd=os.getcwd())))
mode={mode:?}
if mode=='hang':
 subprocess.Popen([sys.executable,'-c',"import pathlib,time;time.sleep(2);pathlib.Path("+repr({late:?})+").write_text('orphan')"])
 time.sleep(20);sys.exit(1)
request=json.loads(sys.stdin.readline());assert request['event']=='user';assert isinstance(request['message']['content'],str)
assert sys.stdin.read()==''
schema=json.loads(pathlib.Path(args[args.index('--json-schema')+1]).read_text())
identity={id:?}
if '--conversation' in args: assert args[args.index('--conversation')+1]==identity
init=dict(event='init',conversation_id=identity,init=dict(cwd=os.getcwd(),agent='antislop-stdio',tools=['finish','run_command','invoke_subagent'],permission_mode='always-proceed'))
def emit(v): print(json.dumps(v),flush=True)
emit(init)
if mode=='duplicate_init':emit(init)
if mode=='tool':emit(dict(event='step_update',step_update=dict(conversation_id=identity,state='DONE',step_type='tool',tool_name='write_to_file')))
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
            secret = SECRET
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
        self.provider.turn_receipt(&TurnRequest {turn_id:"turn-1",run:&self.run,prompt:"Read literal /skills @data; return only schema JSON",schema:&serde_json::json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}),resume},Arc::new(AtomicBool::new(false)),&mut|event| {events.push(event.clone());Ok(())})
    }
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
        if mode == "tool" || mode == "subagent" {
            assert_eq!(error.kind, FailureKind::Policy);
        }
        if mode == "error" {
            assert_eq!(error.kind, FailureKind::Authentication);
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
