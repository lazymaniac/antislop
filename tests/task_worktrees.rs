//! Task-worktree mode end to end: writers change private worktrees, verifiers
//! review the exact diff, verified results land in the project as commits and
//! no other task is invalidated by a write.
use architect::{
    config::*,
    domain::*,
    id,
    orchestration::planning,
    runtime::{Command, Runtime},
    storage::Store,
    team::TaskState,
};
use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command as Process,
    sync::Arc,
    time::{Duration, Instant},
};

struct Fixture {
    root: tempfile::TempDir,
    runtime: Arc<Runtime>,
    run: Run,
}

const SCRIPT: &str = r#"#!/usr/bin/env python3
import sys, json, pathlib, hashlib

base = pathlib.Path(__BASE__)
mode = __MODE__

if '--version' in sys.argv:
    print('2.1.296 (Claude Code)')
    sys.exit(0)
if '--help' in sys.argv:
    print('--safe-mode --strict-mcp-config --tools --permission-prompts --session-id --resume --json-schema --disable-slash-commands')
    sys.exit(0)

prompt = sys.stdin.read()
native = sys.argv[sys.argv.index('--resume' if '--resume' in sys.argv else '--session-id') + 1]
with (base / 'calls.jsonl').open('a') as f:
    f.write(json.dumps(dict(native=native, prompt=prompt)) + '\n')

def source_hashes(c):
    s = c['sources']
    return s['source_hashes'] if 'source_hashes' in s else {k: hashlib.sha256(v.encode()).hexdigest() for k, v in s.items()}

if prompt.startswith('You are the configured orchestrator.'):
    c = json.JSONDecoder().raw_decode(prompt.split('Immutable orchestration context: ', 1)[1])[0]
    team = c['team_template']
    roles = {a['id']: a['role'] for a in team['agents']}
    for task in team['tasks']:
        if task['write_scopes'] or roles[task['owner_id']] in ('implementer','refactorer','test_implementer','integrator'):
            task['change_contract'] = dict(kind='feature', rationale='Synthetic fixture requested new output; semantic classification is unproven', source_refs=task['source_dependencies'], bugfixes=[])
    out = dict(message='Fixture proposal for team execution', team_plan_json=json.dumps(team),
        risk_owners=[dict(risk_id=r['id'], task_id=team['tasks'][0]['id'], rationale='Synthetic risk coverage fixture') for r in c['risk_catalog']['risks']],
        agent_messages=[])
