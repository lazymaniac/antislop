use architect::{
    config::*,
    domain::*,
    id,
    orchestration::planning::{self, RoundStatus},
    runtime::{Command, Runtime},
    storage::Store,
};
use std::{
    os::unix::fs::PermissionsExt,
    sync::Arc,
    time::{Duration, Instant},
};

struct Fixture {
    root: tempfile::TempDir,
    runtime: Arc<Runtime>,
    run: Run,
}
impl Fixture {
    fn new(mode: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("source"), "original").unwrap();
        let binary = root.path().join("cli");
        let script=r#"#!/usr/bin/env python3
import sys,json,pathlib,hashlib
base=pathlib.Path(__BASE__)
mode=__MODE__
if (base/"management-ready").exists() and mode.startswith("management-"):mode="answer"
if (base/"management-ready").exists() and mode=="intake-recover-review":mode="intake-proposal"
if mode=='hang-probe' and '--version' in sys.argv:
 import subprocess,time
 subprocess.Popen([sys.executable,'-c',"import time,pathlib;root=pathlib.Path("+repr(str(base))+ ");\nwhile not (root/'release-late').exists():time.sleep(.01)\n(root/'late-effect').write_text('orphan')"])
 (base/'probe-started').write_text('yes')
 time.sleep(60)
if '--version' in sys.argv:print('2.1.296 (Claude Code)');sys.exit(0)
if '--help' in sys.argv:print('--safe-mode --strict-mcp-config --tools --permission-prompts --session-id --resume --json-schema --disable-slash-commands');sys.exit(0)
prompt=sys.stdin.read()
native=sys.argv[sys.argv.index('--resume' if '--resume' in sys.argv else '--session-id')+1]
with (base/'calls').open('a') as f:f.write(json.dumps(dict(native=native,argv=sys.argv,prompt=prompt))+'\n')
if prompt.startswith('You are the configured orchestrator.'):
 if mode in ('management-executor-once','management-executor-unobserved'):
  if mode=='management-executor-once':print(json.dumps(dict(type='system',subtype='init',session_id=native,tools=['StructuredOutput'])),flush=True)
  sys.exit(1)
 c=json.JSONDecoder().raw_decode(prompt.split('Immutable orchestration context: ',1)[1])[0]
 team=c['team_template'] or dict(agents=[],tasks=[])
 roles={a['id']:a['role'] for a in team['agents']}
 for task in team['tasks']:
  if task['write_scopes'] or roles[task['owner_id']] in ('implementer','refactorer','test_implementer','integrator'):
   task['change_contract']=dict(kind='feature',rationale='Synthetic requested feature fixture',source_refs=task['source_dependencies'],bugfixes=[])
 if mode=='forged':team['agents'][0]['session_id']='00000000-0000-4000-8000-000000000099'
 if mode=='cycle':team['tasks'][0]['dependencies'].append(team['tasks'][0]['id'])
 if mode=='writes':team['tasks'][0]['write_scopes']=['source'];team['tasks'][0]['integration']='in_place'
 out=dict(message='Fixture proposal; no model reasoning or task execution asserted.',team_plan_json=json.dumps(team),risk_owners=[] if mode=='answer' or c.get('intake') else [dict(risk_id=r['id'],task_id=team['tasks'][0]['id'],rationale='Synthetic ownership fixture requiring semantic review') for r in c['risk_catalog']['risks']])
 if mode=='missing-risk':out['risk_owners'].pop()
 if mode=='answer':out.update(team_plan_json=None,risk_owners=[])
 if mode=='intake-guided' and not c['prior_rounds']:out.update(message='Jaki wynik chcesz otrzymać?',team_plan_json=None,risk_owners=[])
 if mode.startswith('intake-') and c.get('intake') and not (mode=='intake-guided' and not c['prior_rounds']):
  configuration=json.loads(json.dumps(c['spec']['config']['config']))
  configuration['policy']['revision']+=1
  if mode=='intake-role-removed':configuration['role_providers']={}
  selected=json.loads(json.dumps(configuration['profiles']['reasoning']))
  selected['model']='interview-selected-context-model'
  configuration['profiles']['interview-context']=selected
  configuration['routing']['context']['executor']='interview-context'
  if mode=='intake-discovered':
   discovered=c['provider_catalogue']['entries']['codex']
   configuration['providers']['codex']=discovered['configuration']
   configuration['profiles']['interview-context']['provider']='codex'
   configuration['profiles']['interview-context']['model']='selected-from-compatible-catalogue'

  configuration['profiles'][configuration['orchestrator']['profile']]['model']='interview-selected-orchestrator-model'
  task=dict(task_family='software',objective=c['spec']['objective'],requirements=[dict(id='R-CHAT',original_text=c['spec']['objective'],source_ref='user:chat',criteria=dict(result='Synthetic file-bound task criterion for transport validation'))],plan_items=[dict(id='P-CHAT',dependencies=[],requirement_ids=['R-CHAT'],description='Synthetic explicitly scoped plan')],input_files=['source'],editable_files=[],check_command=['python3','-c',"print('Synthetic scoped inspection')"])
  proposal=dict(task=task,configuration=configuration,rationale='Synthetic orchestrator decision; no semantic model quality asserted')
  if mode=='intake-binary':proposal['configuration']['providers']['claude']['binary']='/unapproved/model-supplied-cli'
  if mode=='intake-omission':proposal['task']['requirements'][0]['original_text']='lost original scope'
  out.update(team_plan_json=None,risk_owners=[],intake_proposal_json=json.dumps(proposal))

 if mode=='messages':out['agent_messages']=[dict(target=dict(type='agent',agent_id='Discovery:executor'),kind='question',text='Please report blockers through the message channel.'),dict(target=dict(type='broadcast'),kind='status',text='A proposal is under independent review; this is not task completion.')]
 if mode=='bad-messages':out['agent_messages']=[dict(target=dict(type='agent',agent_id='foreign-run:agent'),kind='status',text='Invalid route')]
 if mode=='mutate':(base/'workspace/source').write_text('unexpected effect');out=dict(message='',team_plan_json=None,risk_owners=[])
else:
 if mode in ('management-review-once','intake-recover-review'):
  print(json.dumps(dict(type='system',subtype='init',session_id=native,tools=['StructuredOutput'])),flush=True)
  sys.exit(1)
 c=json.JSONDecoder().raw_decode(prompt[prompt.index('\n{')+1:])[0]
 ids=[e['id'] for e in c['receipts'] if e['output']==c['context'] or e['output']==c['output']]
 if mode.startswith('intake-'):ids += [e['id'] for e in c['receipts'] if e['source']=='runtime:intake-sources']
 out=dict(contract_version=c['contract_version'],output_hash=c['output_hash'],spec_revision=c['spec_revision'],input_hash=c['input_hash'],checks=[dict(check_id=i,status='pass',method='Synthetic structured receipt fixture',evidence_ids=ids,rationale='Fixture only; no product quality assertion') for i in c['expected_checks']],findings=[])
 if mode=='reject' or mode=='intake-reject':out['checks'][0]['status']='fail';out['findings']=['Fixture rejection']
 if mode=='messages':out['agent_messages']=[dict(target=dict(type='agent',agent_id='orchestrator'),kind='answer',text='Independent review report is available as the task artifact.')]
 if mode=='bad-proof':out['checks'][0]['evidence_ids']=['invented']
print(json.dumps(dict(type='system',subtype='init',session_id=native,tools=['StructuredOutput'])))
print(json.dumps(dict(type='result',subtype='success',is_error=False,session_id=native,structured_output=out)))
"#.replace("__BASE__",&serde_json::to_string(root.path().to_str().unwrap()).unwrap()).replace("__MODE__",&serde_json::to_string(mode).unwrap());
        std::fs::write(&binary, script).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut config = Config::from_toml(include_str!("../examples/architect.toml")).unwrap();
        config.workspace.mode = WorkspaceMode::InPlace;
        config
            .providers
            .get_mut(&ProviderKind::Claude)
            .unwrap()
            .binary = binary;
        for profile in config.profiles.values_mut() {
            profile.provider = ProviderKind::Claude;
        }
        let mut review = config.profiles["review"].clone();
        review.model = Some("orchestration-review-model".into());
        review.work_kinds = [WorkKind::Orchestration].into();
        config
            .profiles
            .insert("orchestration-review".into(), review);
        config
            .routing
            .get_mut(&WorkKind::Orchestration)
            .unwrap()
            .verifier = "orchestration-review".into();
        config.profiles.get_mut("orchestrator").unwrap().model = Some("orchestrator-model".into());
        config.profiles.get_mut("review").unwrap().model = Some("stage-review-model".into());
        if mode.starts_with("intake-role-") {
            config.role_providers = [
                RoleKind::Orchestrator,
                RoleKind::Executor,
                RoleKind::Verifier,
            ]
            .into_iter()
            .map(|role| {
                (
                    role,
                    std::collections::BTreeSet::from([ProviderKind::Claude]),
                )
            })
            .collect();
        }
        let mut spec: RunSpec =
            serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
        spec.workspace = workspace.to_string_lossy().into();
        spec.input_files = vec!["source".into()];
        spec.check_command = vec![
            "python3".into(),
            "-c".into(),
            "print('Synthetic orchestration check plan fixture')".into(),
        ];
        spec.config = Some(config.pin().unwrap());
        let runtime =
            Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
        let run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();
        Self { root, runtime, run }
    }
    fn current(&self) -> Run {
        self.runtime
            .store
            .lock()
            .unwrap()
            .get(&self.run.id)
            .unwrap()
    }
    fn queue(&self) -> Run {
        self.runtime
            .dispatch(
                &id(),
                Command::SendMessage {
                    run_id: self.run.id.clone(),
                    target_agent_id: "orchestrator".into(),
                    text: "Propose a complete team; preserve every obligation 界".into(),
                },
            )
            .unwrap()
    }
    fn execute(&self) -> Run {
        let run = self.queue();
        self.runtime
            .dispatch(
                &id(),
                Command::Orchestrate {
                    run_id: run.id.clone(),
                    scope_hash: planning::start_scope(&run).unwrap(),
                },
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let current = self.current();
            if current.status != Status::Running {
                return current;
            }
            assert!(Instant::now() < deadline, "{:?}", current.diagnostic);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.runtime.dispatch(
            &id(),
            Command::Cancel {
                run_id: self.run.id.clone(),
            },
        );
    }
}

