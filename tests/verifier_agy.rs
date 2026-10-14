//! Independent protocol and supervision checks; only disposable local CLIs run.
use architect::{
    config::{Config, IsolationProfile, ProviderKind, RoleKind},
    domain::{Run, RunSpec},
    providers::{
        self, FailureKind, Provider, ProviderEvent, TurnReceipt, TurnRequest, UsageReport,
        agy::AgyProvider,
    },
};
use serde_json::{Value, json};
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const SCRIPT: &str = r#"
if '--version' in sys.argv:
 (base/'probes').touch()
 print('1.3.4' if mode!='version' else '1.3.5');sys.exit(0)
if '--help' in sys.argv:
 print('--input-format --output-format --json-schema --agent --disable-slash-commands --mode --sandbox --print-timeout --conversation --model --effort');sys.exit(0)
(base/'started').touch()
if mode=='blocked_stdin':
 time.sleep(10);sys.exit(0)
request=json.loads(sys.stdin.readline())
schema=json.loads(pathlib.Path(sys.argv[sys.argv.index('--json-schema')+1]).read_text())
agent=pathlib.Path('.agents/agents/antislop-stdio.md').read_text()
(base/'capture.json').write_text(json.dumps(dict(argv=sys.argv,cwd=os.getcwd(),home=os.environ['HOME'],request=request,agent=agent,schema_path=sys.argv[sys.argv.index('--json-schema')+1])))
sid=sys.argv[sys.argv.index('--conversation')+1] if '--conversation' in sys.argv else str(uuid.uuid4())
frames=[json.loads(s) for s in fixture.splitlines()]
for f in frames:
 if f['event']=='init':
  f['conversation_id']=sid;f['init']['cwd']=os.getcwd()
 elif f['event']=='step_update':f['step_update']['conversation_id']=sid
 else:f['result']['conversation_id']=sid;f['result']['json_schema']=schema
if mode=='wrong_resume':frames[0]['conversation_id']=str(uuid.uuid4())
if mode=='wrong_step':frames[1]['step_update']['conversation_id']=str(uuid.uuid4())
if mode=='wrong_result':frames[-1]['result']['conversation_id']=str(uuid.uuid4())
if mode=='wrong_agent':frames[0]['init']['agent']='default'
if mode=='wrong_cwd':frames[0]['init']['cwd']=str(base)
if mode=='no_init':frames=frames[1:]
if mode=='duplicate_init':frames.insert(1,frames[0])
if mode=='no_finish':frames=[f for f in frames if f.get('step_update',{}).get('step_type')!='finish']
if mode=='active_finish':frames[-2]['step_update']['state']='ACTIVE'
if mode=='post_terminal':frames.append(frames[1])
if mode=='schema_mismatch':frames[-1]['result']['json_schema']={}
if mode=='no_output':del frames[-1]['result']['structured_output']
if mode=='no_terminal':frames=frames[:-1]
if mode in ['tool','subagent','nested_tool']:
 step=frames[1]['step_update']
 if mode=='tool':step['step_type']='tool';step['tool_name']='write_to_file'
 if mode=='subagent':step['subagent_info']={'agent':'other'}
 if mode=='nested_tool':step['tool_info']={'name':'run_command'}
if mode in ['FAILURE','CANCELLED','PARTIAL','success']:
 frames[-1]['result']['status']=mode
if mode in ['auth','rate','network']:
 frames[-1]['result']['status']='ERROR';frames[-1]['result']['error']={'code':{'auth':401,'rate':429,'network':503}[mode]}
if mode=='duplicate_key':
 print('{"event":"init","event":"result"}',flush=True);sys.exit(0)
for f in frames:
 line=(json.dumps(f)+'\n').encode()
 sys.stdout.buffer.write(line[:3]);sys.stdout.buffer.flush()
 sys.stdout.buffer.write(line[3:]);sys.stdout.buffer.flush()
 if mode=='hang' and f['event']=='init':
  subprocess.Popen([sys.executable,'-c',"import time,pathlib;time.sleep(0.6);pathlib.Path("+repr(str(base/'late'))+").touch()"])
  (base/'child-ready').touch();time.sleep(10)