elif 'independently verifying task' in prompt:
    c = json.JSONDecoder().raw_decode(prompt.split('Immutable task context: ', 1)[1])[0]
    if c['task']['deliverable'] and c['change'] is not None and c['change']['paths']:
        # The verifier sees the exact diff of the writer's private result.
        (base / ('reviewed-diff-' + c['task']['id'])).write_text(c['change']['diff'])
    reject = any(i['artifact_hash'] == c['output_hash'] and i['state'] == 'completed' and not i['success'] for i in c.get('task_check_invocations', []))
    ids = [e['id'] for e in c['receipts'] if e['source'] == 'runtime:artifact-read' or e['source'].startswith('runner:')]
    hashes = source_hashes(c)
    quality = dict(family=c['spec']['task_family'], synthetic=False, spec_revision=c['spec_revision'],
        source_hashes=[dict(source_ref=k, source_hash=v) for k, v in hashes.items()],
        context=dict(source_refs=list(hashes), consumers=['fixture consumer'], contracts=['fixture contract'],
            configuration=['fixture config'], standards=['fixture standards'], baseline='fixture baseline', unresolved_material_unknowns=[]),
        planning=None, software=json.loads((base / 'software-template.json').read_text()), research=None, analysis=None, editorial=None, delivery=None)
    if c['task'].get('change_contract'):
        quality['change_review'] = dict(contract_hash=c['change_contract_hash'], kind=c['task']['change_contract']['kind'],
            classification_assessment='Synthetic independent fixture classifies requested behavior from original requirements and actual output', classification_evidence_ids=ids + [e['id'] for e in c['receipts'] if e['source'] == 'runtime:input-snapshot' and e.get('input_hash') == c['input_hash']], bugfixes=[])
    executor_output = json.loads(c['artifact']['content'])
    context = executor_output['context']
    solution = executor_output['solution']
    if context is not None or solution is not None:
        model_review = dict(context_hash=c['model_hashes']['context'], solution_hash=c['model_hashes']['solution'],
            inspected_context_sections=c['context_sections'] if context else [],
            independent_context=None, context_counterexample='Synthetic independent context counterexample' if context else None,
            independent_simulations=[])
        if context:
            source = next(k for k in hashes if k.startswith('file:'))
            model_review['independent_context'] = dict(facts=[dict(claim='Independent fixture source observation', source_ref=source, source_hash=hashes[source])], unknowns=[])
        if solution:
            model = solution['model']
            quality['planning'] = dict(option_ids=[o['id'] for o in model['options']], comparison_criteria=model['criteria'],
                rejected_options=[dict(option_id=o['id'], reason='Synthetic scoped rejection') for o in model['options'] if o['id'] != model['chosen_option']],
                chosen_option=model['chosen_option'], sensitivity='Synthetic selection sensitivity', invariants=[i['id'] for i in model['invariants']],
                scenario_kinds=sorted(set(s['kind'] for s in model['simulations'])), requirement_ids=c['task']['requirement_ids'],
                verification_methods=['Synthetic independent simulation protocol'])
            for kind in c['scenario_kinds']:
                scenario = dict(kind=kind, initial_state='Synthetic verifier counterexample initial ' + kind,
                    events=['Independent verifier alternative event ' + kind], expected_result='Independent verifier expected ' + kind,
                    invariant_ids=[i['id'] for i in model['invariants']])
                model_review['independent_simulations'].append(dict(scenario_id='verifier-' + kind, model_revision=c['spec_revision'],
                    criterion_ids=c['task']['criterion_ids'], scenario=scenario, outcome='pass', rationale='Synthetic independent model assessment'))
        quality['model_review'] = model_review
    out = dict(accepted=not reject, checklist_version=c['checklist_version'], output_hash=c['output_hash'], input_hash=c['input_hash'],
        quality=quality,
        checks=[dict(check_id=i, status='fail' if reject else 'pass', method='Synthetic protocol fixture',
            evidence_ids=ids, rationale='Synthetic protocol fixture, not substantive model judgment') for i in c['expected_checks']],
        criteria={i: not reject for i in c['task']['criterion_ids']},
        findings=['Observed required task check defect'] if reject else [], agent_messages=[])
    if quality.get('change_review'):
        for check in out['checks']:
            if check['check_id'] == 'CHANGE:classification': check['evidence_ids'] = quality['change_review']['classification_evidence_ids']