#[test]
fn actual_cli_round_is_independently_reviewed_then_installs_only_the_approved_team() {
    let f = Fixture::new("success");
    let reviewed = f.execute();
    assert_eq!(
        reviewed.status,
        Status::AwaitingStart,
        "{:?}",
        reviewed.diagnostic
    );
    let round = &reviewed.orchestration_rounds[0];
    assert_eq!(
        round.status,
        RoundStatus::Reviewed,
        "{:?}",
        round.diagnostic
    );
    assert!(reviewed.team.is_none());
    assert_eq!(reviewed.attempt, 0);
    assert_eq!(
        reviewed.active_session, f.run.active_session,
        "sidecar conversation must preserve the primary recovery checkpoint"
    );
    assert!(reviewed.artifacts.is_empty());
    let response = planning::authenticate(&reviewed, round).unwrap();
    assert_eq!(response.risk_owners.len(), 240);
    assert_ne!(round.executor_session, round.verifier_session);
    assert_eq!(
        reviewed
            .communication_ledger
            .messages
            .values()
            .next()
            .unwrap()
            .state,
        architect::communication::DeliveryState::Delivered
    );
    let calls: Vec<serde_json::Value> = std::fs::read_to_string(f.root.path().join("calls"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(calls.len(), 2);
    assert!(
        calls[0]["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "orchestrator-model")
    );
    assert!(
        calls[1]["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "orchestration-review-model")
    );
    let key = id();
    let command = Command::AcceptTeam {
        run_id: reviewed.id.clone(),
        round_id: round.id.clone(),
        scope_hash: planning::accept_scope(&reviewed, round).unwrap(),
    };
    let accepted = f.runtime.dispatch(&key, command.clone()).unwrap();
    let team = accepted.team.as_ref().unwrap();
    team.validate(&accepted.spec).unwrap();
    assert!(team.agents.iter().all(|a| a.profile.is_some()));
    assert_eq!(accepted.team_risk_owners.len(), 240);
    assert!(
        team.tasks
            .iter()
            .all(|t| t.state == architect::team::TaskState::Planned && t.attempt == 0)
    );
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::ApproveStart {
                    run_id: reviewed.id.clone(),
                    scope_hash: accepted.scope_hash("start")
                }
            )
            .is_err()
    );
    assert_eq!(
        f.runtime.dispatch(&key, command).unwrap().approvals.len(),
        accepted.approvals.len()
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/source")).unwrap(),
        "original"
    );
}

#[test]
fn malformed_proposals_and_rejected_or_forged_reviews_never_install_a_team() {
    for mode in [
        "forged",
        "cycle",
        "writes",
        "missing-risk",
        "reject",
        "bad-proof",
    ] {
        let f = Fixture::new(mode);
        let run = f.execute();
        let round = &run.orchestration_rounds[0];
        assert_eq!(
            round.status,
            RoundStatus::Rejected,
            "{mode}: {:?}",
            round.diagnostic
        );
        assert!(run.team.is_none());
        assert_eq!(run.status, Status::AwaitingStart);
        assert!(
            f.runtime
                .dispatch(
                    &id(),
                    Command::AcceptTeam {
                        run_id: run.id.clone(),
                        round_id: round.id.clone(),
                        scope_hash: planning::accept_scope(&run, round).unwrap()
                    }
                )
                .is_err()
        );
    }
}

#[test]
fn scope_drift_tampered_authorship_and_missing_configuration_fail_closed() {
    let f = Fixture::new("success");
    let queued = f.queue();
    let scope = planning::start_scope(&queued).unwrap();
    f.queue();
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::Orchestrate {
                    run_id: queued.id.clone(),
                    scope_hash: scope
                }
            )
            .is_err()
    );
    assert!(!f.root.path().join("calls").exists());
    let reviewed = f.execute();
    let round = reviewed.orchestration_rounds[0].clone();
    for mutation in 0..5 {
        let mut corrupt = reviewed.clone();
        if mutation == 0 {
            corrupt.orchestration_rounds[0].output = Some("{}".into());
        }
        if mutation == 1 {
            corrupt.orchestration_rounds[0].verifier_session = round.executor_session.clone();
        }
        if mutation == 2 {
            corrupt
                .input_snapshot
                .insert("source".into(), "changed".into());
        }
        if mutation == 3 {
            corrupt.approvals.clear();
        }
        if mutation == 4 {
            corrupt.orchestration_rounds[0].contract_version = "old-contract".into();
        }
        assert!(planning::authenticate(&corrupt, &corrupt.orchestration_rounds[0]).is_err());
    }
    std::fs::write(f.root.path().join("workspace/source"), "external").unwrap();
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::AcceptTeam {
                    run_id: reviewed.id.clone(),
                    round_id: round.id.clone(),
                    scope_hash: planning::accept_scope(&reviewed, &round).unwrap()
                }
            )
            .is_err()
    );
    let mut without_config = reviewed.clone();
    without_config.spec.config = None;
    assert!(architect::providers::build_for(&without_config, RoleKind::Orchestrator).is_err());
}

#[test]
fn conversational_answer_preserves_stage_and_malformed_output_cannot_hide_source_effects() {
    let f = Fixture::new("answer");
    let run = f.execute();
    assert_eq!(run.orchestration_rounds[0].status, RoundStatus::Reviewed);
    assert!(run.team.is_none());
    assert_eq!(run.attempt, 0);
    assert_eq!(run.status, Status::AwaitingStart);
    let f = Fixture::new("mutate");
    let run = f.execute();
    assert_eq!(run.status, Status::Blocked);
    assert!(run.reconciliation_required);
    assert_eq!(run.orchestration_rounds[0].status, RoundStatus::Failed);
}