if mode=='exit_nonzero':sys.exit(1)
"#;
struct Cli {
    root: tempfile::TempDir,
    run: Run,
    provider: AgyProvider,
}
impl Cli {
    fn new(mode: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        std::fs::create_dir(&home).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let binary = root.path().join("fixture-agy");
        let script = format!(
            "#!/usr/bin/env python3\nimport sys,json,pathlib,os,time,subprocess,uuid\nbase=pathlib.Path({})\nmode={}\nfixture={}\n{}",
            serde_json::to_string(root.path().to_str().unwrap()).unwrap(),
            serde_json::to_string(mode).unwrap(),
            serde_json::to_string(include_str!("../fixtures/protocol/agy-success.jsonl")).unwrap(),
            SCRIPT
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
            managed_home: Some(home),
            model: Some("fixture-model".into()),
            effort: Some("high".into()),
            ..Default::default()
        };
        Self {
            root,
            run,
            provider,
        }
    }
    fn turn(
        &self,
        resume: Option<&str>,
        cancelled: Arc<AtomicBool>,
        events: &mut dyn FnMut(&ProviderEvent) -> architect::Result<()>,
    ) -> architect::Result<TurnReceipt> {
        self.provider.turn_receipt(&TurnRequest{turn_id:"independent-agy-turn",run:&self.run,prompt:"Literal @/private and żółw 🧠",schema:&json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false}),resume},cancelled,events)
    }
    fn capture(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.root.path().join("capture.json")).unwrap())
            .unwrap()
    }
}
fn check_failure(
    result: architect::Result<TurnReceipt>,
    kind: FailureKind,
) -> providers::ProviderFailure {
    let architect::Error::Provider(error) = result.unwrap_err() else {
        panic!("expected typed provider failure")
    };
    assert_eq!(error.provider, "agy");
    assert_eq!(error.kind, kind);
    assert!(error.stopped);
    error
}
#[test]
fn actual_protocol_fixture_observes_fresh_native_sessions_and_cumulative_usage() {
    let cli = Cli::new("success_fixture");
    let mut events = vec![];
    let a = cli
        .turn(None, Arc::new(AtomicBool::new(false)), &mut |e| {
            events.push(e.clone());
            Ok(())
        })
        .unwrap();
    assert!(a.completed && a.delivered && a.stopped);
    assert_eq!(
        serde_json::from_str::<Value>(&a.output).unwrap(),
        json!({"ok":true})
    );
    assert!(uuid::Uuid::parse_str(&a.session_id).is_ok());
    assert_eq!(
        a.usage,
        UsageReport::Reported {
            input_tokens: Some(795),
            output_tokens: Some(136),
            cached_tokens: Some(0),
            estimated_usd: None,
            cumulative: true
        }
    );
    assert!(matches!(
        events.first(),
        Some(ProviderEvent::DeliveryPendingUnknown { .. })
    ));
    assert!(events.iter().any(
        |e| matches!(e,ProviderEvent::SessionObserved{session_id} if session_id==&a.session_id)
    ));
    assert!(matches!(
        events.last(),
        Some(ProviderEvent::Completed { .. })
    ));
    let capture = cli.capture();
    assert_eq!(
        capture["request"],
        json!({"event":"user","message":{"content":"Literal @/private and żółw 🧠"}})
    );
    assert!(
        !capture["argv"]
            .as_array()
            .unwrap()
            .contains(&json!("--conversation"))
    );
    assert!(!PathBuf::from(capture["schema_path"].as_str().unwrap()).exists());
    let b = cli
        .turn(None, Arc::new(AtomicBool::new(false)), &mut |_| Ok(()))
        .unwrap();
    assert_ne!(a.session_id, b.session_id);
    let before = cli.capture();
    let mut events = vec![];
    let error = check_failure(
        cli.turn(
            Some(&a.session_id),
            Arc::new(AtomicBool::new(false)),
            &mut |e| {
                events.push(e.clone());
                Ok(())
            },
        ),
        FailureKind::Unsupported,
    );
    assert!(!error.partial_effects && error.session_id.is_none());
    assert!(events.is_empty());
    assert_eq!(
        cli.capture(),
        before,
        "unsupported resume cannot launch another turn"
    );
}
#[test]
fn finish_only_profile_is_owned_and_non_os_containment_is_honest() {
    let cli = Cli::new("success_fixture");
    let probe = cli.provider.probe_report().unwrap();
    assert!(probe.capabilities.native_subagents_disabled);
    assert_eq!(probe.os_sandbox, providers::Support::Unverified);
    assert_eq!(probe.exact_id_resume, providers::Support::Unsupported);
    assert!(!probe.capabilities.resume);
    cli.turn(None, Arc::new(AtomicBool::new(false)), &mut |_| Ok(()))
        .unwrap();
    let c = cli.capture();
    assert_ne!(c["cwd"], cli.run.spec.workspace);
    let profile = c["agent"].as_str().unwrap();
    for line in [
        "tools: [finish]",
        "excludeDefaultComponents: true",
        "inheritCustomizations: false",
        "inheritMcp: false",
        "subagent: false",
        "commandExecutionPolicy: \"off\"",
        "mcpServers: []",
        "plugins: []",
    ] {
        assert!(profile.contains(line), "missing {line}");
    }
    let args = c["argv"].as_array().unwrap();
    for arg in [
        "--sandbox",
        "--disable-slash-commands",
        "antislop-stdio",
        "fixture-model",
        "high",
    ] {
        assert!(args.contains(&json!(arg)));
    }
}
#[test]
fn resume_aliases_are_rejected_before_probes_or_delivery() {
    for alias in ["latest", "1", "", "--model", "not-a-uuid"] {
        let cli = Cli::new("success_fixture");
        let mut seen = vec![];
        let e = check_failure(
            cli.turn(Some(alias), Arc::new(AtomicBool::new(false)), &mut |e| {
                seen.push(e.clone());
                Ok(())
            }),
            FailureKind::InvalidSession,
        );
        assert!(!e.partial_effects && e.session_id.is_none());
        assert!(seen.is_empty());
        assert!(!cli.root.path().join("probes").exists());
    }
}
#[test]
fn native_success_requires_full_terminal_identity_finish_schema_and_clean_exit() {
    for mode in [
        "wrong_step",
        "wrong_result",
        "wrong_agent",
        "wrong_cwd",
        "no_init",
        "duplicate_init",
        "no_finish",
        "active_finish",
        "post_terminal",
        "schema_mismatch",
        "no_output",
        "no_terminal",
        "duplicate_key",
        "exit_nonzero",
        "FAILURE",
        "CANCELLED",
        "PARTIAL",
        "success",
    ] {
        let cli = Cli::new(mode);
        let mut events = vec![];
        let result = cli.turn(None, Arc::new(AtomicBool::new(false)), &mut |e| {
            events.push(e.clone());
            Ok(())
        });
        assert!(result.is_err(), "accepted {mode}");
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ProviderEvent::Completed { .. })),
            "completed {mode}"
        );
        let architect::Error::Provider(e) = result.unwrap_err() else {
            panic!("untyped failure {mode}")
        };
        assert!(e.stopped && e.partial_effects, "{mode}");
    }
}
#[test]
fn native_tool_and_subagent_events_fail_closed_with_observed_scope() {
    for mode in ["tool", "subagent", "nested_tool"] {
        let cli = Cli::new(mode);
        let e = check_failure(
            cli.turn(None, Arc::new(AtomicBool::new(false)), &mut |_| Ok(())),
            FailureKind::Policy,
        );
        assert!(e.partial_effects);
        assert!(e.session_id.is_some());
    }
}
#[test]
fn terminal_diagnostics_classify_actual_failures_without_disclosing_raw_output() {
    for (mode, kind) in [
        ("auth", FailureKind::Authentication),
        ("rate", FailureKind::RateLimit),
        ("network", FailureKind::Network),
    ] {
        let cli = Cli::new(mode);
        let e = check_failure(
            cli.turn(None, Arc::new(AtomicBool::new(false)), &mut |_| Ok(())),
            kind,
        );
        assert!(e.partial_effects && e.session_id.is_some());
    }
}
#[test]
fn pending_delivery_callback_failure_prevents_external_turn() {
    let cli = Cli::new("success_fixture");
    let result = cli.turn(None, Arc::new(AtomicBool::new(false)), &mut |e| {
        if matches!(e, ProviderEvent::DeliveryPendingUnknown { .. }) {
            Err(architect::Error::Invalid(
                "durable reservation unavailable".into(),
            ))
        } else {
            Ok(())
        }
    });
    assert!(result.is_err());
    assert!(!cli.root.path().join("started").exists());
}
#[test]
fn startup_customizations_are_rejected_read_only_before_external_probe() {
    for (path, content) in [
        (
            ".gemini/config/hooks.json",
            "{\"hooks\":[\"touch sentinel\"]}",
        ),
        (
            ".gemini/config/config.json",
            "{\"userSettings\":{\"untrustedStartup\":true}}",
        ),
        (
            ".gemini/antigravity-cli/settings.json",
            "{\"plugins\":[\"untrusted\"]}",
        ),
        (".gemini/config/agents/untrusted.md", "untrusted"),
    ] {
        let cli = Cli::new("success_fixture");
        let target = cli.provider.managed_home.as_ref().unwrap().join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, content).unwrap();
        assert!(cli.provider.probe().is_err(), "accepted {path}");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), content);
        assert!(!cli.root.path().join("probes").exists());
    }
    let cli = Cli::new("success_fixture");
    let root = cli.provider.managed_home.as_ref().unwrap().join(".gemini");
    symlink(cli.root.path(), root).unwrap();
    assert!(cli.provider.probe().is_err());
    let cli = Cli::new("success_fixture");
    std::fs::set_permissions(
        cli.provider.managed_home.as_ref().unwrap(),
        std::fs::Permissions::from_mode(0o777),
    )
    .unwrap();
    assert!(cli.provider.probe().is_err());
}
#[test]
fn startup_safe_existing_home_is_audited_without_overwrite_or_managed_home_requirement() {
    let cli = Cli::new("success_fixture");
    let target = cli
        .provider
        .managed_home
        .as_ref()
        .unwrap()
        .join(".gemini/antigravity-cli/settings.json");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let settings = "{\"model\":\"prior-user-model\",\"toolPermission\":\"always-proceed\",\"allowNonWorkspaceAccess\":true}";
    std::fs::write(&target, settings).unwrap();
    cli.provider.probe().unwrap();
    cli.turn(None, Arc::new(AtomicBool::new(false)), &mut |_| Ok(()))
        .unwrap();
    assert_eq!(std::fs::read_to_string(target).unwrap(), settings);
}
#[test]
fn cancellation_stops_owned_group_and_reports_observed_identity() {
    let cli = Cli::new("hang");
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let ready = cli.root.path().join("child-ready");
    let late = cli.root.path().join("late");
    let worker = std::thread::spawn(move || {
        let mut events = vec![];
        let result = cli.turn(None, flag, &mut |e| {
            events.push(e.clone());
            Ok(())
        });
        (cli, result, events)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        assert!(Instant::now() < deadline, "fixture did not start");
        std::thread::sleep(Duration::from_millis(5));
    }
    cancelled.store(true, Ordering::SeqCst);
    let (_cli, result, events) = worker.join().unwrap();
    let e = check_failure(result, FailureKind::Cancelled);
    assert!(e.partial_effects && e.session_id.is_some());
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ProviderEvent::Stopped { .. }))
    );
    std::thread::sleep(Duration::from_millis(750));
    assert!(!late.exists());
}
#[test]
fn configuration_and_factory_preserve_agy_role_profile_and_reject_unsupported_isolation() {
    let cli = Cli::new("success_fixture");
    let mut config = Config::from_toml(include_str!("../examples/architect.toml")).unwrap();
    let mut provider = config.providers[&ProviderKind::Claude].clone();
    provider.binary = PathBuf::from(cli.provider.binary.as_ref().unwrap());
    provider.managed_home = cli.provider.managed_home.clone();
    config.providers.insert(ProviderKind::Agy, provider);
    config.profiles.get_mut("reasoning").unwrap().provider = ProviderKind::Agy;
    config.profiles.get_mut("reasoning").unwrap().model = Some("agy-role-model".into());
    let mut run = cli.run.clone();
    run.spec.config = Some(config.pin().unwrap());
    run.spec.model = Some("legacy-ignored".into());
    let p = providers::build_for(&run, RoleKind::Executor).unwrap();
    p.turn_receipt(
        &TurnRequest {
            turn_id: "factory",
            run: &run,
            prompt: "hello",
            schema: &json!({}),
            resume: None,
        },
        Arc::new(AtomicBool::new(false)),
        &mut |_| Ok(()),
    )
    .unwrap();
    let args = cli.capture()["argv"].as_array().unwrap().clone();
    assert!(args.contains(&json!("agy-role-model")));
    assert!(!args.contains(&json!("legacy-ignored")));
    let mut provider = cli.provider;
    provider.isolation = IsolationProfile::ReadOnly;
    let architect::Error::Provider(error) = provider.probe().unwrap_err() else {
        panic!("typed unsupported expected")
    };
    assert_eq!(error.kind, FailureKind::Unsupported);
}