elif 'Task ID:' in prompt:
    c = json.JSONDecoder().raw_decode(prompt.split('Immutable task context: ', 1)[1])[0]
    hashes = source_hashes(c)
    context = None
    solution = None
    if c['task']['stage'] == 'discovery':
        sections = {}
        facts = []
        for section in c['context_sections']:
            source = 'objective' if section == 'intent' else next(k for k in hashes if k.startswith('file:'))
            fact = dict(claim='Synthetic fixture sourced ' + section, source_ref=source, source_hash=hashes[source])
            facts.append(fact)
            sections[section] = [fact]
        context = dict(model=dict(facts=facts, unknowns=[]), sections=sections)
    if c['task']['owner_id'] in ('executor-optionsanalyst', 'executor-planner'):
        model = dict(criteria=['Meet assigned requirements'], options=[
            dict(id='a', mechanism='Synthetic direct mechanism', requirement_ids=c['task']['requirement_ids'], evidence='Synthetic inspected option A'),
            dict(id='b', mechanism='Synthetic alternate mechanism', requirement_ids=c['task']['requirement_ids'], evidence='Synthetic inspected option B')],
            chosen_option='a', rationale='Synthetic scoped option selection', invariants=[dict(id='I1', statement='Preserve assigned behavior')], simulations=[])
        scenarios = []
        for kind in c['scenario_kinds']:
            scenario = dict(kind=kind, initial_state='Synthetic executor initial ' + kind,
                events=['Synthetic executor event ' + kind], expected_result='Synthetic executor expected ' + kind, invariant_ids=['I1'])
            model['simulations'].append(scenario)
            scenarios.append(dict(scenario_id='executor-' + kind, model_revision=c['spec_revision'], criterion_ids=c['task']['criterion_ids'],
                scenario=scenario, outcome='pass', rationale='Synthetic protocol fixture simulation'))
        solution = dict(model=model, scenarios=scenarios)
    out = dict(contract_version=c['task_model_contract'], artifact='Synthetic task execution artifact',
        edits=[], agent_messages=[], context=context, solution=solution)
    if c['task']['deliverable']:
        files = c['sources']['files']
        current = files['source.txt']['content']
        (base / ('executor-saw-' + c['task']['id'] + '-' + str(c['task'].get('attempt', 0)))).write_text(current)
        if mode == 'dirty':
            out['edits'] = [dict(path='source.txt', content=current.replace('original input', 'managed result'))]
        elif mode == 'conflict' and '<<<<<<<' in current:
            # Resolve the integration conflict: keep the project's line, add ours.
            out['edits'] = [dict(path='source.txt', content='user line\nmanaged result\n')]
        else:
            out['edits'] = [dict(path='source.txt', content='managed result\n')]
else:
    c = json.JSONDecoder().raw_decode(prompt[prompt.index('\n{') + 1:])[0]
    ids = [e['id'] for e in c.get('receipts', []) if e['source'] in ('runtime:orchestrator-inputs', 'runtime:orchestrator-output')] or ['synthetic-receipt']
    out = dict(contract_version=c['contract_version'], output_hash=c['output_hash'], spec_revision=c['spec_revision'], input_hash=c['input_hash'],
        checks=[dict(check_id=i, status='pass', method='Structured receipt fixture', evidence_ids=ids, rationale='Passing review fixture') for i in c['expected_checks']],
        findings=[], agent_messages=[])

print(json.dumps(dict(type='system', subtype='init', session_id=native, tools=['StructuredOutput'])))
print(json.dumps(dict(type='result', subtype='success', is_error=False, session_id=native, structured_output=out)))
"#;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Process::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