#[test]
fn cli_and_tui_use_real_daemon_approvals_and_restore_reviewed_dialogue() {
    use architect::{
        ipc::{self, Operation, Request},
        tui::{
            backend::{Action, Backend, IpcBackend},
            model::Snapshot,
        },
    };
    use std::process::{Command as Process, Stdio};
    struct Daemon(std::process::Child);
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let f = Fixture::new("success");
    let state = f.root.path().join("daemon");
    let daemon = Daemon(
        Process::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("daemon")
            .arg(&state)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !state.join("architect.sock").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let reply = ipc::call(
        &state,
        &Request {
            version: ipc::PROTOCOL_VERSION,
            request_id: id(),
            idempotency_key: Some(id()),
            operation: Operation::Mutate {
                command: Box::new(Command::Create {
                    spec: f.run.spec.clone(),
                }),
            },
        },
    )
    .unwrap();
    assert!(reply.error.is_none());
    let rid = reply.data.unwrap()["id"].as_str().unwrap().to_owned();
    let mut backend = IpcBackend::new(&state);
    backend
        .act(
            &rid,
            Action::SendMessage {
                target_agent_id: "orchestrator".into(),
                text: "Propose the team".into(),
            },
        )
        .unwrap();
    let denied = Process::new(env!("CARGO_BIN_EXE_antislop"))
        .args(["orchestrate", state.to_str().unwrap(), &rid])
        .output()
        .unwrap();
    assert!(!denied.status.success());
    assert!(!f.root.path().join("calls").exists());
    let output = Process::new(env!("CARGO_BIN_EXE_antislop"))
        .args([
            "orchestrate",
            state.to_str().unwrap(),
            &rid,
            "--approve-scoped",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    let run = loop {
        let document = ipc::read_document(&state, &rid, "run");
        if let Ok(document) = document {
            let run: Run = serde_json::from_str(&document).unwrap();
            if run.status != Status::Running {
                break run;
            }
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(run.orchestration_rounds[0].status, RoundStatus::Reviewed);
    let snapshot = backend.snapshot(&rid).unwrap();
    assert_eq!(
        snapshot.reviewed_team_round.as_deref(),
        Some(run.orchestration_rounds[0].id.as_str())
    );
    assert!(
        snapshot
            .messages
            .iter()
            .any(|m| m.sender == "orchestrator" && m.state == "orchestration_reviewed")
    );
    assert!(snapshot.run_details["Options"].contains("team_plan_json"));
    let command = Action::AcceptTeam {
        round_id: run.orchestration_rounds[0].id.clone(),
        scope_hash: snapshot.scope_hashes["accept_team"].clone(),
    };
    backend.act(&rid, command).unwrap();
    let accepted: Run =
        serde_json::from_str(&ipc::read_document(&state, &rid, "run").unwrap()).unwrap();
    assert!(accepted.team.is_some());
    drop(daemon);
    let _replacement = Daemon(
        Process::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("daemon")
            .arg(&state)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let restored: Snapshot = loop {
        if let Ok(snapshot) = backend.snapshot(&rid) {
            break snapshot;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(restored.agents.iter().any(|a| a.id == "orchestrator"));
    assert!(restored.messages.iter().any(|m| m.sender == "orchestrator"));
}

#[test]
fn new_user_context_supersedes_old_acceptance_and_history_is_not_recursive() {
    let f = Fixture::new("success");
    let first = f.execute();
    let round = first.orchestration_rounds[0].clone();
    f.queue();
    let updated = f.current();
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::AcceptTeam {
                    run_id: first.id.clone(),
                    round_id: round.id.clone(),
                    scope_hash: planning::accept_scope(&updated, &round).unwrap()
                }
            )
            .is_err()
    );
    let second = f.execute();
    assert_eq!(second.orchestration_rounds.len(), 2);
    let context: serde_json::Value =
        serde_json::from_str(&second.orchestration_rounds[1].request_context).unwrap();
    assert!(context["prior_rounds"][0].get("request_context").is_none());
    assert_eq!(
        context["prior_rounds"][0]["output"].as_str(),
        round.output.as_deref()
    );
    assert_eq!(second.orchestration_rounds[1].status, RoundStatus::Reviewed);
}

#[test]
fn answering_a_question_preserves_a_reconciled_primary_failure_checkpoint() {
    let f = Fixture::new("answer");
    let mut prior = f.current();
    prior.status = Status::Blocked;
    prior.provider_failure = Some(architect::providers::ProviderFailure {
        provider: "claude".into(),
        kind: architect::providers::FailureKind::Network,
        message: "Historical fixture failure".into(),
        session_id: None,
        stopped: true,
        partial_effects: false,
    });
    prior.diagnostic = Some("Historical fixture failure".into());
    f.runtime
        .store
        .lock()
        .unwrap()
        .save(&prior, "fixture_reconciled_primary_failure", None)
        .unwrap();
    let answered = f.execute();
    assert_eq!(answered.status, Status::Blocked);
    assert_eq!(
        answered.orchestration_rounds[0].status,
        RoundStatus::Reviewed
    );
    assert_eq!(answered.diagnostic, prior.diagnostic);
    assert_eq!(
        serde_json::to_value(&answered.provider_failure).unwrap(),
        serde_json::to_value(&prior.provider_failure).unwrap()
    );
    assert_eq!(answered.active_session, prior.active_session);
}

#[test]
fn cancellation_interrupts_the_runtime_preflight_probe_and_releases_its_descendants() {
    let f = Fixture::new("hang-probe");
    let queued = f.queue();
    f.runtime
        .dispatch(
            &id(),
            Command::Orchestrate {
                run_id: queued.id.clone(),
                scope_hash: planning::start_scope(&queued).unwrap(),
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f.root.path().join("probe-started").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    f.runtime
        .dispatch(
            &id(),
            Command::Cancel {
                run_id: queued.id.clone(),
            },
        )
        .unwrap();
    loop {
        if architect::recovery::WorkspaceLease::acquire(&queued.spec.workspace).is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "cancel must stop even the initial version probe"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(f.current().status, Status::Cancelled);
    assert_eq!(
        f.current().orchestration_rounds[0].status,
        RoundStatus::Interrupted
    );
    assert!(f.current().session_ledger.sessions.is_empty());
    std::fs::write(f.root.path().join("release-late"), "").unwrap();
    std::thread::sleep(Duration::from_millis(120));
    assert!(!f.root.path().join("late-effect").exists());
}

#[test]
fn actual_cli_agents_use_direct_and_broadcast_outboxes_without_child_transcript_input() {
    use architect::{
        communication::{DeliveryState, Sender},
        providers::{ProviderEvent, TurnReceipt, UsageReport},
        sessions::{OutputRef, SessionRecord, SessionScope, TurnRecord},
    };
    let f = Fixture::new("messages");
    let mut seeded = f
        .runtime
        .dispatch(
            &id(),
            Command::SendMessage {
                run_id: f.run.id.clone(),
                target_agent_id: "Discovery:executor".into(),
                text: "PRIVATE_CHILD_INBOX_SENTINEL".into(),
            },
        )
        .unwrap();
    let session = SessionRecord::new(
        "historical-child".into(),
        SessionScope::for_run(
            &seeded,
            "Discovery:executor".into(),
            None,
            RoleKind::Executor,
            "fake".into(),
            "1".into(),
        )
        .unwrap(),
    )
    .unwrap();
    let turn = TurnRecord::new(
        "historical-turn".into(),
        session.id.clone(),
        architect::hash("prompt"),
        architect::hash("schema"),
        None,
    )
    .unwrap();
    seeded
        .session_ledger
        .reserve(session, turn, &mut |_, _| Ok(()))
        .unwrap();
    for event in [
        ProviderEvent::SessionObserved {
            session_id: "synthetic-history".into(),
        },
        ProviderEvent::DeliverySent {
            turn_id: "historical-turn".into(),
        },
        ProviderEvent::Completed {
            turn_id: "historical-turn".into(),
        },
    ] {
        seeded
            .session_ledger
            .record_provider_event("historical-turn", &event, &mut |_, _| Ok(()))
            .unwrap();
    }
    let raw = "CHILD_TRANSCRIPT_SENTINEL";
    seeded
        .session_ledger
        .complete(
            "historical-turn",
            &TurnReceipt {
                turn_id: "historical-turn".into(),
                provider: "fake".into(),
                session_id: "synthetic-history".into(),
                delivered: true,
                completed: true,
                stopped: true,
                output: raw.into(),
                usage: UsageReport::Unavailable,
            },
            OutputRef {
                document: "turn:historical-turn".into(),
                hash: architect::hash(raw),
            },
            &mut |_, _| Ok(()),
        )
        .unwrap();
    f.runtime
        .store
        .lock()
        .unwrap()
        .save(&seeded, "test_child_history", None)
        .unwrap();
    let reviewed = f.execute();
    let round = &reviewed.orchestration_rounds[0];
    assert_eq!(
        round.status,
        RoundStatus::Reviewed,
        "{:?}",
        reviewed.diagnostic
    );
    assert!(!round.request_context.contains("CHILD_TRANSCRIPT_SENTINEL"));
    assert!(
        !round
            .request_context
            .contains("PRIVATE_CHILD_INBOX_SENTINEL")
    );
    assert!(
        reviewed.session_ledger.turns["historical-turn"]
            .output
            .as_ref()
            .unwrap()
            .contains(raw)
    );
    let outgoing = reviewed
        .communication_ledger
        .messages
        .values()
        .filter(|m| matches!(m.sender, Sender::Agent { .. }))
        .collect::<Vec<_>>();
    assert_eq!(outgoing.len(), 15); // 1 direct + 13 broadcast recipients + reviewer answer
    assert!(
        outgoing
            .iter()
            .all(|m| m.state == DeliveryState::QueuedNextTurn)
    );
    assert!(outgoing.iter().any(|m| m.sender
        == Sender::Agent {
            agent_id: "orchestrator:verifier".into()
        }
        && m.recipient_agent_id == "orchestrator"));
    assert!(!planning::new_context_after_round(&reviewed, round));
    let accepted = f
        .runtime
        .dispatch(
            &id(),
            Command::AcceptTeam {
                run_id: reviewed.id.clone(),
                round_id: round.id.clone(),
                scope_hash: planning::accept_scope(&reviewed, round).unwrap(),
            },
        )
        .unwrap();
    assert!(
        accepted
            .communication_ledger
            .messages
            .values()
            .filter(|message| message.recipient_agent_id.starts_with("Discovery:"))
            .all(|message| message.state == DeliveryState::Cancelled)
    );
    let agents = accepted
        .team
        .as_ref()
        .unwrap()
        .agents
        .iter()
        .map(|a| a.id.clone())
        .collect::<Vec<_>>();
    let user_broadcast = f
        .runtime
        .dispatch(
            &id(),
            Command::SendMessage {
                run_id: accepted.id.clone(),
                target_agent_id: "*".into(),
                text: "A message for the installed team".into(),
            },
        )
        .unwrap();
    for agent in agents {
        assert!(
            user_broadcast
                .communication_ledger
                .messages
                .values()
                .any(|m| m.sender == Sender::User
                    && m.recipient_agent_id == agent
                    && user_broadcast.message_bodies[&m.id] == "A message for the installed team")
        );
    }
    let first = accepted
        .team
        .as_ref()
        .unwrap()
        .agents
        .iter()
        .find(|a| a.id != "orchestrator")
        .unwrap();
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::SendMessage {
                    run_id: accepted.id,
                    target_agent_id: first.id.clone(),
                    text: "Direct task question".into()
                }
            )
            .is_ok()
    );
}

#[test]
fn invalid_native_outbox_retains_completed_output_and_prevents_false_review_success() {
    let f = Fixture::new("bad-messages");
    let run = f.execute();
    assert_eq!(run.status, Status::Blocked);
    let round = &run.orchestration_rounds[0];
    assert_eq!(round.status, RoundStatus::Failed);
    assert!(round.verifier_session.is_none());
    let completed = run
        .session_ledger
        .turns
        .values()
        .find(|t| t.state == architect::sessions::TurnState::Completed)
        .unwrap();
    assert!(
        completed
            .output
            .as_ref()
            .unwrap()
            .contains("foreign-run:agent")
    );
    assert!(
        run.agent_outboxes[&completed.id]
            .rejection
            .as_ref()
            .unwrap()
            .contains("unknown agent")
    );
    assert!(
        run.communication_ledger
            .messages
            .values()
            .all(|m| m.sender == architect::communication::Sender::User)
    );
}

#[test]
fn chat_prompt_starts_native_interview_and_next_reply_without_spec_import() {
    use architect::{
        ipc,
        tui::backend::{Action, Backend, IpcBackend},
    };
    let f = Fixture::new("answer");
    let workspace = f.root.path().join("workspace");
    std::fs::write(workspace.join("architect.toml"), chat_project_config(&f)).unwrap();
    let state = f.root.path().join("chat-daemon");
    let daemon = architect::tui::connect_or_start_daemon(
        &state,
        std::path::Path::new(env!("CARGO_BIN_EXE_antislop")),
    )
    .unwrap()
    .unwrap();
    struct Daemon(std::process::Child);
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let _daemon = Daemon(daemon);
    let mut backend = IpcBackend::new(&state);
    let started = backend
        .begin_conversation_in("Potrzebuję raportu. Dopytaj o zakres.", &workspace)
        .unwrap();
    assert!(started.message_accepted);
    let rid = started.run_id;
    let notice = started.notice;
    assert!(notice.contains("turn started"), "{notice}");
    let read = || -> architect::Result<Run> {
        let text = ipc::read_document(&state, &rid, "run")?;
        architect::transport::decode(text.as_bytes())
    };
    let wait = |count| {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match read() {
                Ok(run) => {
                    if run.orchestration_rounds.len() == count && run.status != Status::Running {
                        return run;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "interview did not complete: {:?}",
                        run.diagnostic
                    );
                }
                Err(architect::Error::Invalid(message))
                    if message == "document changed; restart from offset zero" =>
                {
                    assert!(
                        Instant::now() < deadline,
                        "interview document did not stabilize"
                    );
                }
                Err(error) => panic!("interview read failed: {error}"),
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let first = wait(1);
    assert!(first.intake);
    assert_eq!(
        first.spec.objective,
        "Potrzebuję raportu. Dopytaj o zakres."
    );
    assert_eq!(
        first.orchestration_rounds[0].status,
        RoundStatus::AwaitingUser,
        "{:?}",
        first.diagnostic
    );
    assert!(first.team.is_none());
    assert!(
        first.orchestration_rounds[0]
            .request_context
            .contains("\"intake\":true")
    );
    assert!(
        first.orchestration_rounds[0]
            .request_context
            .contains("propose the suitable roles")
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("source")).unwrap(),
        "original"
    );
    let snapshot = backend.snapshot(&rid).unwrap();
    assert!(snapshot.intake);
    assert!(snapshot.awaiting_user_reply.is_some());
    assert!(first.orchestration_rounds[0].verifier_session.is_none());
    assert!(first.orchestration_rounds[0].review.is_none());
    assert!(architect::runtime::intake::pending(&first).is_none());
    assert!(planning::authenticate(&first, &first.orchestration_rounds[0]).is_err());
    assert!(planning::authenticate_interview_reply(&first, &first.orchestration_rounds[0]).is_ok());
    assert_eq!(snapshot.stage, "intake");
    assert!(snapshot.messages.iter().any(|m| m.sender == "orchestrator"));
    assert!(
        backend
            .act(
                &rid,
                Action::ChatMessage {
                    text: "Raport miesięczny, po polsku.".into()
                }
            )
            .unwrap()
            .contains("turn started")
    );
    let second = wait(2);
    assert_eq!(
        second.orchestration_rounds[1].status,
        RoundStatus::AwaitingUser,
        "{:?}",
        second.diagnostic
    );
    assert!(
        second.orchestration_rounds[1]
            .request_context
            .contains("Raport miesięczny")
    );
    assert!(
        second.orchestration_rounds[1]
            .request_context
            .contains("Potrzebuję raportu")
    );
    assert!(second.team.is_none());
    assert!(second.artifacts.is_empty());
    assert!(
        second
            .approvals
            .iter()
            .all(|a| a.action.starts_with("orchestrate:"))
    );
    let calls = std::fs::read_to_string(f.root.path().join("calls")).unwrap();
    assert_eq!(
        calls.lines().count(),
        2,
        "each interview reply waits for the user without launching the reviewer"
    );
}

#[test]
fn terminal_i_prompt_ctrl_s_reaches_the_native_orchestrator() {
    use architect::{
        ipc,
        tui::backend::{Backend, IpcBackend},
    };
    let f = Fixture::new("answer");
    let workspace = f.root.path().join("workspace");
    std::fs::write(workspace.join("architect.toml"), chat_project_config(&f)).unwrap();
    let state = f.root.path().join("pty-daemon");
    let daemon = architect::tui::connect_or_start_daemon(
        &state,
        std::path::Path::new(env!("CARGO_BIN_EXE_antislop")),
    )
    .unwrap()
    .unwrap();
    struct Daemon(std::process::Child);
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let _daemon = Daemon(daemon);
    let script = r#"
import os,pty,subprocess,select,time,fcntl,termios,struct,sys
master,slave=pty.openpty()
fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',30,120,0,0))
env=dict(os.environ,TERM='xterm-256color',ARCHITECT_ASCII='1',NO_COLOR='1')
p=subprocess.Popen([sys.argv[1],'ui',sys.argv[2]],cwd=sys.argv[3],stdin=slave,stdout=slave,stderr=slave,env=env)
os.close(slave)
output=b''
try:
 deadline=time.monotonic()+10
 while b'Antislop' not in output:
  assert p.poll() is None, ('UI exited',p.returncode,output[-2000:])
  assert time.monotonic()<deadline, ('UI startup timeout',output[-2000:])
  if select.select([master],[],[],.05)[0]:output+=os.read(master,65536)
 os.write(master,b'i')
 os.write(master,'Przygotuj raport. Najpierw wywiad.'.encode())
 os.write(master,b'\x13')
 deadline=time.monotonic()+15
 while not os.path.exists(sys.argv[4]) or len(open(sys.argv[4]).readlines())<1:
  assert time.monotonic()<deadline, ('Ctrl+S did not invoke the interview orchestrator',output[-2000:])
  if select.select([master],[],[],.05)[0]:output+=os.read(master,65536)
 os.write(master,b'\x03')
 p.wait(timeout=5)
 assert p.returncode==0, ('UI detach failed',p.returncode,output[-2000:])
finally:
 if p.poll() is None:p.kill();p.wait()
 os.close(master)
"#;
    let output = std::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(env!("CARGO_BIN_EXE_antislop"))
        .arg(&state)
        .arg(&workspace)
        .arg(f.root.path().join("calls"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut backend = IpcBackend::new(&state);
    let runs = backend.list_runs().unwrap();
    assert_eq!(runs.len(), 1);
    let run: Run =
        serde_json::from_str(&ipc::read_document(&state, &runs[0].id, "run").unwrap()).unwrap();
    assert!(run.intake);
    assert_eq!(run.spec.objective, "Przygotuj raport. Najpierw wywiad.");
    assert_eq!(
        run.message_bodies
            .values()
            .filter(|text| *text == "Przygotuj raport. Najpierw wywiad.")
            .count(),
        1
    );
    assert_eq!(run.orchestration_rounds.len(), 1);
    assert!(run.team.is_none());
}

fn chat_project_config(f: &Fixture) -> String {
    include_str!("../examples/architect.toml")
        .replace("mode = \"worktree\"", "mode = \"in-place\"")
        .replace("provider = \"codex\"", "provider = \"claude\"")
        .replace("provider = \"gemini\"", "provider = \"claude\"")
        .replace(
            "binary = \"claude\"",
            &format!(
                "binary = {}",
                serde_json::to_string(f.root.path().join("cli").to_str().unwrap()).unwrap()
            ),
        )
}

#[test]
fn first_message_commit_failure_retains_one_intake_and_an_unaccepted_draft() {
    use architect::{
        ipc,
        tui::backend::{Action, Backend, IpcBackend},
    };
    let f = Fixture::new("answer");
    let workspace = f.root.path().join("workspace");
    std::fs::write(workspace.join("architect.toml"), chat_project_config(&f)).unwrap();
    let state = f.root.path().join("refused-message-daemon");
    let daemon = architect::tui::connect_or_start_daemon(
        &state,
        std::path::Path::new(env!("CARGO_BIN_EXE_antislop")),
    )
    .unwrap()
    .unwrap();
    struct Daemon(std::process::Child);
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let _daemon = Daemon(daemon);
    let connection = rusqlite::Connection::open(state.join("state.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_first_message BEFORE INSERT ON events WHEN NEW.kind='message_queued_next_turn' BEGIN SELECT RAISE(ABORT, 'injected message commit failure'); END;").unwrap();
    let mut backend = IpcBackend::new(&state);
    let started = backend
        .begin_conversation_in("Dopytaj o zadanie", &workspace)
        .unwrap();
    assert!(
        !started.message_accepted,
        "the frontend must retain its draft without a confirmed message receipt"
    );
    assert!(started.notice.contains("unconfirmed"));
    assert!(started.notice.contains("Draft retained"));
    let runs = backend.list_runs().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, started.run_id);
    let run: Run =
        serde_json::from_str(&ipc::read_document(&state, &started.run_id, "run").unwrap()).unwrap();
    assert!(run.intake);
    assert_eq!(run.spec.objective, "Dopytaj o zadanie");
    assert!(run.orchestration_rounds.is_empty());
    assert!(run.message_bodies.is_empty());
    assert!(!f.root.path().join("calls").exists());
    connection
        .execute_batch("DROP TRIGGER refuse_first_message;")
        .unwrap();
    assert!(
        backend
            .act(
                &started.run_id,
                Action::ChatMessage {
                    text: "Dopytaj o zadanie".into()
                }
            )
            .unwrap()
            .contains("turn started")
    );
    assert_eq!(backend.list_runs().unwrap().len(), 1);
}

fn reviewed_interview(f: &Fixture) -> Run {
    let run = f
        .runtime
        .dispatch(
            &id(),
            Command::BeginIntake {
                objective: "Build the requested synthetic feature; interview before execution."
                    .into(),
                workspace: f.run.spec.workspace.clone(),
                config: f.run.spec.config.clone().unwrap(),
            },
        )
        .unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::SendMessage {
                run_id: run.id.clone(),
                target_agent_id: "orchestrator".into(),
                text: run.spec.objective.clone(),
            },
        )
        .unwrap();
    let current = f.runtime.store.lock().unwrap().get(&run.id).unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::Orchestrate {
                run_id: run.id.clone(),
                scope_hash: planning::start_scope(&current).unwrap(),
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let current = f.runtime.store.lock().unwrap().get(&run.id).unwrap();
        if current.status != Status::Running {
            return current;
        }
        assert!(
            Instant::now() < deadline,
            "interview did not complete: {:?}",
            current.diagnostic
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn interview_waits_without_verifier_spam_then_reviews_the_completed_proposal() {
    let f = Fixture::new("intake-guided");
    let waiting = reviewed_interview(&f);
    let round = &waiting.orchestration_rounds[0];
    assert_eq!(round.status, RoundStatus::AwaitingUser);
    assert!(round.review.is_none() && round.verifier_session.is_none());
    assert!(architect::runtime::intake::pending(&waiting).is_none());
    let calls = std::fs::read_to_string(f.root.path().join("calls")).unwrap();
    assert_eq!(calls.lines().count(), 1);
    // Refreshing a waiting conversation cannot manufacture a new native turn.
    for _ in 0..3 {
        let run = f.runtime.store.lock().unwrap().get(&waiting.id).unwrap();
        let snapshot = architect::tui::model::Snapshot::from_document(
            &serde_json::to_value(&run).unwrap(),
            &serde_json::json!({}),
        );
        assert_eq!(
            snapshot.awaiting_user_reply.as_deref(),
            Some(round.id.as_str())
        );
    }
    let queued = f
        .runtime
        .dispatch(
            &id(),
            Command::SendMessage {
                run_id: waiting.id.clone(),
                target_agent_id: "orchestrator".into(),
                text: "Raport po polsku, bez zmiany plików.".into(),
            },
        )
        .unwrap();
    let snapshot = architect::tui::model::Snapshot::from_document(
        &serde_json::to_value(&queued).unwrap(),
        &serde_json::json!({}),
    );
    assert!(snapshot.awaiting_user_reply.is_none());
    f.runtime
        .dispatch(
            &id(),
            Command::Orchestrate {
                run_id: waiting.id.clone(),
                scope_hash: planning::start_scope(&queued).unwrap(),
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let reviewed = loop {
        let run = f.runtime.store.lock().unwrap().get(&waiting.id).unwrap();
        if run.status != Status::Running {
            break run;
        }
        assert!(Instant::now() < deadline, "{:?}", run.diagnostic);
        std::thread::sleep(Duration::from_millis(5));
    };
    let round = reviewed.orchestration_rounds.last().unwrap();
    assert_eq!(
        round.status,
        RoundStatus::Reviewed,
        "{:?}",
        round.diagnostic
    );
    assert!(round.request_context.contains("Raport po polsku"));
    assert!(round.review.is_some() && round.verifier_session.is_some());
    let response = planning::authenticate(&reviewed, round).unwrap();
    let review_request = planning::verifier_request(&reviewed, round, &response).unwrap();
    assert!(review_request.prompt.contains("Do not block V08"));
    assert!(
        review_request
            .prompt
            .contains("Runtime alone authenticates")
    );
    let (round_id, scope_hash) = architect::runtime::intake::pending(&reviewed).unwrap();
    let accepted = f
        .runtime
        .dispatch(
            &id(),
            Command::AcceptIntake {
                run_id: reviewed.id,
                round_id,
                scope_hash,
            },
        )
        .unwrap();
    assert!(!accepted.intake && accepted.team.is_none());
    assert!(accepted.artifacts.is_empty());
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls"))
            .unwrap()
            .lines()
            .count(),
        3,
        "one question, one proposal, one independent review; acceptance starts no executor"
    );
}

#[test]
fn reviewed_interview_contract_binds_original_scope_sources_and_orchestrator_selected_configuration()
 {
    let f = Fixture::new("intake-proposal");
    let reviewed = reviewed_interview(&f);
    let round = &reviewed.orchestration_rounds[0];
    assert_eq!(
        round.status,
        RoundStatus::Reviewed,
        "{:?}",
        round.diagnostic
    );
    assert!(reviewed.intake);
    assert!(reviewed.team.is_none());
    assert!(reviewed.quality_gates.is_empty());
    assert_eq!(round.proposed_sources["source"], "original");
    let response = planning::authenticate(&reviewed, round).unwrap();
    assert!(planning::expected_checks(&reviewed, &response).contains("INTAKE:sources"));
    assert!(planning::expected_checks(&reviewed, &response).contains("REQ:R-CHAT:result"));
    let (round_id, scope_hash) = architect::runtime::intake::pending(&reviewed).unwrap();
    let calls = std::fs::read_to_string(f.root.path().join("calls")).unwrap();
    assert_eq!(calls.lines().count(), 2);
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::AcceptIntake {
                    run_id: reviewed.id.clone(),
                    round_id: round_id.clone(),
                    scope_hash: "wrong".into()
                }
            )
            .is_err()
    );
    let accepted = f
        .runtime
        .dispatch(
            &id(),
            Command::AcceptIntake {
                run_id: reviewed.id.clone(),
                round_id,
                scope_hash,
            },
        )
        .unwrap();
    assert!(!accepted.intake);
    assert!(accepted.chat_origin);
    assert_eq!(accepted.spec_revision, 2);
    assert_eq!(
        accepted.spec.requirements[0].original_text,
        reviewed.spec.objective
    );
    assert_eq!(accepted.spec.requirements[0].source_ref, "user:chat");
    assert_eq!(accepted.input_snapshot["source"], "original");
    assert_eq!(
        accepted
            .spec
            .config
            .as_ref()
            .unwrap()
            .config
            .resolve(WorkKind::Context, RoleKind::Executor)
            .unwrap()
            .model
            .as_deref(),
        Some("interview-selected-context-model")
    );
    assert!(accepted.team.is_none());
    assert!(accepted.artifacts.is_empty());
    assert!(accepted.quality_gates.is_empty());
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls")).unwrap(),
        calls
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/source")).unwrap(),
        "original"
    );
    assert!(
        accepted
            .approvals
            .iter()
            .any(|a| a.action.starts_with("intake:")
                && a.actor == "local_user"
                && a.spec_revision == 1)
    );
    assert_eq!(
        accepted.orchestration_rounds[0].status,
        RoundStatus::Accepted
    );
}

#[test]
fn interview_configuration_and_obligation_omissions_or_rejected_review_cannot_be_accepted() {
    for mode in ["intake-binary", "intake-omission", "intake-reject"] {
        let f = Fixture::new(mode);
        let run = reviewed_interview(&f);
        assert_eq!(
            run.orchestration_rounds[0].status,
            RoundStatus::Rejected,
            "{mode}: {:?}",
            run.orchestration_rounds[0].diagnostic
        );
        assert!(run.intake);
        assert!(architect::runtime::intake::pending(&run).is_none());
        assert!(
            f.runtime
                .dispatch(
                    &id(),
                    Command::AcceptIntake {
                        run_id: run.id.clone(),
                        round_id: run.orchestration_rounds[0].id.clone(),
                        scope_hash: "unreviewed".into()
                    }
                )
                .is_err()
        );
        assert!(run.team.is_none());
        assert!(run.artifacts.is_empty());
    }
}

#[test]
fn interview_acceptance_rejects_changed_sources_new_messages_and_forged_native_source_review() {
    let f = Fixture::new("intake-proposal");
    let reviewed = reviewed_interview(&f);
    assert_eq!(
        reviewed.orchestration_rounds[0].status,
        RoundStatus::Reviewed,
        "{:?}",
        reviewed.orchestration_rounds[0].diagnostic
    );
    let (round_id, scope_hash) = architect::runtime::intake::pending(&reviewed).unwrap();
    std::fs::write(f.root.path().join("workspace/source"), "changed").unwrap();
    assert!(architect::runtime::intake::pending(&reviewed).is_none());
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::AcceptIntake {
                    run_id: reviewed.id.clone(),
                    round_id: round_id.clone(),
                    scope_hash: scope_hash.clone()
                }
            )
            .is_err()
    );
    let mut forged = reviewed.clone();
    forged.orchestration_rounds[0]
        .proposed_sources
        .insert("source".into(), "changed".into());
    let eid = forged.orchestration_rounds[0]
        .intake_evidence_id
        .clone()
        .unwrap();
    let bytes = serde_json::to_string(&forged.orchestration_rounds[0].proposed_sources).unwrap();
    let evidence = forged.evidence.iter_mut().find(|e| e.id == eid).unwrap();
    evidence.output = bytes.clone();
    evidence.output_hash = architect::hash(&bytes);
    evidence.artifact_hash = architect::hash(&bytes);
    let (forged_id, forged_scope) = architect::runtime::intake::pending(&forged).unwrap();
    assert!(
        architect::runtime::intake::accept(&mut forged, &forged_id, &forged_scope)
            .unwrap_err()
            .to_string()
            .contains("actual native input")
    );
    std::fs::write(f.root.path().join("workspace/source"), "original").unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::SendMessage {
                run_id: reviewed.id.clone(),
                target_agent_id: "orchestrator".into(),
                text: "A new obligation invalidates the old contract.".into(),
            },
        )
        .unwrap();
    let current = f.runtime.store.lock().unwrap().get(&reviewed.id).unwrap();
    assert!(architect::runtime::intake::pending(&current).is_none());
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::AcceptIntake {
                    run_id: reviewed.id.clone(),
                    round_id,
                    scope_hash
                }
            )
            .is_err()
    );
}

#[test]
fn daemon_ui_accepts_the_reviewed_interview_then_plans_and_installs_its_selected_team() {
    use architect::{
        ipc,
        tui::backend::{Action, Backend, IpcBackend},
    };
    let f = Fixture::new("intake-proposal");
    let workspace = f.root.path().join("workspace");
    std::fs::write(workspace.join("architect.toml"), chat_project_config(&f)).unwrap();
    let state = f.root.path().join("interview-to-team");
    let daemon = architect::tui::connect_or_start_daemon(
        &state,
        std::path::Path::new(env!("CARGO_BIN_EXE_antislop")),
    )
    .unwrap()
    .unwrap();
    struct Daemon(std::process::Child);
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut daemon = Daemon(daemon);
    let mut backend = IpcBackend::new(&state);
    let started = backend
        .begin_conversation_in(
            "Implement the scoped synthetic feature after an interview.",
            &workspace,
        )
        .unwrap();
    assert!(started.message_accepted);
    let rid = started.run_id;
    let wait = |backend: &mut IpcBackend, team: bool| {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match backend.snapshot(&rid) {
                Ok(snapshot)
                    if if team {
                        snapshot.reviewed_team_round.is_some()
                    } else {
                        snapshot.reviewed_intake_proposal.is_some()
                    } =>
                {
                    return snapshot;
                }
                Ok(snapshot) => assert!(
                    Instant::now() < deadline,
                    "proposal unavailable: {}",
                    snapshot.diagnostic
                ),
                Err(error)
                    if error.to_string().contains("document changed")
                        || error.to_string().contains("run changed between") =>
                {
                    assert!(Instant::now() < deadline, "snapshot did not stabilize")
                }
                Err(error) => panic!("snapshot failed: {error}"),
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let snapshot = wait(&mut backend, false);
    assert!(
        snapshot
            .intake_proposal_details
            .contains("interview-selected-context-model")
    );
    assert!(
        snapshot
            .intake_proposal_details
            .contains("Synthetic scoped inspection")
    );
    assert!(snapshot.intake_proposal_details.contains("INTAKE:sources"));
    let (round_id, scope_hash) = snapshot.reviewed_intake_proposal.unwrap();
    let notice = backend
        .act(
            &rid,
            Action::AcceptIntake {
                round_id,
                scope_hash,
            },
        )
        .unwrap();
    assert!(notice.contains("turn started"), "{notice}");
    let planning = wait(&mut backend, true);
    assert!(!planning.intake);
    assert!(planning.chat_origin);
    let raw = ipc::read_document(&state, &rid, "run").unwrap();
    let before: Run = serde_json::from_str(&raw).unwrap();
    assert_eq!(before.spec_revision, 2);
    assert!(before.team.is_none());
    assert!(before.artifacts.is_empty());
    assert!(before.quality_gates.is_empty());
    backend
        .act(
            &rid,
            Action::AcceptTeam {
                round_id: planning.reviewed_team_round.unwrap(),
                scope_hash: planning.scope_hashes["accept_team"].clone(),
            },
        )
        .unwrap();
    let installed: Run =
        serde_json::from_str(&ipc::read_document(&state, &rid, "run").unwrap()).unwrap();
    let team = installed.team.as_ref().unwrap();
    let context = team
        .agents
        .iter()
        .find(|a| a.kind == RoleKind::Executor && a.role.work_kind() == WorkKind::Context)
        .unwrap();
    assert_eq!(
        context.profile.as_ref().unwrap().model.as_deref(),
        Some("interview-selected-context-model")
    );
    assert!(
        team.tasks
            .iter()
            .all(|t| t.state == architect::team::TaskState::Planned && t.attempt == 0)
    );
    assert!(installed.artifacts.is_empty());
    assert!(installed.quality_gates.is_empty());
    assert_eq!(
        std::fs::read_to_string(workspace.join("source")).unwrap(),
        "original"
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls"))
            .unwrap()
            .lines()
            .count(),
        4
    );
    let invocations: Vec<serde_json::Value> = std::fs::read_to_string(f.root.path().join("calls"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(invocations.iter().any(|call| {
        call["argv"]
            .as_array()
            .unwrap()
            .windows(2)
            .any(|args| args[0] == "--model" && args[1] == "interview-selected-orchestrator-model")
    }));

    daemon.0.kill().unwrap();
    daemon.0.wait().unwrap();
    let replacement = Daemon(
        std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("daemon")
            .arg(&state)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let _replacement = replacement;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(restored) = backend.snapshot(&rid) {
            assert!(restored.chat_origin);
            assert!(!restored.intake);
            assert!(restored.reviewed_intake_proposal.is_none());
            let run: Run =
                serde_json::from_str(&ipc::read_document(&state, &rid, "run").unwrap()).unwrap();
            assert_eq!(
                run.spec.config.as_ref().unwrap(),
                installed.spec.config.as_ref().unwrap()
            );
            assert_eq!(run.orchestration_rounds.len(), 2);
            assert!(
                run.approvals
                    .iter()
                    .any(|a| a.action.starts_with("intake:"))
            );
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls"))
            .unwrap()
            .lines()
            .count(),
        4
    );
}

#[test]
fn intake_acceptance_commit_failure_preserves_the_reviewed_scope_and_does_not_launch_work() {
    let f = Fixture::new("intake-proposal");
    let reviewed = reviewed_interview(&f);
    let (round_id, scope_hash) = architect::runtime::intake::pending(&reviewed).unwrap();
    let calls = std::fs::read_to_string(f.root.path().join("calls")).unwrap();
    let connection = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_intake_acceptance BEFORE INSERT ON events WHEN NEW.kind='intake_accepted' BEGIN SELECT RAISE(ABORT, 'injected contract acceptance failure'); END;").unwrap();
    let command = Command::AcceptIntake {
        run_id: reviewed.id.clone(),
        round_id,
        scope_hash,
    };
    assert!(
        f.runtime
            .dispatch("refused-contract", command.clone())
            .is_err()
    );
    let current = f.runtime.store.lock().unwrap().get(&reviewed.id).unwrap();
    assert!(current.intake);
    assert_eq!(current.spec_revision, 1);
    assert_eq!(current.spec.config, reviewed.spec.config);
    assert_eq!(current.approvals.len(), reviewed.approvals.len());
    assert_eq!(
        current.orchestration_rounds[0].status,
        RoundStatus::Reviewed
    );
    assert!(current.input_snapshot.is_empty());
    assert!(current.team.is_none());
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls")).unwrap(),
        calls
    );
    connection
        .execute_batch("DROP TRIGGER refuse_intake_acceptance;")
        .unwrap();
    let accepted = f
        .runtime
        .dispatch("refused-contract", command.clone())
        .unwrap();
    assert!(!accepted.intake);
    assert_eq!(accepted.spec_revision, 2);
    assert_eq!(
        f.runtime
            .dispatch("refused-contract", command)
            .unwrap()
            .spec_revision,
        2
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls")).unwrap(),
        calls
    );
}

#[test]
fn terminal_interview_cards_accept_contract_and_team_without_a_specification_form() {
    let f = Fixture::new("intake-proposal");
    let workspace = f.root.path().join("workspace");
    std::fs::write(workspace.join("architect.toml"), chat_project_config(&f)).unwrap();
    let state = f.root.path().join("two-card-terminal");
    let daemon = architect::tui::connect_or_start_daemon(
        &state,
        std::path::Path::new(env!("CARGO_BIN_EXE_antislop")),
    )
    .unwrap()
    .unwrap();
    struct Daemon(std::process::Child);
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let _daemon = Daemon(daemon);
    let script = r#"
import os,pty,subprocess,select,time,fcntl,termios,struct,sys,sqlite3,json,signal,re
master,slave=pty.openpty()
fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',30,120,0,0))
env=dict(os.environ,TERM='xterm-256color',ARCHITECT_ASCII='1',NO_COLOR='1')
p=subprocess.Popen([sys.argv[1],'ui',sys.argv[2]],cwd=sys.argv[3],stdin=slave,stdout=slave,stderr=slave,env=env)
os.close(slave)
output=b''
resize_count=0
# Inspect rendered cells, since Ratatui may write a word in several cursor/style
# chunks. Raw transport substrings are not a terminal screen assertion.
cells={}; cursor=[0,0]; vt_pending=b''
def render_cells(chunk):
 global vt_pending
 data=vt_pending+chunk; vt_pending=b''; i=0
 while i<len(data):
  byte=data[i]
  if byte==27:
   if i+1>=len(data):vt_pending=data[i:];break
   if data[i+1]==91:
    match=re.match(rb'\x1b\[([0-9;?]*)([@-~])',data[i:])
    if not match:vt_pending=data[i:];break
    params,command=match.groups();i+=len(match[0])
    nums=[int(x or b'0') for x in params.lstrip(b'?').split(b';')]
    n=nums[0] or 1
    if command in (b'H',b'f'):cursor[:]=[n-1,(nums[1] or 1)-1 if len(nums)>1 else 0]
    elif command==b'G':cursor[1]=n-1
    elif command==b'A':cursor[0]=max(0,cursor[0]-n)
    elif command==b'B':cursor[0]+=n
    elif command==b'C':cursor[1]+=n
    elif command==b'D':cursor[1]=max(0,cursor[1]-n)
    elif command==b'J' and nums[0] in (2,3):cells.clear()
    elif command==b'K':
     for key in list(cells):
      if key[0]==cursor[0] and (nums[0]==2 or nums[0]==0 and key[1]>=cursor[1] or nums[0]==1 and key[1]<=cursor[1]):del cells[key]
    elif params==b'?1049' and command==b'h':cells.clear();cursor[:]=[0,0]
    continue
   i+=2;continue
  if byte==13:cursor[1]=0
  elif byte==10:cursor[0]+=1
  elif byte==8:cursor[1]=max(0,cursor[1]-1)
  elif byte>=32:cells[tuple(cursor)]=byte;cursor[1]+=1
  i+=1
def visible():
 return b'\n'.join(bytes(cells.get((row,col),32) for col in range(130)) for row in range(35))
def drain(timeout=.05):
 global output
 if select.select([master],[],[],timeout)[0]:
  chunk=os.read(master,65536);output+=chunk;render_cells(chunk)
def current():
 with sqlite3.connect(os.path.join(sys.argv[2],'state.db')) as db:row=db.execute('SELECT state FROM runs').fetchone()
 return json.loads(row[0]) if row else None
def wait(predicate):
 deadline=time.monotonic()+15
 while True:
  assert p.poll() is None, ('frontend exited',p.returncode,output[-2000:])
  r=current()
  if r and predicate(r):return r
  assert time.monotonic()<deadline, ('terminal flow timeout',r and r.get('diagnostic'),output[-2000:])
  drain()
def discard_pending():
 global output
 while select.select([master],[],[],0)[0]:drain(0)
 output=b''
def wait_ui(token):
 deadline=time.monotonic()+5
 while token not in (output if token.startswith(b'\x1b') else visible()):
  assert p.poll() is None, ('UI exited',p.returncode,output[-2000:])
  assert time.monotonic()<deadline, ('UI state missing',token,output[-2000:])
  drain()
def rendered_review():
 global resize_count
 discard_pending()
 resize_count+=1
 h,w=(31,121) if resize_count%2 else (30,120)
 # Alternate actual sizes so a redraw cannot reuse a same-size diff buffer.
 fcntl.ioctl(master,termios.TIOCSWINSZ,struct.pack('HHHH',h,w,0,0))
 os.kill(p.pid,signal.SIGWINCH)
 # A new rendered frame may show the cursor while the composer is focused.
 wait_ui(b'\x1b[?25')
 wait_ui(b'Reviewed')
def open_card(leave_editor):
 if leave_editor:
  discard_pending()
  os.write(master,b'\x1b')
  wait_ui(b'\x1b[?25l')
 discard_pending()
 os.write(master,b'a')
 wait_ui(b'Confirm')
 os.write(master,b'\r')
try:
 deadline=time.monotonic()+10
 while b'Antislop' not in output:
  assert time.monotonic()<deadline, ('UI did not draw',output[-2000:])
  drain()
 os.write(master,b'iImplement the synthetic scoped task.\x13')
 wait(lambda r:r.get('intake') and r['orchestration_rounds'] and r['orchestration_rounds'][-1]['status']=='reviewed')
 rendered_review()
 open_card(True)
 wait(lambda r:not r.get('intake') and len(r['orchestration_rounds'])==2 and r['orchestration_rounds'][-1]['status']=='reviewed')
 rendered_review()
 open_card(False)
 r=wait(lambda r:r.get('team') is not None)
 assert r['chat_origin'] and r['spec_revision']==2
 assert all(t['state']=='planned' and t['attempt']==0 for t in r['team']['tasks'])
 assert not r['artifacts'] and not r['quality_gates']
 os.write(master,b'\x03')
 p.wait(timeout=5)
 assert p.returncode==0
finally:
 if p.poll() is None:p.kill();p.wait()
 os.close(master)
"#;
    let output = std::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(env!("CARGO_BIN_EXE_antislop"))
        .arg(&state)
        .arg(&workspace)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls"))
            .unwrap()
            .lines()
            .count(),
        4
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("source")).unwrap(),
        "original"
    );
}

#[test]
fn daemon_discovery_exposes_new_compatible_cli_and_binds_the_orchestrator_selection_to_its_observation()
 {
    use architect::{
        ipc,
        tui::backend::{Action, Backend, IpcBackend},
    };
    let f = Fixture::new("intake-discovered");
    let workspace = f.root.path().join("workspace");
    let configured = chat_project_config(&f);
    // Keep only the explicitly configured Claude provider; every profile in
    // this test config already selects it. Codex comes solely from discovery.
    let mut config = Config::from_toml(&configured).unwrap();
    config
        .providers
        .retain(|kind, _| *kind == ProviderKind::Claude);
    let initial = f
        .runtime
        .dispatch(
            &id(),
            Command::BeginIntake {
                objective: "Choose agents using the observed local CLI catalogue.".into(),
                workspace: workspace.to_string_lossy().into(),
                config: config.pin().unwrap(),
            },
        )
        .unwrap();
    let state = f.root.path().join("discovered-daemon");
    let bin = f.root.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let codex = bin.join("codex");
    std::fs::write(&codex, "#!/bin/sh\nprintf 'codex-cli 0.162.1\\n'\n").unwrap();
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o700)).unwrap();
    struct Daemon(std::process::Child);
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let search = std::env::join_paths(std::iter::once(bin.clone()).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .unwrap();
    let _daemon = Daemon(
        std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("daemon")
            .arg(&state)
            .env("PATH", search)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !state.join("architect.sock").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let created = ipc::call(
        &state,
        &ipc::Request {
            version: ipc::PROTOCOL_VERSION,
            request_id: id(),
            idempotency_key: Some(id()),
            operation: ipc::Operation::Mutate {
                command: Box::new(Command::BeginIntake {
                    objective: initial.spec.objective.clone(),
                    workspace: initial.spec.workspace.clone(),
                    config: initial.spec.config.clone().unwrap(),
                }),
            },
        },
    )
    .unwrap();
    assert!(created.error.is_none());
    let rid = created.data.unwrap()["id"].as_str().unwrap().to_owned();
    let mut backend = IpcBackend::new(&state);
    assert!(
        backend
            .act(
                &rid,
                Action::ChatMessage {
                    text: initial.spec.objective.clone()
                }
            )
            .unwrap()
            .contains("turn started")
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    let snapshot = loop {
        match backend.snapshot(&rid) {
            Ok(s) if s.reviewed_intake_proposal.is_some() => break s,
            Ok(s) => assert!(
                Instant::now() < deadline,
                "discovery/proposal unavailable: {}",
                s.diagnostic
            ),
            Err(e)
                if e.to_string().contains("document changed")
                    || e.to_string().contains("run changed between") =>
            {
                assert!(Instant::now() < deadline)
            }
            Err(e) => panic!("snapshot: {e}"),
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(
        snapshot
            .provider_catalogue_details
            .contains("codex-cli 0.162.1")
    );
    assert!(snapshot.provider_catalogue_details.contains("unverified"));
    let mut run: Run =
        serde_json::from_str(&ipc::read_document(&state, &rid, "run").unwrap()).unwrap();
    let round = &run.orchestration_rounds[0];
    let catalogue = round.provider_catalogue.as_ref().unwrap();
    assert!(matches!(
        catalogue.entries[&ProviderKind::Codex].state,
        architect::providers::discovery::State::Compatible
    ));
    assert_eq!(
        catalogue.entries[&ProviderKind::Codex].configuration.binary,
        codex.canonicalize().unwrap()
    );
    assert!(round.request_context.contains("provider_catalogue"));
    assert!(
        !run.spec
            .config
            .as_ref()
            .unwrap()
            .config
            .providers
            .contains_key(&ProviderKind::Codex)
    );
    let (round_id, scope) = snapshot.reviewed_intake_proposal.unwrap();
    let mut forged = run.clone();
    forged.orchestration_rounds[0]
        .provider_catalogue
        .as_mut()
        .unwrap()
        .entries
        .get_mut(&ProviderKind::Codex)
        .unwrap()
        .configuration
        .binary = "/unobserved/model-binary".into();
    assert!(architect::runtime::intake::pending(&forged).is_none());
    assert!(architect::runtime::intake::accept(&mut forged, &round_id, &scope).is_err());
    architect::runtime::intake::accept(&mut run, &round_id, &scope).unwrap();
    let profile = run
        .spec
        .config
        .as_ref()
        .unwrap()
        .config
        .resolve(WorkKind::Context, RoleKind::Executor)
        .unwrap();
    assert_eq!(profile.provider, ProviderKind::Codex);
    assert_eq!(profile.binary, codex.canonicalize().unwrap());
    assert_eq!(
        profile.model.as_deref(),
        Some("selected-from-compatible-catalogue")
    );
    assert!(run.artifacts.is_empty());
    assert!(run.team.is_none());
    assert_eq!(
        std::fs::read_to_string(workspace.join("source")).unwrap(),
        "original"
    );
}

#[test]
fn explicit_initial_claude_without_project_config_waits_for_the_interview_answer() {
    use architect::{
        ipc,
        tui::backend::{Backend, IpcBackend},
    };
    let f = Fixture::new("answer");
    let workspace = f.root.path().join("workspace");
    assert!(!workspace.join("architect.toml").exists());
    let bin = f.root.path().join("initial-cli-bin");
    std::fs::create_dir(&bin).unwrap();
    std::os::unix::fs::symlink(f.root.path().join("cli"), bin.join("claude")).unwrap();
    std::fs::write(
        bin.join("codex"),
        "#!/bin/sh\nprintf 'codex-cli 99.0.0\\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(bin.join("codex"), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let state = f.root.path().join("explicit-bootstrap");
    let daemon = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("daemon")
        .arg(&state)
        .env("PATH", std::env::join_paths(paths).unwrap())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    struct Daemon(std::process::Child);
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let _daemon = Daemon(daemon);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !state.join("architect.sock").exists() {
        assert!(
            Instant::now() < deadline,
            "bootstrap daemon startup timeout"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut backend = IpcBackend::new(&state).with_bootstrap_provider(ProviderKind::Claude);
    let started = backend
        .begin_conversation_in("Start from chat; ask before planning", &workspace)
        .unwrap();
    assert!(started.message_accepted, "{}", started.notice);
    let deadline = Instant::now() + Duration::from_secs(15);
    let run = loop {
        match ipc::read_document(&state, &started.run_id, "run") {
            Ok(text) => {
                let run: Run = architect::transport::decode(text.as_bytes()).unwrap();
                if !run.orchestration_rounds.is_empty() && run.status != Status::Running {
                    break run;
                }
                assert!(
                    Instant::now() < deadline,
                    "bootstrap interview did not stop: {:?}",
                    run.diagnostic
                );
            }
            Err(architect::Error::Invalid(message))
                if message == "document changed; restart from offset zero" =>
            {
                assert!(
                    Instant::now() < deadline,
                    "bootstrap document did not stabilize"
                );
            }
            Err(error) => panic!("bootstrap read failed: {error}"),
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(run.intake && run.team.is_none());
    assert!(run.artifacts.is_empty() && run.spec.editable_files.is_empty());
    assert_eq!(
        run.orchestration_rounds[0].status,
        RoundStatus::AwaitingUser,
        "{:?}",
        run.diagnostic
    );
    let config = &run.spec.config.as_ref().unwrap().config;
    assert_eq!(config.providers.len(), 1);
    assert_eq!(
        config
            .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
            .unwrap()
            .provider,
        ProviderKind::Claude
    );
    let catalogue = run.orchestration_rounds[0]
        .provider_catalogue
        .as_ref()
        .unwrap();
    assert!(matches!(
        catalogue.entries[&ProviderKind::Claude].state,
        architect::providers::discovery::State::Compatible
    ));
    assert!(matches!(
        catalogue.entries[&ProviderKind::Codex].state,
        architect::providers::discovery::State::Rejected
    ));
    let calls = std::fs::read_to_string(f.root.path().join("calls")).unwrap();
    let calls: Vec<serde_json::Value> = calls
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        calls.len(),
        1,
        "provisional interview runs only the orchestrator"
    );
    assert!(
        calls[0]["prompt"]
            .as_str()
            .unwrap()
            .starts_with("You are the configured orchestrator.")
    );
    assert!(calls.iter().all(|call| {
        !call["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "--model")
    }));
    assert_eq!(
        std::fs::read_to_string(workspace.join("source")).unwrap(),
        "original"
    );
    assert_eq!(backend.list_runs().unwrap().len(), 1);
    let native_calls_before = std::fs::read_to_string(f.root.path().join("calls")).unwrap();
    let rejected = IpcBackend::new(&state)
        .with_bootstrap_provider(ProviderKind::Codex)
        .begin_conversation_in(
            "Keep this explicit CLI choice even when unsupported",
            &workspace,
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let rejected_run = loop {
        match ipc::read_document(&state, &rejected.run_id, "run") {
            Ok(text) => {
                let run: Run = architect::transport::decode(text.as_bytes()).unwrap();
                if !run.orchestration_rounds.is_empty() && run.status != Status::Running {
                    break run;
                }
                assert!(
                    Instant::now() < deadline,
                    "rejected bootstrap did not stop: {:?}",
                    run.diagnostic
                );
            }
            Err(architect::Error::Invalid(message))
                if message == "document changed; restart from offset zero" =>
            {
                assert!(
                    Instant::now() < deadline,
                    "rejected bootstrap document did not stabilize"
                );
            }
            Err(error) => panic!("rejected bootstrap read failed: {error}"),
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(
        rejected_run.orchestration_rounds[0].status,
        RoundStatus::Failed,
        "{:?}",
        rejected_run.diagnostic
    );
    assert!(rejected_run.intake && rejected_run.team.is_none());
    assert_eq!(rejected_run.spec.provider, "codex");
    assert!(
        rejected_run
            .diagnostic
            .as_deref()
            .unwrap_or_default()
            .contains("unsupported")
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls")).unwrap(),
        native_calls_before,
        "a rejected explicitly chosen Codex must not fall back to the compatible Claude"
    );
}

fn reconciled_management(f: &Fixture, run: &Run) -> Run {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let observation = architect::recovery::observe(run).unwrap();
        match f.runtime.dispatch(
            &id(),
            Command::Reconcile {
                run_id: run.id.clone(),
                scope_hash: run.scope_hash("reconcile"),
                input_hash: observation.hash,
            },
        ) {
            Ok(run) => return run,
            Err(error) if error.to_string().contains("worker is still stopping") => {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("management reconciliation failed: {error}"),
        }
    }
}
fn management_idle(f: &Fixture) -> Run {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let run = f.current();
        if run.status != Status::Running {
            return run;
        }
        assert!(Instant::now() < deadline, "{:?}", run.diagnostic);
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn management_recovery_preserves_round_and_completed_executor_for_exact_and_replacement_sessions() {
    use architect::runtime::orchestration_recovery::{self, Kind};
    for (mode, kind, expected_role) in [
        (
            "management-executor-once",
            Kind::Resume,
            RoleKind::Orchestrator,
        ),
        (
            "management-executor-unobserved",
            Kind::Replacement,
            RoleKind::Orchestrator,
        ),
        ("management-review-once", Kind::Resume, RoleKind::Verifier),
        (
            "management-review-once",
            Kind::Replacement,
            RoleKind::Verifier,
        ),
    ] {
        let f = Fixture::new(mode);
        let failed = f.execute();
        assert_eq!(failed.status, Status::Blocked);
        let before = reconciled_management(&f, &failed);
        let rid = before.orchestration_rounds[0].id.clone();
        let p = orchestration_recovery::for_round(&before, &rid, kind).unwrap();
        assert_eq!(p.role, expected_role);
        let calls = std::fs::read_to_string(f.root.path().join("calls")).unwrap();
        let old_output = before.orchestration_rounds[0].output.clone();
        let old_executor = before.orchestration_rounds[0].executor_session.clone();
        let old_turn = p
            .previous_turn
            .as_ref()
            .map(|id| before.session_ledger.turns[id].clone());
        assert!(
            f.runtime
                .dispatch(
                    &id(),
                    Command::RecoverOrchestration {
                        run_id: before.id.clone(),
                        round_id: rid.clone(),
                        kind,
                        scope_hash: "stale".into()
                    }
                )
                .is_err()
        );
        std::fs::write(f.root.path().join("management-ready"), "ready").unwrap();
        let restarted =
            Arc::new(Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap());
        let key = id();
        let command = Command::RecoverOrchestration {
            run_id: before.id.clone(),
            round_id: rid.clone(),
            kind,
            scope_hash: p.scope_hash.clone(),
        };
        restarted.dispatch(&key, command.clone()).unwrap();
        let after = management_idle(&f);
        assert_eq!(after.orchestration_rounds.len(), 1);
        let round = &after.orchestration_rounds[0];
        assert_eq!(
            round.status,
            RoundStatus::Reviewed,
            "{mode}/{kind:?}: {:?}",
            after.diagnostic
        );
        assert_eq!(
            round.request_context,
            before.orchestration_rounds[0].request_context
        );
        planning::authenticate(&after, round).unwrap();
        if expected_role == RoleKind::Verifier {
            assert_eq!(round.output, old_output);
            assert_eq!(round.executor_session, old_executor);
        }
        if let Some(turn) = old_turn {
            assert_eq!(after.session_ledger.turns[&turn.id], turn);
        }
        let new_calls = std::fs::read_to_string(f.root.path().join("calls")).unwrap();
        assert_eq!(
            new_calls.lines().count(),
            calls.lines().count()
                + if expected_role == RoleKind::Verifier {
                    1
                } else {
                    2
                }
        );
        let first: serde_json::Value =
            serde_json::from_str(new_calls.lines().nth(calls.lines().count()).unwrap()).unwrap();
        let args = first["argv"].as_array().unwrap();
        assert!(args.iter().any(|a| a
            == if kind == Kind::Resume {
                "--resume"
            } else {
                "--session-id"
            }));
        if kind == Kind::Resume {
            assert_eq!(first["native"], p.native_session_id.unwrap());
        } else {
            assert_ne!(
                Some(first["native"].as_str().unwrap()),
                p.native_session_id.as_deref()
            );
        }
        assert!(
            first["prompt"]
                .as_str()
                .unwrap()
                .contains("Immutable orchestration recovery")
        );
        assert!(after.team.is_none() && after.artifacts.is_empty());
        restarted.dispatch(&key, command).unwrap();
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("calls")).unwrap(),
            new_calls
        );
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("workspace/source")).unwrap(),
            "original"
        );
    }
}

#[test]
fn management_recovery_restores_completed_review_without_native_replay() {
    use architect::runtime::orchestration_recovery::{self, Kind};
    let f = Fixture::new("answer");
    let original = f.execute();
    assert_eq!(
        original.orchestration_rounds[0].status,
        RoundStatus::Reviewed
    );
    let mut lost = original.clone();
    lost.orchestration_rounds[0].status = RoundStatus::Interrupted;
    lost.orchestration_rounds[0].review = None;
    lost.orchestration_rounds[0].verifier_session = None;
    lost.status = Status::Blocked;
    f.runtime
        .store
        .lock()
        .unwrap()
        .save(&lost, "test_management_review_checkpoint_lost", None)
        .unwrap();
    let p =
        orchestration_recovery::for_round(&lost, &lost.orchestration_rounds[0].id, Kind::Restore)
            .unwrap();
    let calls = std::fs::read_to_string(f.root.path().join("calls")).unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::RecoverOrchestration {
                run_id: lost.id.clone(),
                round_id: p.round_id,
                kind: Kind::Restore,
                scope_hash: p.scope_hash,
            },
        )
        .unwrap();
    let after = management_idle(&f);
    assert_eq!(
        after.orchestration_rounds[0].status,
        RoundStatus::Reviewed,
        "{:?}",
        after.diagnostic
    );
    assert_eq!(
        after.orchestration_rounds[0].review,
        original.orchestration_rounds[0].review
    );
    assert_eq!(after.session_ledger, original.session_ledger);
    planning::authenticate(&after, &after.orchestration_rounds[0]).unwrap();
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls")).unwrap(),
        calls
    );
}

#[test]
fn management_recovery_refuses_new_obligations_source_drift_and_failed_approval() {
    use architect::runtime::orchestration_recovery::{self, Kind};
    let f = Fixture::new("management-review-once");
    let before = reconciled_management(&f, &f.execute());
    let rid = before.orchestration_rounds[0].id.clone();
    let p = orchestration_recovery::for_round(&before, &rid, Kind::Replacement).unwrap();
    let mut changed = before.clone();
    changed.spec.objective = "changed objective".into();
    assert!(orchestration_recovery::pending(&changed).is_empty());
    let path = f.root.path().join("workspace/source");
    std::fs::write(&path, "changed input").unwrap();
    assert!(orchestration_recovery::pending(&before).is_empty());
    std::fs::write(&path, "original").unwrap();
    let calls = std::fs::read_to_string(f.root.path().join("calls")).unwrap();
    let db = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_management_recovery BEFORE INSERT ON events WHEN NEW.kind='orchestration_recovery_started' BEGIN SELECT RAISE(ABORT, 'injected management approval failure'); END;").unwrap();
    let key = id();
    let command = Command::RecoverOrchestration {
        run_id: before.id.clone(),
        round_id: rid.clone(),
        kind: Kind::Replacement,
        scope_hash: p.scope_hash,
    };
    assert!(f.runtime.dispatch(&key, command.clone()).is_err());
    assert_eq!(
        serde_json::to_value(f.current()).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls")).unwrap(),
        calls
    );
    db.execute_batch("DROP TRIGGER refuse_management_recovery")
        .unwrap();
    let queued = f
        .runtime
        .dispatch(
            &id(),
            Command::SendMessage {
                run_id: before.id.clone(),
                target_agent_id: "orchestrator".into(),
                text: "A new obligation supersedes the old management context".into(),
            },
        )
        .unwrap();
    assert!(orchestration_recovery::pending(&queued).is_empty());
    assert!(f.runtime.dispatch(&key, command).is_err());
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls")).unwrap(),
        calls
    );
}

#[test]
fn management_recovery_keeps_intake_sources_and_actual_native_review_binding() {
    use architect::runtime::orchestration_recovery::{self, Kind};
    let f = Fixture::new("intake-recover-review");
    let failed = reviewed_interview(&f);
    assert_eq!(failed.status, Status::Blocked);
    let before = reconciled_management(&f, &failed);
    let rid = before.orchestration_rounds[0].id.clone();
    let p = orchestration_recovery::for_round(&before, &rid, Kind::Replacement).unwrap();
    assert_eq!(p.role, RoleKind::Verifier);
    let source = f.root.path().join("workspace/source");
    std::fs::write(&source, "changed proposed source").unwrap();
    assert!(orchestration_recovery::pending(&before).is_empty());
    std::fs::write(&source, "original").unwrap();
    let calls = std::fs::read_to_string(f.root.path().join("calls")).unwrap();
    std::fs::write(f.root.path().join("management-ready"), "ready").unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::RecoverOrchestration {
                run_id: before.id.clone(),
                round_id: rid,
                kind: Kind::Replacement,
                scope_hash: p.scope_hash,
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let after = loop {
        let current = f.runtime.store.lock().unwrap().get(&before.id).unwrap();
        if current.status != Status::Running {
            break current;
        }
        assert!(Instant::now() < deadline, "{:?}", current.diagnostic);
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(
        after.orchestration_rounds[0].status,
        RoundStatus::Reviewed,
        "{:?}",
        after.diagnostic
    );
    assert_eq!(
        after.orchestration_rounds[0].proposed_sources,
        before.orchestration_rounds[0].proposed_sources
    );
    assert_eq!(
        after.orchestration_rounds[0].intake_evidence_id,
        before.orchestration_rounds[0].intake_evidence_id
    );
    planning::authenticate(&after, &after.orchestration_rounds[0]).unwrap();
    assert!(architect::runtime::intake::pending(&after).is_some());
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls"))
            .unwrap()
            .lines()
            .count(),
        calls.lines().count() + 1
    );
    assert!(after.intake && after.team.is_none() && after.artifacts.is_empty());
}

#[test]
fn management_recovery_is_exposed_in_daemon_tui_cli_and_survives_restart() {
    use architect::{
        ipc,
        runtime::orchestration_recovery::{self, Kind},
        tui::backend::{Backend, IpcBackend},
    };
    let f = Fixture::new("management-review-once");
    let before = reconciled_management(&f, &f.execute());
    let rid = before.orchestration_rounds[0].id.clone();
    let p = orchestration_recovery::for_round(&before, &rid, Kind::Replacement).unwrap();
    let state = f.root.path();
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_antislop"));
    struct Daemon(std::process::Child);
    impl Drop for Daemon {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut daemon = Daemon(
        architect::tui::connect_or_start_daemon(state, executable)
            .unwrap()
            .unwrap(),
    );
    let mut backend = IpcBackend::new(state);
    let snapshot = backend.snapshot(&before.id).unwrap();
    assert!(
        snapshot
            .orchestration_recoveries
            .values()
            .any(|v| v.scope_hash == p.scope_hash && v.kind == Kind::Replacement)
    );
    assert!(snapshot.orchestration_recovery_details[&p.scope_hash].contains(&p.handoff_hash));
    let calls = std::fs::read_to_string(f.root.path().join("calls")).unwrap();
    let refused = std::process::Command::new(executable)
        .arg("replace-orchestration")
        .arg(state)
        .arg(&before.id)
        .arg(&rid)
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls")).unwrap(),
        calls
    );
    std::fs::write(f.root.path().join("management-ready"), "ready").unwrap();
    let accepted = std::process::Command::new(executable)
        .arg("replace-orchestration")
        .arg(state)
        .arg(&before.id)
        .arg(&rid)
        .arg("--approve-scoped")
        .output()
        .unwrap();
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let after = management_idle(&f);
    assert_eq!(
        after.orchestration_rounds[0].status,
        RoundStatus::Reviewed,
        "{:?}",
        after.diagnostic
    );
    let current_calls = std::fs::read_to_string(f.root.path().join("calls")).unwrap();
    assert_eq!(current_calls.lines().count(), calls.lines().count() + 1);
    daemon.0.kill().unwrap();
    daemon.0.wait().unwrap();
    daemon.0 = architect::tui::connect_or_start_daemon(state, executable)
        .unwrap()
        .unwrap();
    let refreshed = backend.snapshot(&after.id).unwrap();
    assert!(refreshed.orchestration_recoveries.is_empty());
    let saved: Run = architect::transport::decode(
        ipc::read_document(state, &after.id, "run")
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    planning::authenticate(&saved, &saved.orchestration_rounds[0]).unwrap();
    assert_eq!(saved.orchestration_rounds[0].recoveries.len(), 1);
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls")).unwrap(),
        current_calls
    );
}

#[test]
fn intake_cannot_remove_the_users_role_provider_limits() {
    let allowed = Fixture::new("intake-role-retained");
    let reviewed = reviewed_interview(&allowed);
    assert_eq!(
        reviewed.orchestration_rounds[0].status,
        RoundStatus::Reviewed
    );
    let response = planning::authenticate(&reviewed, &reviewed.orchestration_rounds[0]).unwrap();
    let proposed = architect::runtime::intake::proposed_spec(
        &reviewed,
        response.intake_proposal_json.as_ref().unwrap(),
    )
    .unwrap();
    assert_eq!(
        proposed.config.as_ref().unwrap().config.role_providers,
        reviewed.spec.config.as_ref().unwrap().config.role_providers
    );
    let removed = Fixture::new("intake-role-removed");
    let rejected = reviewed_interview(&removed);
    let round = &rejected.orchestration_rounds[0];
    assert_eq!(round.status, RoundStatus::Rejected);
    assert!(
        round
            .diagnostic
            .as_ref()
            .unwrap()
            .contains("role provider limits")
    );
    assert!(round.verifier_session.is_none());
    assert!(architect::runtime::intake::pending(&rejected).is_none());
}