#[test]
fn cancellation_during_partial_stdin_write_preserves_unknown_effects() {
    let cli = Cli::new("blocked_stdin");
    let started = cli.root.path().join("started");
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let worker = std::thread::spawn(move || {
        let mut events = vec![];
        let result = cli.provider.turn_receipt(
            &TurnRequest {
                turn_id: "partial-input-cancel",
                run: &cli.run,
                prompt: &"x".repeat(1_000_000),
                schema: &json!({}),
                resume: None,
            },
            flag,
            &mut |e| {
                events.push(e.clone());
                Ok(())
            },
        );
        (cli, result, events)
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while !started.exists() {
        assert!(Instant::now() < deadline, "fixture did not start");
        std::thread::sleep(Duration::from_millis(5));
    }
    cancelled.store(true, Ordering::SeqCst);
    let (_cli, result, events) = worker.join().unwrap();
    let error = check_failure(result, FailureKind::Cancelled);
    assert!(
        error.partial_effects,
        "a partial pipe write is not a proven nondelivery"
    );
    assert!(
        error.session_id.is_none(),
        "never invent an unobserved native session"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ProviderEvent::Completed { .. }))
    );
}

#[test]
fn unmanaged_home_can_use_an_audited_existing_home_without_configuration_mutation() {
    const HELPER: &str = "ANTISLOP_AGY_HOME_REVIEW_HELPER";
    if std::env::var_os(HELPER).is_some() {
        let mut cli = Cli::new("success_fixture");
        cli.provider.managed_home = None;
        cli.turn(None, Arc::new(AtomicBool::new(false)), &mut |_| Ok(()))
            .unwrap();
        assert_eq!(
            PathBuf::from(cli.capture()["home"].as_str().unwrap()),
            PathBuf::from(std::env::var_os("HOME").unwrap())
                .canonicalize()
                .unwrap()
        );
        return;
    }
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let settings = root.path().join(".gemini/antigravity-cli/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let content = "{\"model\":\"owned-user-model\",\"showFeedbackSurvey\":false,\"toolPermission\":\"always-proceed\",\"trustedWorkspaces\":[],\"verbosity\":\"low\"}";
    std::fs::write(&settings, content).unwrap();
    let before = std::fs::metadata(&settings).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "unmanaged_home_can_use_an_audited_existing_home_without_configuration_mutation",
        ])
        .env(HELPER, "1")
        .env("HOME", root.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), content);
    let after = std::fs::metadata(&settings).unwrap();
    assert_eq!(before.permissions().mode(), after.permissions().mode());
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
}