impl Fixture {
    fn new(mode: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(
            workspace.join("source.txt"),
            if mode == "dirty" {
                "top\n\n\n\noriginal input\n"
            } else {
                "original input\n"
            },
        )
        .unwrap();
        std::fs::write(workspace.join("other.txt"), "untouched\n").unwrap();
        git(&workspace, &["init", "--quiet"]);
        git(&workspace, &["add", "."]);
        git(&workspace, &["commit", "--quiet", "-m", "initial"]);

        let binary = root.path().join("mock-cli");
        let script = SCRIPT
            .replace(
                "__BASE__",
                &serde_json::to_string(root.path().to_str().unwrap()).unwrap(),
            )
            .replace("__MODE__", &serde_json::to_string(mode).unwrap());
        std::fs::write(&binary, script).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();

        let mut config = Config::from_toml(include_str!("../examples/architect.toml")).unwrap();
        config.workspace.mode = WorkspaceMode::TaskWorktrees;
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

        let mut spec: RunSpec =
            serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
        spec.workspace = workspace.to_string_lossy().into();
        spec.input_files = vec!["source.txt".into()];
        spec.editable_files = spec.input_files.clone();
        // Checks run in the task checkout: writers see their own result.
        spec.check_command = vec![
            "python3".into(),
            "-c".into(),
            "from pathlib import Path; assert 'managed result' in Path('source.txt').read_text()"
                .into(),
        ];
        spec.config = Some(config.pin().unwrap());
        let runtime =
            Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
        let run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();
        std::fs::write(
            root.path().join("software-template.json"),
            serde_json::to_vec(&architect::quality::synthetic_artifacts(&run).software).unwrap(),
        )
        .unwrap();
        Self { root, runtime, run }
    }
    fn workspace(&self) -> std::path::PathBuf {
        self.root.path().join("workspace")
    }
    fn current(&self) -> Run {
        self.runtime
            .store
            .lock()
            .unwrap()
            .get(&self.run.id)
            .unwrap()
    }
    fn wait_for_idle(&self) -> Run {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let current = self.current();
            if current.status != Status::Running {
                return current;
            }
            assert!(
                Instant::now() < deadline,
                "timed out: {:?}",
                current.diagnostic
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    fn install_team(&self) -> Run {
        let queued = self
            .runtime
            .dispatch(
                &id(),
                Command::SendMessage {
                    run_id: self.run.id.clone(),
                    target_agent_id: "orchestrator".into(),
                    text: "Propose team".into(),
                },
            )
            .unwrap();
        self.runtime
            .dispatch(
                &id(),
                Command::Orchestrate {
                    run_id: queued.id.clone(),
                    scope_hash: planning::start_scope(&queued).unwrap(),
                },
            )
            .unwrap();
        let reviewed = self.wait_for_idle();
        let round = &reviewed.orchestration_rounds[0];
        assert_eq!(
            round.status,
            planning::RoundStatus::Reviewed,
            "{:?} {}",
            reviewed.diagnostic,
            serde_json::to_string(round).unwrap()
        );
        self.runtime
            .dispatch(
                &id(),
                Command::AcceptTeam {
                    run_id: reviewed.id.clone(),
                    round_id: round.id.clone(),
                    scope_hash: planning::accept_scope(&reviewed, round).unwrap(),
                },
            )
            .unwrap()
    }
    fn step(&self, run: &Run) -> Run {
        self.runtime
            .dispatch(
                &id(),
                Command::StepTeam {
                    run_id: run.id.clone(),
                    scope_hash: run.team_step_scope().unwrap(),
                },
            )
            .unwrap();
        let mut run = self.wait_for_idle();
        let packages = architect::runtime::team::stage::pending(&run);
        for package in packages.values() {
            run = self
                .runtime
                .dispatch(
                    &id(),
                    Command::AcceptTeamStage {
                        run_id: run.id.clone(),
                        stage: package.stage,
                        task_ids: package.task_ids.clone(),
                        scope_hash: package.scope_hash.clone(),
                    },
                )
                .unwrap();
        }
        run
    }
}

fn writer(run: &Run) -> &architect::team::TaskRecord {
    run.team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.deliverable)
        .unwrap()
}

#[test]
fn completed_reader_before_binding_uses_its_original_view_without_new_git_effects() {
    let f = Fixture::new("success");
    let installed = f.install_team();
    let db = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_reader_binding BEFORE UPDATE ON runs WHEN EXISTS (SELECT 1 FROM json_each(NEW.state, '$.task_executions') WHERE json_extract(value, '$.task_id')='task-contextanalyst') BEGIN SELECT RAISE(ABORT,'injected reader binding failure'); END;").unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::StepTeam {
                run_id: installed.id.clone(),
                scope_hash: installed.team_step_scope().unwrap(),
            },
        )
        .unwrap();
    let stopped = f.wait_for_idle();
    assert_eq!(stopped.status, Status::Blocked, "{:?}", stopped.diagnostic);
    db.execute_batch("DROP TRIGGER refuse_reader_binding")
        .unwrap();
    let before = f
        .runtime
        .dispatch(
            &id(),
            Command::Reconcile {
                run_id: stopped.id.clone(),
                scope_hash: stopped.scope_hash("reconcile"),
                input_hash: architect::recovery::observe(&stopped).unwrap().hash,
            },
        )
        .unwrap();
    let task_id = "task-contextanalyst";
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    let view = before.task_views.as_ref().unwrap().tasks[task_id].clone();
    let head = git(&f.workspace(), &["rev-parse", "HEAD"]);
    let restored = f
        .runtime
        .dispatch(
            &id(),
            Command::RestoreTaskResult {
                run_id: before.id.clone(),
                task_id: task_id.into(),
                scope_hash: before.task_result_restore_scope(task_id).unwrap(),
            },
        )
        .unwrap();
    assert_eq!(
        restored
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .unwrap()
            .state,
        TaskState::AwaitingVerification
    );
    assert_eq!(
        serde_json::to_value(&before.task_views).unwrap(),
        serde_json::to_value(&restored.task_views).unwrap()
    );
    let execution = restored
        .task_executions
        .iter()
        .find(|e| e.task_id == task_id)
        .unwrap();
    assert!(execution.read_only_admission.is_some() && execution.receipts.is_empty());
    assert_eq!(
        execution.view.as_ref().unwrap().result.as_ref(),
        Some(&view.start)
    );
    assert!(execution.view.as_ref().unwrap().worktree.is_none());
    assert_eq!(git(&f.workspace(), &["rev-parse", "HEAD"]), head);
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    let mut changed = before.clone();
    changed
        .task_views
        .as_mut()
        .unwrap()
        .tasks
        .get_mut(task_id)
        .unwrap()
        .start = "0".repeat(40);
    assert!(changed.task_result_restore_scope(task_id).is_err());
}

#[test]
fn verified_writer_result_lands_as_one_commit_without_invalidating_other_tasks() {
    let f = Fixture::new("success");
    let mut run = f.install_team();
    let initial_head = git(&f.workspace(), &["rev-parse", "HEAD"]);
    let mut executed = false;
    for _ in 0..16 {
        run = f.step(&run);
        assert!(
            matches!(run.status, Status::Ready | Status::Completed),
            "{:?}: {:?}",
            run.status,
            run.diagnostic
        );
        let team = run.team.as_ref().unwrap();
        // A private write never makes another task stale.
        assert!(team.tasks.iter().all(|t| t.state != TaskState::Stale));
        let task = writer(&run);
        if task.state == TaskState::AwaitingVerification && !executed {
            executed = true;
            // Executed but unverified: the project is untouched; the result is private.
            assert_eq!(
                std::fs::read_to_string(f.workspace().join("source.txt")).unwrap(),
                "original input\n"
            );
            let views = run.task_views.as_ref().unwrap();
            let view = &views.tasks[&task.id];
            let worktree = Path::new(view.worktree.as_ref().unwrap());
            assert_eq!(
                std::fs::read_to_string(worktree.join("source.txt")).unwrap(),
                "managed result\n"
            );
            assert!(view.result.is_some() && view.integration.is_none());
            assert_eq!(run.input_snapshot["source.txt"], "original input\n");
            assert!(run.pending_edits.is_empty());
        }
        if task.state == TaskState::Verified {
            break;
        }
    }
    assert!(executed, "writer never executed");
    let task = writer(&run).clone();
    assert_eq!(task.state, TaskState::Verified, "{:?}", run.diagnostic);
    // The verifier reviewed the exact private diff.
    let reviewed =
        std::fs::read_to_string(f.root.path().join(format!("reviewed-diff-{}", task.id))).unwrap();
    assert!(reviewed.contains("-original input") && reviewed.contains("+managed result"));
    // The verified result landed in the project as exactly one commit.
    assert_eq!(
        std::fs::read_to_string(f.workspace().join("source.txt")).unwrap(),
        "managed result\n"
    );
    let integration = run.task_views.as_ref().unwrap().tasks[&task.id]
        .integration
        .clone()
        .unwrap();
    assert_eq!(integration.paths, vec!["source.txt".to_owned()]);
    let commit = integration.commit.unwrap();
    assert_eq!(git(&f.workspace(), &["rev-parse", "HEAD"]).trim(), commit);
    assert_eq!(git(&f.workspace(), &["rev-parse", "HEAD^"]), initial_head);
    assert_eq!(
        git(
            &f.workspace(),
            &["show", "--name-only", "--format=", "HEAD"]
        )
        .trim(),
        "source.txt"
    );
    assert!(
        git(&f.workspace(), &["log", "-1", "--format=%B"]).contains(&format!("Task: {}", task.id))
    );
    assert!(
        git(&f.workspace(), &["status", "--porcelain"])
            .trim()
            .is_empty()
    );
    // Pinned run inputs and every other verified result stay current.
    assert_eq!(run.input_snapshot["source.txt"], "original input\n");
    let team = run.team.as_ref().unwrap();
    assert!(team.tasks.iter().all(|t| t.state != TaskState::Stale));
    // Prompts carry bounded sources, never the run-wide snapshot evidence.
    let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    assert!(calls.contains("Pinned base input snapshot"));
}

#[test]
fn integration_conflict_returns_to_the_writer_and_never_overwrites_the_user() {
    let f = Fixture::new("conflict");
    let mut run = f.install_team();
    let mut conflicted = false;
    for _ in 0..20 {
        let task = writer(&run).clone();
        if task.state == TaskState::AwaitingVerification && !conflicted {
            // The user commits a competing change before the result is verified.
            std::fs::write(f.workspace().join("source.txt"), "user line\n").unwrap();
            git(&f.workspace(), &["commit", "--quiet", "-am", "user change"]);
        }
        run = f.step(&run);
        assert!(
            matches!(run.status, Status::Ready | Status::Completed),
            "{:?}: {:?}",
            run.status,
            run.diagnostic
        );
        let task = writer(&run);
        if let Some(conflict) = task
            .history
            .last()
            .and_then(|h| h.integration_conflict.as_ref())
            && !conflicted
        {
            conflicted = true;
            assert_eq!(conflict.paths, ["source.txt".to_owned()].into());
            // The user's committed change is untouched.
            assert_eq!(
                std::fs::read_to_string(f.workspace().join("source.txt")).unwrap(),
                "user line\n"
            );
            assert!(task.repair.is_some());
        }
        if conflicted && task.state == TaskState::Verified {
            break;
        }
    }
    assert!(conflicted, "no integration conflict was observed");
    let task = writer(&run);
    assert_eq!(task.state, TaskState::Verified, "{:?}", run.diagnostic);
    // The repair attempt saw conflict markers and its resolution was reviewed and landed.
    let saw = std::fs::read_to_string(
        f.root
            .path()
            .join(format!("executor-saw-{}-{}", task.id, task.attempt)),
    )
    .unwrap();
    assert!(saw.contains("<<<<<<< project") && saw.contains(">>>>>>> task"));
    assert_eq!(
        std::fs::read_to_string(f.workspace().join("source.txt")).unwrap(),
        "user line\nmanaged result\n"
    );
    assert_eq!(
        git(&f.workspace(), &["log", "-2", "--format=%s"])
            .lines()
            .nth(1),
        Some("user change")
    );
}

#[test]
fn local_uncommitted_edits_are_merged_but_never_committed() {
    let f = Fixture::new("dirty");
    let mut run = f.install_team();
    let head = git(&f.workspace(), &["rev-parse", "HEAD"]);
    let mut edited = false;
    for _ in 0..16 {
        if writer(&run).state == TaskState::AwaitingVerification && !edited {
            edited = true;
            std::fs::write(
                f.workspace().join("source.txt"),
                "top (local)\n\n\n\noriginal input\n",
            )
            .unwrap();
        }
        run = f.step(&run);
        assert!(
            matches!(run.status, Status::Ready | Status::Completed),
            "{:?}: {:?}",
            run.status,
            run.diagnostic
        );
        if writer(&run).state == TaskState::Verified {
            break;
        }
    }
    assert!(edited);
    let task = writer(&run);
    assert_eq!(task.state, TaskState::Verified, "{:?}", run.diagnostic);
    let integration = run.task_views.as_ref().unwrap().tasks[&task.id]
        .integration
        .clone()
        .unwrap();
    // Both the local edit and the verified change are kept; nothing is committed.
    assert_eq!(
        std::fs::read_to_string(f.workspace().join("source.txt")).unwrap(),
        "top (local)\n\n\n\nmanaged result\n"
    );
    assert_eq!(integration.commit, None);
    assert_eq!(integration.uncommitted, vec!["source.txt".to_owned()]);
    assert_eq!(git(&f.workspace(), &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git(&f.workspace(), &["status", "--porcelain"]).trim(),
        "M source.txt"
    );
}
