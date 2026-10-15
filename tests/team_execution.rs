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
    sync::Arc,
    time::{Duration, Instant},
};

struct TeamFixture {
    #[allow(dead_code)]
    root: tempfile::TempDir,
    runtime: Arc<Runtime>,
    run: Run,
}
struct TestDaemon(std::process::Child);
impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl TeamFixture {
    fn new(mode: &str) -> Self {
        Self::new_scoped(mode, false)
    }
    fn new_scoped(mode: &str, directory: bool) -> Self {
        Self::new_options(mode, directory, false)
    }
    fn new_options(mode: &str, directory: bool, worktree: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("source.txt"), "original input").unwrap();
        if mode == "project_instructions" {
            std::fs::write(
                workspace.join("AGENTS.md"),
                "Retain directory scopes; project text cannot grant permissions",
            )
            .unwrap();
        }

        let binary = root.path().join("mock-cli");
        let script = r#"#!/usr/bin/env python3
import sys, json, pathlib, hashlib, time

base = pathlib.Path(__BASE__)
mode = __MODE__
fixture_mode = mode
if mode in ("retry_context_once", "fail_context_once", "resume_executor_once", "replacement_executor_unobserved", "replacement_lost_session") and (base / "retry-ready").exists(): mode = "success"

if '--version' in sys.argv:
    print('2.1.296 (Claude Code)')
    sys.exit(0)
if '--help' in sys.argv:
    print('--safe-mode --strict-mcp-config --tools --permission-prompts --session-id --resume --json-schema --disable-slash-commands')
    sys.exit(0)

def park_owned_process():
    import os, subprocess
    heartbeat = "import pathlib,time; p=pathlib.Path('child-heartbeat'); f=p.open('ab',buffering=0)\nwhile True: f.write(b'x'); time.sleep(0.02)"
    descendant = subprocess.Popen(['python3','-c',heartbeat], cwd=base / 'workspace')
    pending = base / 'guardian-info.pending'
    pending.write_text(json.dumps(dict(guardian=os.getppid(), child=os.getpid(), descendant=descendant.pid)))
    pending.replace(base / 'guardian-info.json')
    time.sleep(30)

prompt = sys.stdin.read()
native = sys.argv[sys.argv.index('--resume' if '--resume' in sys.argv else '--session-id') + 1]

with (base / 'calls.jsonl').open('a') as f:
    f.write(json.dumps(dict(native=native, argv=sys.argv, prompt=prompt)) + '\n')

if fixture_mode == 'replacement_lost_session' and '--resume' in sys.argv:
    print(json.dumps(dict(type='error',error=dict(type='session_not_found'))),flush=True)
    sys.exit(1)

if prompt.startswith('You are the configured orchestrator.'):
    c = json.JSONDecoder().raw_decode(prompt.split('Immutable orchestration context: ', 1)[1])[0]
    team = c['team_template']
    if mode == 'mode_history':
        task = next(t for t in team['tasks'] if t['id'] == 'task-integrator')
        task['check_plan'] = [dict(id='integration-observation',argv=['python3','-c',"from pathlib import Path; assert Path('source.txt').read_text() == 'original input'"],phase='before_verification',expected_exit_code=0,
            criterion_ids=task['criterion_ids'],gate_ids=['GATE:integration_verified:delivery'],rationale='Actual fixture source assertion; independent review must assess real integration coverage, not infer it from exit zero')]
    if mode.startswith('restore_single'):
        for task in team['tasks']:
            if task['stage'] == 'discovery' and task['id'] != 'task-contextanalyst':
                task['dependencies'].append('task-contextanalyst')
    if mode == 'restore_single_baseline':
        task = next(t for t in team['tasks'] if t['id'] == 'task-contextanalyst')
        code = "from pathlib import Path; p=Path('baseline-count'); p.write_text(str(int(p.read_text())+1) if p.exists() else '1'); print('actual original observation')"
        task['check_plan'] = [dict(id='original-observation', argv=['python3','-c',code], phase='before_execution', expected_exit_code=0,
            criterion_ids=task['criterion_ids'], gate_ids=[], rationale='Synthetic proposal pins an observed original baseline')]
    if mode == 'managed_delivery_files':
        for task in team['tasks']:
            if task['deliverable']: task['write_scopes'] = ['source.txt', 'new', 'obsolete.txt']
    if mode == 'managed_other_input':
        for task in team['tasks']:
            if task['deliverable']: task['write_scopes'] = ['source.txt']
    roles = {a['id']: a['role'] for a in team['agents']}
    for task in team['tasks']:
        if c['spec']['task_family'] == 'software' and (task['write_scopes'] or roles[task['owner_id']] in ('implementer','refactorer','test_implementer','integrator')):
            task['change_contract'] = dict(kind='feature', rationale='Synthetic fixture requested new output; semantic classification is unproven', source_refs=task['source_dependencies'], bugfixes=[])
    if mode in ('managed_disjoint', 'managed_disjoint_baseline'):
        writers = [t for t in team['tasks'] if t['deliverable']]
        for task, path in zip(writers, ['source.txt', 'second.txt']): task['write_scopes'] = [path]
    if mode.startswith('task_check_'):
        task = next(t for t in team['tasks'] if t['id'] == 'task-contextanalyst')
        code = "from pathlib import Path; p=Path('runner-count'); p.write_text(str(int(p.read_text())+1) if p.exists() else '1'); print('actual task check'); assert Path('source.txt').read_text() == 'original input'"
        if mode in ('task_check_failure', 'task_check_lie'): code += "; raise SystemExit(1)"
        if mode == 'task_check_mutation': code += "; Path('source.txt').write_text('check changed input')"
        if mode == 'task_check_lease':
            body = "import os,json,subprocess,time; from pathlib import Path; base=Path(" + repr(str(base)) + "); heartbeat=\"import pathlib,time; f=pathlib.Path('child-heartbeat').open('ab',buffering=0)\\nwhile True: f.write(b'x'); time.sleep(0.02)\"; descendant=subprocess.Popen(['python3','-c',heartbeat]); pending=base/'guardian-info.pending'; pending.write_text(json.dumps(dict(guardian=os.getppid(),child=os.getpid(),descendant=descendant.pid))); pending.replace(base/'guardian-info.json'); time.sleep(30)"
            code += "; exec(" + repr(body) + ")"
        if mode == 'task_check_hang': code += "; Path('check-started').write_text('started'); import time; time.sleep(30)"
        task['check_plan'] = [dict(id='context-check', argv=['python3','-c',code], criterion_ids=task['criterion_ids'], gate_ids=[],
            rationale='Synthetic proposal binds exact actual local context observation')]
    if mode.startswith('baseline_'):
        task = next(t for t in team['tasks'] if t['id'] == 'task-contextanalyst')
        code = "from pathlib import Path; p=Path('baseline-count'); p.write_text(str(int(p.read_text())+1) if p.exists() else '1'); print('actual baseline')"
        if mode == 'baseline_mutation': code += "; Path('source.txt').write_text('baseline changed input')"
        if mode == 'baseline_hang': code += "; Path('baseline-started').write_text('started'); import time; time.sleep(30)"
        if mode == 'baseline_signal': code += "; import os, signal; os.kill(os.getpid(), signal.SIGTERM)"
        code += "; raise SystemExit(" + ('0' if mode == 'baseline_unexpected_success' else '2' if mode == 'baseline_wrong_failure' else '1') + ")"
        task['check_plan'] = [dict(id='reproduction', argv=['python3','-c',code], phase='before_execution', expected_exit_code=1,
            criterion_ids=task['criterion_ids'], gate_ids=[], rationale='Synthetic baseline reproduction; expected failure is not positive product evidence')]
    if mode in ('managed_baseline', 'managed_disjoint_baseline'):
        for task in [t for t in team['tasks'] if t['deliverable']]:
            code = "from pathlib import Path; import sys; p=Path('baseline-" + task['id'] + "'); p.write_text(str(int(p.read_text())+1) if p.exists() else '1'); print('baseline before any writer'); sys.exit(1 if Path('source.txt').read_text() == 'original input'" + (" and Path('second.txt').read_text() == 'second original'" if mode == 'managed_disjoint_baseline' else '') + " else 2)"
            task['check_plan'].append(dict(id='reproduction', argv=['python3','-c',code], phase='before_execution', expected_exit_code=1,
                criterion_ids=task['criterion_ids'], gate_ids=[], rationale='Observe all original writer inputs before any native executor'))
    if mode.startswith('managed_bugfix_'):
        for task in [t for t in team['tasks'] if t['deliverable']]:
            alternative = mode.startswith('managed_bugfix_alternative')
            measurement = mode == 'managed_bugfix_measurement'
            code = "from pathlib import Path; p=Path('bugfix-before-count'); p.write_text(str(int(p.read_text())+1) if p.exists() else '1'); print('inspect actual original bytes'); assert Path('source.txt').read_text() == " + ("'original input'" if alternative else "'managed result'")
            if alternative: code += "; assert not Path('original-entrypoint').exists(); print('Original entrypoint is unavailable: file does not exist; retained source value is ' + Path('source.txt').read_text())"
            if measurement: code = "from pathlib import Path; p=Path('bugfix-before-count'); p.write_text('1'); print('Observed original value: ' + Path('source.txt').read_text())"
            task['check_plan'].append(dict(id='bug-original', argv=['python3','-c',code], phase='before_execution', expected_exit_code=0 if alternative or measurement else 1,
                criterion_ids=task['criterion_ids'], gate_ids=[], rationale='Synthetic original defect/investigation fixture; actual file assertion'))
            before = dict(kind='alternative', investigation_check_id='bug-original', unavailable_reason='Fixture models an unavailable original entrypoint; retained source bytes permit alternate proof',
                method='Inspect retained original source value against specified desired value and then assert the corrected current value', source_refs=task['source_dependencies']) if alternative else dict(kind='reproduction', check_id='bug-original')
            task['change_contract'] = dict(kind='bugfix', rationale='Synthetic requested correction of the original source value', source_refs=task['source_dependencies'],
                bugfixes=[dict(id='wrong-value', defect='Original source value violates the specified desired value', criterion_ids=task['criterion_ids'],
                    oracle="source.txt must contain managed result; inspect actual assertion against original and modified input", before=before, after_check_id='configured-check')])
            if mode == 'managed_bugfix_missing_before': task['check_plan'] = [p for p in task['check_plan'] if p['id'] != 'bug-original']
            if mode == 'managed_bugfix_bad_after': task['change_contract']['bugfixes'][0]['after_check_id'] = 'bug-original'
            if mode == 'managed_bugfix_incomplete_criteria': task['change_contract']['bugfixes'][0]['criterion_ids'] = []
            if mode == 'managed_bugfix_alternative_no_reason': before['unavailable_reason'] = ''
            if mode.startswith('managed_bugfix_nondefect'):
                process = next(c for c in task['criterion_ids'] if c.endswith(':approved_workflow'))
                defects = [c for c in task['criterion_ids'] if c != process]
                task['change_contract']['bugfixes'][0]['criterion_ids'] = defects
                task['check_plan'][-1]['criterion_ids'] = defects
                task['change_contract']['non_defect_criteria'] = {process:'An approval/workflow constraint remains required but is not the original value defect'}
    if mode == 'missing_change_classification':
        next(t for t in team['tasks'] if t['deliverable'])['change_contract'] = None
    if mode == 'task_check_missing_plan':
        next(t for t in team['tasks'] if t['deliverable'])['check_plan'] = []
    out = dict(
        message='Fixture proposal for team execution',
        team_plan_json=json.dumps(team),
        risk_owners=[
            dict(risk_id=r['id'], task_id=team['tasks'][0]['id'], rationale='Synthetic risk coverage fixture')
            for r in c['risk_catalog']['risks']
        ],
        agent_messages=[]
    )
    comments = [v for v in c.get('element_comments', []) if not v['resolved'] and not v['deleted']]
    if comments:
        out.update(message='Reviewed response to graph element comments', team_plan_json=None, risk_owners=[],
            comment_reactions=[dict(comment_id=v['id'], kind='answer', text='Fixture inspected the annotated element and answered this exact comment', task_id=None) for v in comments])
        if mode == 'omit_comment_reaction': out['comment_reactions'] = []
        if mode == 'invent_comment_task': out['comment_reactions'][0].update(kind='proposed_task',task_id='invented-task')
elif 'independently verifying task' in prompt:
    c = json.JSONDecoder().raw_decode(prompt.split('Immutable task context: ', 1)[1])[0]
    if mode == 'lease_verifier' and c['task']['id'] == 'task-contextanalyst': park_owned_process()
    if mode in ('fail_context_once','replacement_lost_session') and c['task']['id'] == 'task-contextanalyst':
        print(json.dumps(dict(type='system', subtype='init', session_id=native, tools=['StructuredOutput'])), flush=True)
        sys.exit(1)
    reject = (mode == 'reject' and c['task']['id'] == 'task-contextanalyst') or any(i['artifact_hash'] == c['output_hash'] and i['state'] == 'completed' and not i['success'] for i in c.get('task_check_invocations', []))
    if mode == 'task_check_lie': reject = False
    ids = [e['id'] for e in c['receipts'] if e['source'] == 'runtime:artifact-read' or e['source'].startswith('runner:')]
    if mode == 'baseline_only_evidence': ids = [e['id'] for e in c['receipts'] if e['source'].startswith('runner:baseline:')] or ids
    quality = dict(family=c['spec']['task_family'], synthetic=False, spec_revision=c['spec_revision'],
        source_hashes=[dict(source_ref=k, source_hash=hashlib.sha256(v.encode()).hexdigest()) for k, v in c['sources'].items()],
        context=dict(source_refs=list(c['sources']), consumers=['fixture consumer'], contracts=['fixture contract'],
            configuration=['fixture config'], standards=['fixture standards'], baseline='fixture baseline', unresolved_material_unknowns=[]),
        planning=None, software=None, research=None, analysis=None, editorial=None, delivery=None)
    if mode.startswith('managed_'): quality['software'] = json.loads((base / 'software-template.json').read_text())
    if mode.startswith('managed_delivery'):
        quality['delivery']=dict(requirement_ids=[r['id'] for r in c['spec']['requirements']],instructions=['Fixture result is inspected through the runtime'],demonstrated_results=['Actual local fixture command asserts managed source content'],limitations=['Protocol and filesystem test only; no substantive product-quality claim'],unsupported_claims=[])
        sw=quality['software']
        runner=[e['id'] for e in c['receipts'] if e['source'].startswith('runner:') and e.get('input_hash')==c['input_hash']]
        analyzer=sw['contract']['analyzers'][0]
        sw['static_analysis']=dict(analyses=[dict(analyzer_id=analyzer['id'],version=analyzer['version'],command=analyzer['command'],targets=analyzer['targets'],configurations=analyzer['configurations'],rules=analyzer['rules'],raw_evidence_ids=runner,diagnostics=[])],suppressions=[],unresolved_findings=[])
        sw['behavior']=dict(requirement_scenarios=[dict(criterion_id='REQ:'+r['id']+':'+criterion,scenario_ids=['fixture-current']) for r in c['spec']['requirements'] for criterion in r['criteria']],invariant_scenarios=[dict(criterion_id='I1',scenario_ids=['fixture-current'])],oracle='Actual fixture command asserts the managed source value',scenario_evidence=[dict(scenario_id='fixture-current',evidence_ids=runner)],mutation_or_counterexample='Protocol fixture only; no substantive mutation-test claim',unchanged_test_contracts='Original fixture assertion retained',unresolved_findings=[])
        sw['refactor']['behavior_evidence_ids']=runner
    if c['task'].get('change_contract'):
        quality['change_review'] = dict(contract_hash=c['change_contract_hash'], kind=c['task']['change_contract']['kind'],
            classification_assessment='Synthetic independent fixture classifies requested behavior from original requirements and actual output', classification_evidence_ids=ids + [e['id'] for e in c['receipts'] if e['source'] == 'runtime:input-snapshot' and e.get('input_hash') == c['input_hash']], bugfixes=[])
    if mode.startswith('managed_bugfix_') and c['task']['deliverable']:
        contract = c['task']['change_contract']
        quality['change_review']['non_defect_assessments'] = {key:dict(assessment='Independent synthetic fixture distinguishes the original value defect from the separately mandatory process criterion',evidence_ids=ids) for key in contract.get('non_defect_criteria',{})}
        if mode == 'managed_bugfix_nondefect_omit': quality['change_review']['non_defect_assessments'] = {}
        if mode == 'managed_bugfix_nondefect_unbound':
            for assessment in quality['change_review']['non_defect_assessments'].values(): assessment['evidence_ids'] = ['unobserved']
        for bug in contract['bugfixes']:
            before_id = bug['before'].get('check_id', bug['before'].get('investigation_check_id'))
            original = next(i for i in c['task_check_invocations'] if i['plan']['id'] == before_id)
            after = next(i for i in c['task_check_invocations'] if i['plan']['id'] == bug['after_check_id'] and i['artifact_hash'] == c['output_hash'])
            quality['change_review']['bugfixes'].append(dict(id=bug['id'], before_evidence_ids=[original['evidence_id']], after_evidence_ids=[after['evidence_id']],
                before_assessment='Independent fixture inspected original source and actual check output: the original value differs from the required managed result', independent_oracle='Independent fixture inspection of desired-value assertion against actual before/after bytes', counterexample='Returning the original value must still fail the post-change assertion',
                alternative_assessment='Independently inspect retained original bytes and the simulated unavailable-entrypoint limitation; actual corrected-value assertion supplies post-change proof' if bug['before']['kind'] == 'alternative' else None))
        if mode == 'managed_bugfix_no_review': quality['change_review'] = None
        if mode == 'managed_bugfix_wrong_contract_hash': quality['change_review']['contract_hash'] = '0' * 64
        if mode == 'managed_bugfix_omit_proof': quality['change_review']['bugfixes'] = []
        if mode == 'managed_bugfix_wrong_receipt': quality['change_review']['bugfixes'][0]['after_evidence_ids'] = quality['change_review']['bugfixes'][0]['before_evidence_ids']
        if mode == 'managed_bugfix_alternative_no_assessment': quality['change_review']['bugfixes'][0]['alternative_assessment'] = None
    executor_output = json.loads(c['artifact']['content'])
    context = executor_output['context']
    solution = executor_output['solution']
    if context is not None or solution is not None:
        model_review = dict(context_hash=c['model_hashes']['context'], solution_hash=c['model_hashes']['solution'],
            inspected_context_sections=c['context_sections'] if context else [],
            independent_context=None, context_counterexample='Synthetic independent context counterexample' if context else None,
            independent_simulations=[])
        if context:
            source = next(k for k in c['sources'] if k.startswith('file:'))
            model_review['independent_context'] = dict(facts=[dict(claim='Independent fixture source observation',
                source_ref=source, source_hash=hashlib.sha256(c['sources'][source].encode()).hexdigest())], unknowns=[])
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
                if mode == 'copied_simulations': scenario = next(s for s in model['simulations'] if s['kind'] == kind)
                model_review['independent_simulations'].append(dict(scenario_id='verifier-' + kind, model_revision=c['spec_revision'],
                    criterion_ids=c['task']['criterion_ids'], scenario=scenario, outcome='pass', rationale='Synthetic independent model assessment'))
        quality['model_review'] = model_review
        if mode == 'missing_context_review': quality['model_review'] = None
        if mode == 'wrong_context_model_hash' and context: quality['model_review']['context_hash'] = '0' * 64
        if mode == 'model_review_unknown_fact' and context: quality['model_review']['independent_context']['facts'][0]['source_hash'] = '0' * 64
        if mode == 'wrong_solution_hash' and solution: quality['model_review']['solution_hash'] = '0' * 64
        if mode == 'missing_planning_model_bind' and solution: quality['planning']['chosen_option'] = 'b'
        if mode == 'false_simulation' and solution: quality['model_review']['independent_simulations'][0]['outcome'] = 'fail'
    out = dict(
        accepted=not reject,
        checklist_version=c['checklist_version'], output_hash=c['output_hash'], input_hash=c['input_hash'],
        quality=quality,
        checks=[dict(check_id=i, status='fail' if reject else 'pass', method='Synthetic protocol fixture',
            evidence_ids=ids, rationale='Synthetic protocol fixture, not substantive model judgment') for i in c['expected_checks']],
        criteria={i: not reject for i in c['task']['criterion_ids']},
        findings=['Seeded fixture or observed required task check defect'] if reject else [], agent_messages=[])
    if quality.get('change_review'):
        for check in out['checks']:
            if check['check_id'] == 'CHANGE:classification': check['evidence_ids'] = quality['change_review']['classification_evidence_ids']
    if mode == 'missing_criteria' or (mode in ('task_check_retry', 'baseline_retry') and not (base / 'retry-ready').exists()): out['criteria'] = {}
    if mode == 'false_criterion': out['criteria'][next(iter(out['criteria']))] = False
    if mode == 'accepted_findings': out['findings'] = ['Unresolved defect must not be cleared']
    if mode == 'malformed_negative_findings': out = dict(agent_messages=[], findings='Reported defect in malformed field')
    if mode == 'missing_checks': out['checks'] = []
    if mode == 'missing_evidence': out['checks'][0]['evidence_ids'] = ['invented-evidence']
    if mode == 'wrong_hash': out['output_hash'] = '0' * 64
    if mode == 'synthetic_quality': out['quality']['synthetic'] = True
    if mode == 'empty_quality': out['quality']['context'] = None
    if mode == 'malformed_review': out = dict(agent_messages=[])
    if mode == 'retry_context_once' and c['task']['id'] == 'task-contextanalyst': out['checks'] = []
elif 'Task ID:' in prompt:
    c = json.JSONDecoder().raw_decode(prompt.split('Immutable task context: ', 1)[1])[0]
    if mode == 'replacement_executor_unobserved' and c['task']['id'] == 'task-contextanalyst': sys.exit(1)
    if mode == 'resume_executor_once' and c['task']['id'] == 'task-contextanalyst':
        print(json.dumps(dict(type='system', subtype='init', session_id=native, tools=['StructuredOutput'])), flush=True)
        sys.exit(1)
    if mode == 'lease_executor' and c['task']['id'] == 'task-contextanalyst': park_owned_process()
    if mode == 'checkpoint_stop':
        (base / ('started-' + c['task']['id'])).write_text('started')
        if c['task']['id'] == 'task-contextanalyst':
            until = time.monotonic() + 8
            while len(list(base.glob('started-*'))) < 3:
                if time.monotonic() > until: raise RuntimeError('fixture sibling turns did not start')
                time.sleep(0.01)
        else:
            time.sleep(30)
    if mode == 'slow_executor':
        (base / 'executor-started').write_text('started')
        time.sleep(0.5)
    if mode in ('managed_baseline', 'managed_disjoint_baseline') and c['task']['deliverable']:
        assert len(list((base / 'workspace').glob('baseline-task-*'))) == (2 if mode == 'managed_disjoint_baseline' else 1)
    context = None
    solution = None
    if c['task']['stage'] == 'discovery':
        sections = {}
        facts = []
        for section in c['context_sections']:
            source = 'objective' if section == 'intent' else next(k for k in c['sources'] if k.startswith('file:'))
            fact = dict(claim='Synthetic fixture sourced ' + section, source_ref=source,
                source_hash=hashlib.sha256(c['sources'][source].encode()).hexdigest())
            facts.append(fact)
            sections[section] = [fact]
        context = dict(model=dict(facts=facts, unknowns=[]), sections=sections)
        if mode == 'missing_context': context = None
        if mode == 'omitted_context_section': context['sections']['contracts'] = []
        if mode == 'stale_context_fact': context['model']['facts'][0]['source_hash'] = '0' * 64
        if mode == 'unresolved_context': context['model']['unknowns'] = [dict(question='Unresolved material rule', material=True, resolution=None, source_ref=None)]
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
        if mode == 'missing_solution': solution = None
        if mode == 'stale_model_revision': solution['scenarios'][0]['model_revision'] = 0
        if mode == 'missing_design_class': solution['scenarios'] = solution['scenarios'][:-1]
        if mode == 'duplicate_scenario_id': solution['scenarios'][1]['scenario_id'] = solution['scenarios'][0]['scenario_id']
        if mode == 'missing_model_criterion':
            for scenario in solution['scenarios']: scenario['criterion_ids'] = []
    out = dict(
        contract_version=c['task_model_contract'],
        artifact='Synthetic task execution artifact',
        edits=[], agent_messages=[], context=context, solution=solution)
    if mode.startswith('managed_') and c['task']['deliverable']:
        refreshing = bool(c['task']['history'] and c['task']['history'][-1].get('source_refresh'))
        if not refreshing or mode == 'managed_repeat_write': out['edits'] = [dict(path=c['task']['write_scopes'][0], content='managed result')]
        if mode == 'managed_delivery_files' and not refreshing:
            out['edits'] = [dict(path='source.txt',content='managed result',operation='write'), dict(path='new/sub.txt',content='created result',operation='create'), dict(path='obsolete.txt',content='',operation='delete')]
    if mode == 'escape_edit': out['edits'] = [dict(path='source.txt/../../escape.txt', content='unauthorized')]
    if mode == 'outside_scope': out['edits'] = [dict(path='source.txt', content='unauthorized')]
    if mode == 'empty_artifact': out['artifact'] = ''
else:
    # Orchestrator review turn
    c = json.JSONDecoder().raw_decode(prompt[prompt.index('\n{') + 1:])[0]
    ids = [e['id'] for e in c.get('receipts', []) if e['source'] in ('runtime:orchestrator-inputs', 'runtime:orchestrator-output')]
    if not ids:
        ids = ['synthetic-receipt']
    out = dict(
        contract_version=c['contract_version'],
        output_hash=c['output_hash'],
        spec_revision=c['spec_revision'],
        input_hash=c['input_hash'],
        checks=[
            dict(
                check_id=i,
                status='fail' if mode == 'reject_comment_review' and i.startswith('COMMENT:') else 'pass',
                method='Structured receipt fixture',
                evidence_ids=ids,
                rationale='Passing review fixture'
            )
            for i in c['expected_checks']
        ],
        findings=[],
        agent_messages=[]
    )

print(json.dumps(dict(type='system', subtype='init', session_id=native, tools=['StructuredOutput'])))
print(json.dumps(dict(type='result', subtype='success', is_error=False, session_id=native, structured_output=out)))
"#
        .replace("__BASE__", &serde_json::to_string(root.path().to_str().unwrap()).unwrap())
        .replace("__MODE__", &serde_json::to_string(mode).unwrap());

        std::fs::write(&binary, script).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();

        let mut config = Config::from_toml(include_str!("../examples/architect.toml")).unwrap();
        config.orchestrator.review_checks_per_part = 0;
        config.workspace.mode = if worktree {
            WorkspaceMode::Worktree
        } else {
            WorkspaceMode::InPlace
        };
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
        if mode.starts_with("managed_bugfix_nondefect") {
            spec.requirements[0].criteria.insert("approved_workflow".into(),"Retain the approved process and verify it separately from the original value defect".into());
        }
        spec.input_files = vec!["source.txt".into()];
        spec.check_command = vec![
            "python3".into(),
            "-c".into(),
            "print('Synthetic fixture command; substantive gate judgment is unproven')".into(),
        ];
        if mode == "managed_other_input" {
            std::fs::write(workspace.join("second.txt"), "second original").unwrap();
            spec.input_files.push("second.txt".into());
        }
        if matches!(mode, "managed_disjoint" | "managed_disjoint_baseline") {
            std::fs::write(workspace.join("second.txt"), "second original").unwrap();
            spec.input_files.push("second.txt".into());
            spec.plan_items.push(PlanItem {
                id: "PLAN-2".into(),
                dependencies: Default::default(),
                requirement_ids: spec.plan_items[0].requirement_ids.clone(),
                description: "Exercise disjoint managed writer".into(),
            });
        }
        if mode.starts_with("managed_") {
            spec.editable_files = spec.input_files.clone();
            spec.check_command = vec!["python3".into(), "-c".into(), "from pathlib import Path; assert Path('source.txt').read_text() == 'managed result'".into()];
        }
        if mode == "managed_runner_fail" {
            spec.check_command = vec!["python3".into(),"-c".into(),"from pathlib import Path; Path('check-invoked').write_text('observed'); raise SystemExit(1)".into()];
        }
        if directory {
            spec.input_files.push(".".into());
        }
        if mode == "managed_delivery_files" {
            std::fs::write(workspace.join("obsolete.txt"), "original obsolete").unwrap();
            spec.input_files = vec![".".into()];
            spec.editable_files = spec.input_files.clone();
            spec.check_command = vec!["python3".into(), "-c".into(), "from pathlib import Path; assert Path('source.txt').read_text() == 'managed result'; assert Path('new/sub.txt').read_text() == 'created result'; assert not Path('obsolete.txt').exists()".into()];
        }
        spec.config = Some(config.pin().unwrap());
        if mode.starts_with("mode_history") {
            spec.task_family = architect::quality::TaskFamily::Research;
            spec.check_command.clear();
        }

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

    fn current(&self) -> Run {
        self.runtime
            .store
            .lock()
            .unwrap()
            .get(&self.run.id)
            .unwrap()
    }

    fn install_team(&self) -> Run {
        // Queue user message to orchestrator
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

        // Run orchestration turn
        self.runtime
            .dispatch(
                &id(),
                Command::Orchestrate {
                    run_id: queued.id.clone(),
                    scope_hash: planning::start_scope(&queued).unwrap(),
                },
            )
            .unwrap();

        // Wait for orchestration round to finish
        let deadline = Instant::now() + Duration::from_secs(15);
        let reviewed = loop {
            let cur = self.current();
            if cur.status != Status::Running
                && architect::recovery::WorkspaceLease::acquire(&cur.spec.workspace).is_ok()
            {
                break cur;
            }
            assert!(
                Instant::now() < deadline,
                "orchestration timed out: {:?}",
                cur.diagnostic
            );
            std::thread::sleep(Duration::from_millis(50));
        };

        let round = &reviewed.orchestration_rounds[0];
        assert_eq!(
            round.status,
            planning::RoundStatus::Reviewed,
            "{:?}",
            round.diagnostic
        );

        // Accept team proposal
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

    fn wait_for_idle(&self) -> Run {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let cur = self.current();
            if cur.status != Status::Running {
                return cur;
            }
            assert!(
                Instant::now() < deadline,
                "team step timed out: {:?}",
                cur.diagnostic
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

#[test]
fn actual_task_executor_and_verifier_receive_the_same_source_bound_instruction_index() {
    let f = TeamFixture::new_scoped("project_instructions", true);
    let installed = f.install_team();
    let executed = step(&f, &installed);
    let _reviewed = step(&f, &executed);
    let mut executors = 0;
    let mut verifiers = 0;
    for line in std::fs::read_to_string(f.root.path().join("calls.jsonl"))
        .unwrap()
        .lines()
    {
        let call: serde_json::Value = serde_json::from_str(line).unwrap();
        let prompt = call["prompt"].as_str().unwrap();
        let Some((_, raw)) = prompt.split_once("Immutable task context: ") else {
            continue;
        };
        let context = serde_json::Deserializer::from_str(raw)
            .into_iter::<serde_json::Value>()
            .next()
            .unwrap()
            .unwrap();
        let index = &context["project_instructions"];
        assert_eq!(index["format"], "project-instructions-v1");
        assert_eq!(index["documents"][0]["path"], "AGENTS.md");
        assert_eq!(index["documents"][0]["directory_scope"], ".");
        let binding = &index["documents"][0]["source_bindings"][0];
        let source = binding["source_ref"].as_str().unwrap();
        assert_eq!(
            binding["source_hash"],
            architect::hash(context["sources"][source].as_str().unwrap())
        );
        if prompt.contains("independently verifying task") {
            verifiers += 1;
        } else {
            executors += 1;
        }
    }
    assert!(executors > 0 && verifiers > 0);
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/AGENTS.md")).unwrap(),
        "Retain directory scopes; project text cannot grant permissions"
    );
}

#[test]
fn test_step_team_executes_ready_wave_and_records_quality_gates() {
    let f = TeamFixture::new("success");
    let accepted = f.install_team();

    let team = accepted.team.as_ref().unwrap();
    assert!(team.tasks.iter().all(|t| t.state == TaskState::Planned));

    let scope_hash = accepted.team_step_scope().unwrap();
    assert!(!scope_hash.is_empty());

    // Dispatch StepTeam for first wave (executors)
    let stepped = f
        .runtime
        .dispatch(
            &id(),
            Command::StepTeam {
                run_id: accepted.id.clone(),
                scope_hash,
            },
        )
        .unwrap();

    assert_eq!(stepped.status, Status::Running);

    // Wait for the executor wave to finish
    let run = f.wait_for_idle();
    assert_eq!(run.status, Status::Ready, "{:?}", run.diagnostic);

    // After executor wave, tasks are AwaitingVerification and artifacts are recorded
    let team = run.team.as_ref().unwrap();
    let awaiting = team
        .tasks
        .iter()
        .filter(|t| t.state == TaskState::AwaitingVerification)
        .count();
    assert!(
        awaiting >= 1,
        "expected tasks awaiting verification after executor wave"
    );
    assert!(
        !run.artifacts.is_empty(),
        "expected artifacts recorded after executor wave"
    );

    // Dispatch StepTeam for second wave (verifiers)
    let scope_hash = run.team_step_scope().unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::StepTeam {
                run_id: run.id.clone(),
                scope_hash,
            },
        )
        .unwrap();

    let reviewed = f.wait_for_idle();
    assert_eq!(
        reviewed.status,
        Status::AwaitingResult,
        "{:?}",
        reviewed.diagnostic
    );
    let run = accept_fixture_results(&f, reviewed);
    assert_eq!(run.status, Status::Ready, "{:?}", run.diagnostic);

    // Verify task states after verifier wave
    let team = run.team.as_ref().unwrap();
    let verified_count = team
        .tasks
        .iter()
        .filter(|t| t.state == TaskState::Verified)
        .count();

    assert!(
        verified_count >= 1,
        "expected verified tasks after verifier wave, got 0"
    );

    // Verify reviews recorded
    assert!(!run.reviews.is_empty(), "expected reviews recorded");

    // Verify evidence recorded
    assert!(!run.evidence.is_empty(), "expected evidence recorded");

    // Verify quality gates updated for verified tasks
    assert!(
        !run.quality_gates.is_empty(),
        "expected quality gates updated"
    );
    assert!(
        run.quality_gates.contains_key("context_ready:design")
            || run.quality_gates.contains_key("input_domain_verified"),
        "expected design quality gates"
    );
    // Equal bytes from distinct task authors must not make gate selection depend
    // on artifact insertion/completion order.
    let context_session = team
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap()
        .artifact
        .as_ref()
        .unwrap()
        .session_id
        .clone();
    assert!(
        run.artifacts
            .iter()
            .all(|a| a.hash == run.artifacts[0].hash)
    );
    let mut reordered = run.clone();
    reordered
        .artifacts
        .sort_by_key(|a| a.session_id == context_session);
    f.runtime
        .store
        .lock()
        .unwrap()
        .save(&reordered, "test_reordered_equal_artifacts", None)
        .unwrap();
    let planning = step(&f, &reordered);
    assert_eq!(planning.status, Status::Ready, "{:?}", planning.diagnostic);
    assert!(
        planning
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .any(|t| t.stage == architect::team::TaskStage::Planning
                && t.state == TaskState::AwaitingVerification)
    );
}

#[test]
fn unsupported_mode_change_preserves_actual_native_reviews_and_stage_acceptance() {
    for mode in ["success", "mode_history"] {
        let f = TeamFixture::new(mode);
        let installed = f.install_team();
        let executed = step(&f, &installed);
        f.runtime
            .dispatch(
                &id(),
                Command::StepTeam {
                    run_id: executed.id.clone(),
                    scope_hash: executed.team_step_scope().unwrap(),
                },
            )
            .unwrap();
        let reviewed = f.wait_for_idle();
        let pending = architect::runtime::team::stage::pending(&reviewed);
        assert!(
            !pending.is_empty(),
            "{mode}: {:?}; {:?}",
            reviewed.diagnostic,
            reviewed
                .team
                .as_ref()
                .unwrap()
                .tasks
                .iter()
                .map(|t| (&t.id, t.state, &t.diagnostic))
                .collect::<Vec<_>>()
        );
        if mode == "mode_history" {
            assert!(reviewed.spec.check_command.is_empty());
            assert!(
                reviewed
                    .team
                    .as_ref()
                    .unwrap()
                    .tasks
                    .iter()
                    .any(|task| !task.check_plan.is_empty())
            );
        }
        let before = serde_json::to_value(&reviewed).unwrap();
        let error = f
            .runtime
            .dispatch(
                &id(),
                Command::ChangeMode {
                    run_id: reviewed.id.clone(),
                    mode: Mode::Autonomous,
                    expected_policy_revision: reviewed
                        .spec
                        .config
                        .as_ref()
                        .unwrap()
                        .config
                        .policy
                        .revision,
                },
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("restricted runner"), "{mode}: {error}");
        let after = f.current();
        assert_eq!(serde_json::to_value(&after).unwrap(), before);
        assert_eq!(
            serde_json::to_value(architect::runtime::team::stage::pending(&after)).unwrap(),
            serde_json::to_value(pending).unwrap()
        );
        let accepted = accept_fixture_results(&f, after);
        assert!(architect::runtime::team::stage::pending(&accepted).is_empty());
        assert!(!accepted.team_result_approvals.is_empty());
        assert!(accepted.mode_changes.is_empty());
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("workspace/source.txt")).unwrap(),
            "original input"
        );
    }
}

#[test]
fn missing_initial_check_command_reaches_planning_but_incomplete_proof_still_rejects() {
    let f = TeamFixture::new("mode_history_missing_plan");
    assert!(f.run.spec.check_command.is_empty());
    let queued = f
        .runtime
        .dispatch(
            &id(),
            Command::SendMessage {
                run_id: f.run.id.clone(),
                target_agent_id: "orchestrator".into(),
                text: "Choose checks for this research task".into(),
            },
        )
        .unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::Orchestrate {
                run_id: queued.id.clone(),
                scope_hash: planning::start_scope(&queued).unwrap(),
            },
        )
        .unwrap();
    let rejected = f.wait_for_idle();
    let round = rejected.orchestration_rounds.last().unwrap();
    assert_eq!(round.status, planning::RoundStatus::Rejected);
    assert!(round.output.is_some());
    assert!(round.diagnostic.as_ref().unwrap().contains("runner plan"));
    assert!(round.verifier_session.is_none());
    assert!(rejected.team.is_none() && rejected.task_checks.is_empty());
    let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    assert_eq!(
        calls.lines().count(),
        1,
        "only the read-only planner should run"
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/source.txt")).unwrap(),
        "original input"
    );
}

#[test]
fn mode_change_refuses_a_live_worker_even_while_paused() {
    let f = TeamFixture::new("lease_executor");
    let installed = f.install_team();
    f.runtime
        .dispatch(
            &id(),
            Command::StepTeam {
                run_id: installed.id.clone(),
                scope_hash: installed.team_step_scope().unwrap(),
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f.root.path().join("guardian-info.json").exists() {
        assert!(Instant::now() < deadline, "owned executor did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    let paused = f
        .runtime
        .dispatch(
            &id(),
            Command::Pause {
                run_id: installed.id.clone(),
            },
        )
        .unwrap();
    assert_eq!(paused.status, Status::Paused);
    let error = f
        .runtime
        .dispatch(
            &id(),
            Command::ChangeMode {
                run_id: paused.id.clone(),
                mode: Mode::Autonomous,
                expected_policy_revision: paused
                    .spec
                    .config
                    .as_ref()
                    .unwrap()
                    .config
                    .policy
                    .revision,
            },
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("active worker"), "{error}");
    let after = f.current();
    assert_eq!(
        serde_json::to_value(&after.spec).unwrap(),
        serde_json::to_value(&paused.spec).unwrap()
    );
    assert!(after.mode_changes.is_empty());
    f.runtime
        .dispatch(
            &id(),
            Command::Cancel {
                run_id: paused.id.clone(),
            },
        )
        .unwrap();
    while architect::recovery::WorkspaceLease::acquire(&paused.spec.workspace).is_err() {
        assert!(
            Instant::now() < deadline,
            "cancelled worker did not release its workspace"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(f.current().status, Status::Cancelled);
}

#[test]
fn test_step_team_rejects_stale_scope_hash() {
    let f = TeamFixture::new("success");
    let accepted = f.install_team();

    // Passing wrong scope hash must be rejected
    let err = f.runtime.dispatch(
        &id(),
        Command::StepTeam {
            run_id: accepted.id.clone(),
            scope_hash: "invalid-stale-hash".into(),
        },
    );

    assert!(err.is_err());
    let current = f.current();
    assert_eq!(current.status, Status::AwaitingStart);
    assert_eq!(current.attempt, accepted.attempt);
}

#[test]
fn test_step_team_handles_changes_requested_and_repair() {
    let f = TeamFixture::new("reject");
    let accepted = f.install_team();

    // First wave: executors run
    let scope_hash = accepted.team_step_scope().unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::StepTeam {
                run_id: accepted.id.clone(),
                scope_hash,
            },
        )
        .unwrap();

    let run = f.wait_for_idle();
    assert_eq!(run.status, Status::Ready, "{:?}", run.diagnostic);

    // Second wave: verifiers run, task-contextanalyst is rejected
    let scope_hash = run.team_step_scope().unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::StepTeam {
                run_id: run.id.clone(),
                scope_hash,
            },
        )
        .unwrap();

    let run = f.wait_for_idle();
    assert_eq!(run.status, Status::ChangesRequested);

    let team = run.team.as_ref().unwrap();
    let rejected_task = team
        .tasks
        .iter()
        .find(|t| t.state == TaskState::ChangesRequested)
        .expect("expected task with changes_requested");

    // Repair task
    let repair_plan = "Address defect and satisfy criteria";
    let repair_scope = run
        .team_repair_scope(&rejected_task.id, repair_plan)
        .unwrap();

    let repaired = f
        .runtime
        .dispatch(
            &id(),
            Command::RepairTask {
                run_id: run.id.clone(),
                task_id: rejected_task.id.clone(),
                plan: repair_plan.into(),
                scope_hash: repair_scope,
            },
        )
        .unwrap();

    assert_eq!(repaired.status, Status::Ready);
    let team_after_repair = repaired.team.as_ref().unwrap();
    let task_after_repair = team_after_repair
        .tasks
        .iter()
        .find(|t| t.id == rejected_task.id)
        .unwrap();
    assert_eq!(task_after_repair.state, TaskState::Planned);
    assert_eq!(task_after_repair.history.len(), 1);
}

fn accept_fixture_results(f: &TeamFixture, mut run: Run) -> Run {
    // This fixture's simulated user explicitly accepts every independently reviewed package.
    let packages = architect::runtime::team::stage::pending(&run);
    for package in packages.values() {
        run = f
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

fn step(f: &TeamFixture, run: &Run) -> Run {
    f.runtime
        .dispatch(
            &id(),
            Command::StepTeam {
                run_id: run.id.clone(),
                scope_hash: run.team_step_scope().unwrap(),
            },
        )
        .unwrap();
    accept_fixture_results(f, f.wait_for_idle())
}

fn managed_change(f: &TeamFixture) -> Run {
    let mut run = f.install_team();
    for _ in 0..12 {
        run = step(f, &run);
        if run.input_snapshot["source.txt"] == "managed result" {
            return run;
        }
        assert_eq!(run.status, Status::Ready, "{:?}", run.diagnostic);
    }
    panic!("fixture did not reach managed write");
}
fn before_managed_change(f: &TeamFixture) -> Run {
    let mut run = f.install_team();
    for _ in 0..12 {
        let team = run.team.as_ref().unwrap();
        if team.tasks.iter().filter(|t| t.deliverable).all(|t| {
            t.dependencies.iter().all(|id| {
                team.tasks
                    .iter()
                    .any(|parent| &parent.id == id && parent.state == TaskState::Verified)
            })
        }) {
            return run;
        }
        run = step(f, &run);
        assert_eq!(run.status, Status::Ready, "{:?}", run.diagnostic);
    }
    panic!("fixture did not reach writer frontier");
}
fn refresh_sources(f: &TeamFixture, run: &Run, task_id: &str) -> Run {
    f.runtime
        .dispatch(
            &id(),
            Command::RefreshTaskSources {
                run_id: run.id.clone(),
                task_id: task_id.into(),
                scope_hash: run.task_source_refresh_scope(task_id).unwrap(),
            },
        )
        .unwrap();
    accept_fixture_results(f, f.wait_for_idle())
}
fn refresh_prerequisites(f: &TeamFixture, mut run: Run, writer_id: &str) -> Run {
    for _ in 0..20 {
        if run.task_source_refresh_scope(writer_id).is_ok() {
            return run;
        }
        let eligible: Vec<_> = run
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .filter(|t| t.id != writer_id)
            .filter(|t| run.task_source_refresh_scope(&t.id).is_ok())
            .map(|t| t.id.clone())
            .collect();
        assert!(
            !eligible.is_empty(),
            "no source refresh frontier: {:?}",
            run.diagnostic
        );
        for task_id in eligible {
            run = refresh_sources(f, &run, &task_id);
        }
        run = step(f, &run);
    }
    panic!("fixture refresh frontier did not reach writer");
}

#[test]
fn managed_write_invalidates_old_proofs_and_refresh_keeps_effects_and_native_history() {
    let f = TeamFixture::new("managed_success");
    let before = managed_change(&f);
    assert_eq!(before.status, Status::Blocked, "{:?}", before.diagnostic);
    assert!(
        !before.reconciliation_required,
        "known committed effects must not require replay reconciliation"
    );
    let writer = before
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.deliverable && t.artifact.is_some())
        .unwrap();
    assert_eq!(writer.state, TaskState::Stale);
    let original = writer.artifact.as_ref().unwrap();
    let binding = before
        .task_executions
        .iter()
        .find(|e| Some(&e.id) == original.execution_id.as_ref())
        .unwrap();
    assert_eq!(binding.input_snapshot["source.txt"], "original input");
    assert_eq!(
        binding.after_snapshot.as_ref().unwrap()["source.txt"],
        "managed result"
    );
    assert_eq!(
        binding.state,
        architect::runtime::team::effects::ExecutionState::Committed
    );
    assert_eq!(
        binding.receipts[0].before_hash,
        architect::hash("original input")
    );
    assert_eq!(
        binding.receipts[0].after_hash,
        architect::hash("managed result")
    );
    assert_eq!(
        original.source_hashes["file:source.txt"],
        architect::hash("original input")
    );
    let old_context = before
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap()
        .artifact
        .clone()
        .unwrap();
    let receipts = serde_json::to_value(&before.pending_edits).unwrap();
    let refreshed = refresh_sources(&f, &before, "task-contextanalyst");
    let context = refreshed
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    assert_eq!(
        context.state,
        TaskState::AwaitingVerification,
        "{:?}",
        refreshed.diagnostic
    );
    assert_eq!(context.history.len(), 1);
    assert_eq!(context.history[0].artifact.as_ref().unwrap(), &old_context);
    assert!(context.history[0].source_refresh.is_some());
    assert_ne!(context.artifact.as_ref().unwrap().id, old_context.id);
    assert_ne!(
        context.artifact.as_ref().unwrap().session_id,
        old_context.session_id
    );
    assert_eq!(
        context.artifact.as_ref().unwrap().source_hashes["file:source.txt"],
        architect::hash("managed result")
    );
    assert_eq!(
        serde_json::to_value(&refreshed.pending_edits).unwrap(),
        receipts
    );
    let verified = step(&f, &refreshed);
    assert_eq!(
        verified
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == "task-contextanalyst")
            .unwrap()
            .state,
        TaskState::Verified,
        "{:?}",
        verified.diagnostic
    );
    let current_gate = &verified.quality_gates["context_ready:design"];
    assert_eq!(
        current_gate.source_hashes["file:source.txt"],
        architect::hash("managed result")
    );
    assert!(verified.reviews.iter().any(|r| {
        r.checks
            .iter()
            .any(|c| c.check_id == format!("SOURCE_REFRESH:{}", old_context.id))
    }));
    for (id, turn) in &before.session_ledger.turns {
        assert_eq!(&verified.session_ledger.turns[id], turn);
    }
    assert_eq!(
        serde_json::to_value(&verified.pending_edits).unwrap(),
        receipts
    );
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&verified.spec.workspace).join("source.txt"))
            .unwrap(),
        "managed result"
    );
}

#[test]
fn writer_source_refresh_is_read_only_and_new_review_keeps_original_applied_proposal() {
    let f = TeamFixture::new("managed_success");
    let before = managed_change(&f);
    let writer = before
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.deliverable && t.artifact.is_some())
        .unwrap();
    let original = writer.artifact.clone().unwrap();
    let ready = refresh_prerequisites(&f, before.clone(), &writer.id);
    let refreshed = refresh_sources(&f, &ready, &writer.id);
    let reviewed = step(&f, &refreshed);
    assert!(
        reviewed
            .evidence
            .iter()
            .any(|e| e.source.starts_with("runner:") && e.success)
    );
    assert!(reviewed.task_checks.iter().any(|i| i.task_id == writer.id
        && i.state == architect::runtime::team::checks::CheckState::Completed
        && i.success));
    let current = reviewed
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == writer.id)
        .unwrap();
    assert_eq!(
        current.state,
        TaskState::Verified,
        "{:?}",
        current.diagnostic
    );
    assert_eq!(
        current.history.last().unwrap().artifact.as_ref(),
        Some(&original)
    );
    let current_artifact = current.artifact.as_ref().unwrap();
    let execution = reviewed
        .task_executions
        .iter()
        .find(|e| Some(&e.id) == current_artifact.execution_id.as_ref())
        .unwrap();
    assert!(execution.receipts.is_empty());
    assert_eq!(
        execution.after_snapshot.as_ref(),
        Some(&execution.input_snapshot)
    );
    assert_eq!(
        current_artifact.source_hashes["file:source.txt"],
        architect::hash("managed result")
    );
    let original_execution = reviewed
        .task_executions
        .iter()
        .find(|e| Some(&e.id) == original.execution_id.as_ref())
        .unwrap();
    assert_eq!(original_execution.receipts.len(), 1);
    assert_eq!(
        original_execution.input_snapshot["source.txt"],
        "original input"
    );
    assert_eq!(
        serde_json::to_value(&reviewed.pending_edits).unwrap(),
        serde_json::to_value(&before.pending_edits).unwrap()
    );
    let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    let last: serde_json::Value = serde_json::from_str(calls.lines().last().unwrap()).unwrap();
    assert!(
        last["prompt"]
            .as_str()
            .unwrap()
            .contains(&original.content_hash)
    );
    assert!(
        last["prompt"]
            .as_str()
            .unwrap()
            .contains("execution_effects")
    );
    assert!(last["prompt"].as_str().unwrap().contains("managed result"));
    let reopened = Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap();
    let durable = reopened.store.lock().unwrap().get(&reviewed.id).unwrap();
    assert_eq!(
        serde_json::to_value(&durable.task_executions).unwrap(),
        serde_json::to_value(&reviewed.task_executions).unwrap()
    );
}

#[test]
fn refresh_cannot_repeat_writer_effect_or_use_stale_approval() {
    let f = TeamFixture::new("managed_repeat_write");
    let before = managed_change(&f);
    let writer_id = before
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.deliverable && t.artifact.is_some())
        .unwrap()
        .id
        .clone();
    let ready = refresh_prerequisites(&f, before, &writer_id);
    let scope = ready.task_source_refresh_scope(&writer_id).unwrap();
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::RefreshTaskSources {
                    run_id: ready.id.clone(),
                    task_id: writer_id.clone(),
                    scope_hash: "0".repeat(64)
                }
            )
            .is_err()
    );
    let receipts = serde_json::to_value(&ready.pending_edits).unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::RefreshTaskSources {
                run_id: ready.id.clone(),
                task_id: writer_id.clone(),
                scope_hash: scope,
            },
        )
        .unwrap();
    let blocked = f.wait_for_idle();
    let writer = blocked
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == writer_id)
        .unwrap();
    assert_eq!(writer.state, TaskState::Blocked);
    assert!(writer.diagnostic.as_ref().unwrap().contains("read-only"));
    assert_eq!(
        serde_json::to_value(&blocked.pending_edits).unwrap(),
        receipts
    );
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&blocked.spec.workspace).join("source.txt"))
            .unwrap(),
        "managed result"
    );
}

#[test]
fn disjoint_native_writers_share_wave_and_keep_their_original_input_and_owned_receipts() {
    let f = TeamFixture::new("managed_disjoint");
    let result = managed_change(&f);
    assert_eq!(result.input_snapshot["source.txt"], "managed result");
    assert_eq!(result.input_snapshot["second.txt"], "managed result");
    let writes: Vec<_> = result
        .task_executions
        .iter()
        .filter(|e| !e.receipts.is_empty())
        .collect();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].wave_attempt, writes[1].wave_attempt);
    assert_eq!(writes[0].input_snapshot, writes[1].input_snapshot);
    assert_ne!(writes[0].session_id, writes[1].session_id);
    assert_ne!(writes[0].receipts[0].path, writes[1].receipts[0].path);
    for execution in writes {
        assert_eq!(
            execution.state,
            architect::runtime::team::effects::ExecutionState::Committed
        );
        assert_eq!(execution.receipts.len(), 1);
        assert_eq!(
            execution.receipts[0].before_hash,
            architect::hash(&execution.input_snapshot[&execution.receipts[0].path])
        );
        assert_eq!(
            execution.receipts[0].after_hash,
            architect::hash("managed result")
        );
    }
}

#[test]
fn directory_scoped_native_writes_keep_aggregate_versions_and_durable_diff_baseline() {
    let f = TeamFixture::new_scoped("managed_disjoint", true);
    let result = managed_change(&f);
    let current = architect::workspace::read_inputs(
        std::path::Path::new(&result.spec.workspace),
        &result.spec.input_files,
    )
    .unwrap();
    assert_eq!(result.input_snapshot, current);
    assert_eq!(result.diff_baseline(), f.run.diff_baseline());
    assert_eq!(result.diff_baseline()["source.txt"], "original input");
    let diff = architect::workspace::compute_diff(
        std::path::Path::new(&result.spec.workspace),
        result.diff_baseline(),
    )
    .unwrap();
    assert_eq!(diff.files.len(), 2);
    assert_eq!(diff.files[0].path, "second.txt");
    assert_eq!(diff.files[1].path, "source.txt");
    let restored = Store::open(&f.root.path().join("state.db"))
        .unwrap()
        .get(&result.id)
        .unwrap();
    assert_eq!(restored.diff_baseline(), f.run.diff_baseline());
    let display = architect::tui::model::Snapshot::from_document(
        &serde_json::to_value(&restored).unwrap(),
        &serde_json::json!({}),
    );
    assert!(display.run_details["Diff"].contains("Total files changed: 2"));
}

#[test]
fn native_effect_intent_and_receipt_failure_preserve_uncertainty_without_replay() {
    for boundary in ["task_execution_intent", "task_execution_edit_applied"] {
        let f = TeamFixture::new("managed_success");
        let ready = before_managed_change(&f);
        let connection = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
        connection.execute_batch(&format!("CREATE TRIGGER refuse_effect BEFORE INSERT ON events WHEN NEW.kind='{boundary}' BEGIN SELECT RAISE(ABORT, 'injected source effect checkpoint failure'); END;")).unwrap();
        let result = step(&f, &ready);
        assert_eq!(result.status, Status::Blocked);
        assert!(result.reconciliation_required);
        let physical = std::fs::read_to_string(
            std::path::Path::new(&result.spec.workspace).join("source.txt"),
        )
        .unwrap();
        if boundary == "task_execution_intent" {
            assert_eq!(physical, "original input");
            assert_eq!(result.task_executions.len(), ready.task_executions.len());
        } else {
            assert_eq!(physical, "managed result");
            let execution = result.task_executions.last().unwrap();
            assert_eq!(
                execution.state,
                architect::runtime::team::effects::ExecutionState::Intent
            );
            assert!(execution.after_snapshot.is_none());
            assert_eq!(
                execution.receipts[0].state,
                architect::recovery::EditState::Pending
            );
        }
        assert!(
            f.runtime
                .dispatch(
                    &id(),
                    Command::StepTeam {
                        run_id: result.id.clone(),
                        scope_hash: result.team_step_scope().unwrap()
                    }
                )
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(
                std::path::Path::new(&result.spec.workspace).join("source.txt")
            )
            .unwrap(),
            physical
        );
    }
}

#[test]
fn source_refresh_refuses_corrupted_original_inputs_missing_effects_and_failed_approval() {
    let f = TeamFixture::new("managed_success");
    let before = managed_change(&f);
    let task_id = "task-contextanalyst";
    let artifact = before
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == task_id)
        .unwrap()
        .artifact
        .as_ref()
        .unwrap();
    let mut tampered = before.clone();
    tampered
        .task_executions
        .iter_mut()
        .find(|e| Some(&e.id) == artifact.execution_id.as_ref())
        .unwrap()
        .input_snapshot
        .insert("source.txt".into(), "invented input".into());
    assert!(tampered.task_source_refresh_scope(task_id).is_err());
    let mut missing = before.clone();
    missing
        .task_executions
        .retain(|e| Some(&e.id) != artifact.execution_id.as_ref());
    assert!(missing.task_source_refresh_scope(task_id).is_err());
    let command = Command::RefreshTaskSources {
        run_id: before.id.clone(),
        task_id: task_id.into(),
        scope_hash: before.task_source_refresh_scope(task_id).unwrap(),
    };
    let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    let connection = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_source_refresh BEFORE INSERT ON events WHEN NEW.kind='task_source_refresh_started' BEGIN SELECT RAISE(ABORT, 'injected refresh approval failure'); END;").unwrap();
    assert!(f.runtime.dispatch(&id(), command).is_err());
    assert_eq!(
        serde_json::to_value(f.current()).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
}

#[test]
fn source_refresh_is_available_through_daemon_tui_and_cli_and_keeps_scope_pinned() {
    use architect::tui::backend::{Action, Backend, IpcBackend};
    let f = TeamFixture::new("managed_success");
    let before = managed_change(&f);
    std::fs::set_permissions(f.root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let _daemon = TestDaemon(
        std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("daemon")
            .arg(f.root.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut backend = IpcBackend::new(f.root.path());
    let deadline = Instant::now() + Duration::from_secs(5);
    let snapshot = loop {
        match backend.snapshot(&before.id) {
            Ok(snapshot) => break snapshot,
            Err(error) => {
                assert!(Instant::now() < deadline, "{error}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    assert_eq!(
        snapshot.task_source_refreshes["task-contextanalyst"],
        before
            .task_source_refresh_scope("task-contextanalyst")
            .unwrap()
    );
    assert!(
        backend
            .act(
                &before.id,
                Action::RefreshTaskSources {
                    task_id: "task-contextanalyst".into(),
                    scope_hash: "0".repeat(64)
                }
            )
            .is_err()
    );
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("refresh-task-sources")
        .arg(f.root.path())
        .arg(&before.id)
        .arg("task-contextanalyst")
        .arg("--approve-scoped")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let after = f.wait_for_idle();
    let task = after
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    assert_eq!(
        task.state,
        TaskState::AwaitingVerification,
        "{:?}",
        task.diagnostic
    );
    assert_eq!(
        serde_json::to_value(&after.pending_edits).unwrap(),
        serde_json::to_value(&before.pending_edits).unwrap()
    );
    let snapshot = backend.snapshot(&after.id).unwrap();
    assert!(
        !snapshot
            .task_source_refreshes
            .contains_key("task-contextanalyst")
    );
}

#[test]
fn context_gate_requires_retained_model_sections_and_independent_current_inspection() {
    for mode in [
        "missing_context",
        "omitted_context_section",
        "stale_context_fact",
        "unresolved_context",
        "missing_context_review",
        "wrong_context_model_hash",
        "model_review_unknown_fact",
    ] {
        let f = TeamFixture::new(mode);
        let executed = step(&f, &f.install_team());
        let stopped = if executed.status == Status::Ready {
            step(&f, &executed)
        } else {
            executed
        };
        let task = stopped
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == "task-contextanalyst")
            .unwrap();
        assert_eq!(
            task.state,
            TaskState::Blocked,
            "{mode}: {:?}",
            stopped.diagnostic
        );
        assert!(
            !stopped.quality_gates.contains_key("context_ready:design"),
            "{mode}"
        );
        assert!(
            stopped
                .session_ledger
                .turns
                .values()
                .any(|turn| turn.output.is_some()),
            "raw model/review output must survive: {mode}"
        );
        assert_eq!(
            std::fs::read_to_string(
                std::path::Path::new(&stopped.spec.workspace).join("source.txt")
            )
            .unwrap(),
            "original input"
        );
    }
}

#[test]
fn native_options_review_inspects_exact_model_and_records_independent_scenarios() {
    let f = TeamFixture::new("success");
    let executed = step(&f, &f.install_team());
    let discovered = step(&f, &executed);
    let modeled = step(&f, &discovered);
    assert_eq!(modeled.status, Status::Ready, "{:?}", modeled.diagnostic);
    let reviewed = step(&f, &modeled);
    let task = reviewed
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-optionsanalyst")
        .unwrap();
    assert_eq!(task.state, TaskState::Verified, "{:?}", task.diagnostic);
    let reference = task.artifact.as_ref().unwrap();
    let artifact = reviewed
        .artifacts
        .iter()
        .find(|a| a.id == reference.id)
        .unwrap();
    let output: architect::team::models::Output = serde_json::from_str(&artifact.content).unwrap();
    let model = output.solution.as_ref().unwrap();
    assert_eq!(model.scenarios.len(), 9);
    assert_eq!(reference.content_hash, architect::hash(&artifact.content));
    let review = reviewed
        .reviews
        .iter()
        .find(|r| {
            r.artifact_hash == reference.content_hash && r.executor_session == reference.session_id
        })
        .unwrap();
    let inspection = review
        .quality
        .as_ref()
        .unwrap()
        .model_review
        .as_ref()
        .unwrap();
    assert_eq!(
        inspection.solution_hash.as_deref(),
        architect::team::models::model_hashes(&output)["solution"].as_str()
    );
    assert_eq!(inspection.independent_simulations.len(), 9);
    for scenario in &inspection.independent_simulations {
        assert!(
            model
                .model
                .simulations
                .iter()
                .all(|s| serde_json::to_value(s).unwrap()
                    != serde_json::to_value(&scenario.scenario).unwrap())
        );
    }
    let gate = &reviewed.quality_gates["solution_selected:design"];
    assert_eq!(gate.artifact_hash, reference.content_hash);
    assert_eq!(
        gate.report_hash,
        architect::hash(serde_json::to_vec(review.quality.as_ref().unwrap()).unwrap())
    );
    let native = &reviewed.session_ledger.sessions[&review.verifier_session];
    assert_ne!(
        native.native_session_id,
        reviewed.session_ledger.sessions[&reference.session_id].native_session_id
    );
    let reopened = Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap();
    let durable = reopened.store.lock().unwrap().get(&reviewed.id).unwrap();
    assert_eq!(
        serde_json::to_value(&durable.artifacts).unwrap(),
        serde_json::to_value(&reviewed.artifacts).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&durable.reviews).unwrap(),
        serde_json::to_value(&reviewed.reviews).unwrap()
    );
}

#[test]
fn solution_gate_rejects_missing_or_stale_model_obligations_and_copied_reviews() {
    for mode in [
        "missing_solution",
        "stale_model_revision",
        "missing_design_class",
        "duplicate_scenario_id",
        "missing_model_criterion",
        "copied_simulations",
        "wrong_solution_hash",
        "missing_planning_model_bind",
        "false_simulation",
    ] {
        let f = TeamFixture::new(mode);
        let executed = step(&f, &f.install_team());
        let discovered = step(&f, &executed);
        let modeled = step(&f, &discovered);
        let stopped = if modeled.status == Status::Ready {
            step(&f, &modeled)
        } else {
            modeled
        };
        let task = stopped
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == "task-optionsanalyst")
            .unwrap();
        assert_eq!(
            task.state,
            TaskState::Blocked,
            "{mode}: {:?}",
            task.diagnostic
        );
        assert!(
            !stopped
                .quality_gates
                .contains_key("solution_selected:design"),
            "{mode}"
        );
        assert!(
            stopped.quality_gates.contains_key("context_ready:design"),
            "{mode}: prerequisite evidence was lost"
        );
        if matches!(
            mode,
            "false_simulation" | "wrong_solution_hash" | "missing_planning_model_bind"
        ) {
            let reconciled = reconcile_team(&f, &stopped);
            assert!(
                reconciled
                    .task_verification_retry_scope("task-optionsanalyst")
                    .is_err(),
                "{mode}: replacement must not erase negative or contradictory model inspection"
            );
        }
    }
}

fn reconcile_team(f: &TeamFixture, run: &Run) -> Run {
    let command = Command::Reconcile {
        run_id: run.id.clone(),
        scope_hash: run.scope_hash("reconcile"),
        input_hash: architect::recovery::observe(run).unwrap().hash,
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match f.runtime.dispatch(&id(), command.clone()) {
            Ok(run) => return run,
            Err(error) if error.to_string().contains("worker is still stopping") => {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("reconciliation failed: {error}"),
        }
    }
}

fn retry_task(run: &Run) -> Command {
    let task_id = "task-contextanalyst";
    Command::RetryTaskVerification {
        run_id: run.id.clone(),
        task_id: task_id.into(),
        scope_hash: run.task_verification_retry_scope(task_id).unwrap(),
    }
}

#[test]
fn approved_task_check_is_observed_once_and_verifier_retry_reuses_its_exact_receipt() {
    let f = TeamFixture::new("task_check_retry");
    let executed = step(&f, &f.install_team());
    assert!(!f.root.path().join("workspace/runner-count").exists());
    let blocked = step(&f, &executed);
    assert_eq!(blocked.task_checks.len(), 1);
    let record = &blocked.task_checks[0];
    assert_eq!(
        record.state,
        architect::runtime::team::checks::CheckState::Completed
    );
    assert!(record.success);
    assert_eq!(record.before_hash, record.after_hash.clone().unwrap());
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/runner-count")).unwrap(),
        "1"
    );
    let stopped = reconcile_team(&f, &blocked);
    std::fs::write(f.root.path().join("retry-ready"), "ready").unwrap();
    f.runtime.dispatch(&id(), retry_task(&stopped)).unwrap();
    let after = f.wait_for_idle();
    assert_eq!(
        after
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == "task-contextanalyst")
            .unwrap()
            .state,
        TaskState::Verified,
        "{:?}",
        after.diagnostic
    );
    assert_eq!(
        serde_json::to_value(&after.task_checks).unwrap(),
        serde_json::to_value(&stopped.task_checks).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/runner-count")).unwrap(),
        "1"
    );
    let task = after
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == record.task_id)
        .unwrap();
    let review = after
        .reviews
        .iter()
        .find(|r| Some(&r.verifier_session) == task.review.as_ref().map(|r| &r.session_id))
        .unwrap();
    let result = review
        .checks
        .iter()
        .find(|c| c.check_id == "RUNNER:context-check")
        .unwrap();
    assert!(
        result
            .evidence_ids
            .contains(record.evidence_id.as_ref().unwrap())
    );
}

#[test]
fn actual_failed_task_check_reaches_independent_rejection_without_creating_a_gate() {
    let f = TeamFixture::new("task_check_failure");
    let executed = step(&f, &f.install_team());
    let rejected = step(&f, &executed);
    let task = rejected
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    assert_eq!(
        task.state,
        TaskState::ChangesRequested,
        "{:?}",
        task.diagnostic
    );
    assert!(!rejected.task_checks[0].success);
    assert_eq!(
        rejected.task_checks[0].state,
        architect::runtime::team::checks::CheckState::Completed
    );
    assert!(!rejected.quality_gates.contains_key("context_ready:design"));
    assert!(
        task.review
            .as_ref()
            .is_some_and(|r| !r.accepted && !r.finding_ids.is_empty())
    );
}

#[test]
fn lying_positive_review_cannot_hide_actual_failed_task_check() {
    let f = TeamFixture::new("task_check_lie");
    let executed = step(&f, &f.install_team());
    let stopped = step(&f, &executed);
    assert_eq!(
        stopped
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == "task-contextanalyst")
            .unwrap()
            .state,
        TaskState::Blocked
    );
    assert!(!stopped.quality_gates.contains_key("context_ready:design"));
    let reconciled = reconcile_team(&f, &stopped);
    assert!(
        reconciled
            .task_verification_retry_scope("task-contextanalyst")
            .is_err()
    );
}

#[test]
fn task_check_input_mutation_and_receipt_commit_failure_never_launch_the_verifier() {
    for boundary in [
        "task_check_intent",
        "task_check_output_captured",
        "task_check_finished",
        "input_mutation",
    ] {
        let f = TeamFixture::new(if boundary == "input_mutation" {
            "task_check_mutation"
        } else {
            "task_check_success"
        });
        let executed = step(&f, &f.install_team());
        if boundary != "input_mutation" {
            let connection = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
            connection.execute_batch(&format!("CREATE TRIGGER refuse_task_check BEFORE INSERT ON events WHEN NEW.kind='{boundary}' BEGIN SELECT RAISE(ABORT, 'injected task check checkpoint failure'); END;")).unwrap();
        }
        let stopped = step(&f, &executed);
        if boundary == "task_check_output_captured" {
            assert!(stopped.task_check_outcomes.is_empty());
            let reconciled = reconcile_team(&f, &stopped);
            assert!(architect::runtime::team::check_recovery::pending(&reconciled).is_empty());
        }
        assert_eq!(
            stopped
                .team
                .as_ref()
                .unwrap()
                .tasks
                .iter()
                .find(|t| t.id == "task-contextanalyst")
                .unwrap()
                .state,
            TaskState::Blocked
        );
        assert!(stopped.reconciliation_required);
        assert!(
            !stopped
                .session_ledger
                .sessions
                .values()
                .any(
                    |s| s.scope.task_id.as_deref() == Some("task-contextanalyst")
                        && s.scope.role == RoleKind::Verifier
                )
        );
        if boundary == "task_check_intent" {
            assert!(stopped.task_checks.is_empty());
            assert!(!f.root.path().join("workspace/runner-count").exists());
        } else {
            assert_eq!(
                std::fs::read_to_string(f.root.path().join("workspace/runner-count")).unwrap(),
                "1"
            );
            assert_ne!(
                stopped.task_checks[0].state,
                architect::runtime::team::checks::CheckState::Completed
            );
        }
    }
}

#[test]
fn cancelled_task_check_preserves_unknown_invocation_and_admits_no_verifier() {
    let f = TeamFixture::new("task_check_hang");
    let executed = step(&f, &f.install_team());
    f.runtime
        .dispatch(
            &id(),
            Command::StepTeam {
                run_id: executed.id.clone(),
                scope_hash: executed.team_step_scope().unwrap(),
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f.root.path().join("workspace/check-started").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    f.runtime
        .dispatch(
            &id(),
            Command::Cancel {
                run_id: executed.id.clone(),
            },
        )
        .unwrap();
    loop {
        if let Ok(_lease) = architect::recovery::WorkspaceLease::acquire(&executed.spec.workspace) {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let cancelled = f.current();
    assert_eq!(cancelled.status, Status::Cancelled);
    assert_eq!(
        cancelled.task_checks[0].state,
        architect::runtime::team::checks::CheckState::Unknown
    );
    assert!(cancelled.task_checks[0].evidence_id.is_none());
    assert!(
        !cancelled
            .session_ledger
            .sessions
            .values()
            .any(
                |s| s.scope.task_id.as_deref() == Some("task-contextanalyst")
                    && s.scope.role == RoleKind::Verifier
            )
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/runner-count")).unwrap(),
        "1"
    );
}

#[test]
fn proposal_without_required_runner_plan_is_rejected_before_team_installation() {
    let f = TeamFixture::new("task_check_missing_plan");
    let queued = f
        .runtime
        .dispatch(
            &id(),
            Command::SendMessage {
                run_id: f.run.id.clone(),
                target_agent_id: "orchestrator".into(),
                text: "Propose team".into(),
            },
        )
        .unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::Orchestrate {
                run_id: queued.id.clone(),
                scope_hash: planning::start_scope(&queued).unwrap(),
            },
        )
        .unwrap();
    let rejected = f.wait_for_idle();
    assert!(rejected.team.is_none());
    assert_ne!(
        rejected.orchestration_rounds[0].status,
        planning::RoundStatus::Reviewed
    );
    assert!(
        rejected.orchestration_rounds[0]
            .diagnostic
            .as_ref()
            .unwrap()
            .contains("runner plan")
    );
}

#[test]
fn task_verification_recovery_preserves_executor_and_runs_only_selected_verifier() {
    let f = TeamFixture::new("retry_context_once");
    let executed = step(&f, &f.install_team());
    let blocked = step(&f, &executed);
    assert_eq!(blocked.status, Status::Blocked);
    assert!(
        blocked
            .task_verification_retry_scope("task-contextanalyst")
            .is_err(),
        "reconciliation must precede recovery"
    );
    let before = reconcile_team(&f, &blocked);
    let task = before
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    assert_eq!(task.state, TaskState::Blocked);
    let old_verifier = before
        .team
        .as_ref()
        .unwrap()
        .agents
        .iter()
        .find(|a| a.id == task.verifier_id)
        .unwrap()
        .session_id
        .clone()
        .unwrap();
    let calls_before = std::fs::read_to_string(f.root.path().join("calls.jsonl"))
        .unwrap()
        .lines()
        .count();
    let source_before =
        std::fs::read_to_string(std::path::Path::new(&before.spec.workspace).join("source.txt"))
            .unwrap();
    std::fs::write(f.root.path().join("retry-ready"), "ready").unwrap();
    let key = id();
    let command = retry_task(&before);
    f.runtime.dispatch(&key, command.clone()).unwrap();
    let after = f.wait_for_idle();
    assert!(
        f.runtime.dispatch(&key, command).is_ok(),
        "idempotent retry must not launch twice"
    );
    let recovered = after
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == task.id)
        .unwrap();
    assert_eq!(
        recovered.state,
        TaskState::Verified,
        "{:?}",
        after.diagnostic
    );
    assert_eq!(recovered.attempt, task.attempt);
    assert_eq!(
        serde_json::to_value(&recovered.artifact).unwrap(),
        serde_json::to_value(&task.artifact).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&recovered.history).unwrap(),
        serde_json::to_value(&task.history).unwrap()
    );
    assert_eq!(after.spec_revision, before.spec_revision);
    assert_eq!(
        serde_json::to_value(&after.artifacts).unwrap(),
        serde_json::to_value(&before.artifacts).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&after.pending_edits).unwrap(),
        serde_json::to_value(&before.pending_edits).unwrap()
    );
    assert_eq!(after.input_snapshot, before.input_snapshot);
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&after.spec.workspace).join("source.txt"))
            .unwrap(),
        source_before
    );
    assert_eq!(
        after.session_ledger.sessions.len(),
        before.session_ledger.sessions.len() + 1
    );
    for (id, session) in &before.session_ledger.sessions {
        assert_eq!(
            &after.session_ledger.sessions[id], session,
            "prior native session changed"
        );
    }
    for (id, turn) in &before.session_ledger.turns {
        assert_eq!(
            &after.session_ledger.turns[id], turn,
            "prior raw output changed"
        );
    }
    for sibling in before
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .filter(|t| t.id != task.id)
    {
        assert_eq!(
            serde_json::to_value(
                after
                    .team
                    .as_ref()
                    .unwrap()
                    .tasks
                    .iter()
                    .find(|t| t.id == sibling.id)
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(sibling).unwrap(),
            "unrelated task was scheduled or reset"
        );
    }
    let verifier = after
        .team
        .as_ref()
        .unwrap()
        .agents
        .iter()
        .find(|a| a.id == task.verifier_id)
        .unwrap();
    assert!(verifier.session_history.contains(&old_verifier));
    assert_ne!(verifier.session_id.as_ref(), Some(&old_verifier));
    assert_eq!(recovered.verification_retries.len(), 1);
    assert_eq!(
        recovered.verification_retries[0].previous_session.as_ref(),
        Some(&old_verifier)
    );
    let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    assert_eq!(calls.lines().count(), calls_before + 1);
    let call: serde_json::Value = serde_json::from_str(calls.lines().last().unwrap()).unwrap();
    let prompt = call["prompt"].as_str().unwrap();
    assert!(prompt.contains("independently verifying task task-contextanalyst"));
    assert!(prompt.contains("prior_verification_outputs"));
    let context = serde_json::Deserializer::from_str(
        prompt.split("Immutable task context: ").nth(1).unwrap(),
    )
    .into_iter::<serde_json::Value>()
    .next()
    .unwrap()
    .unwrap();
    let prior_turn = &before.session_ledger.turns[before.session_ledger.sessions[&old_verifier]
        .current_turn_id
        .as_ref()
        .unwrap()];
    assert_eq!(
        context["prior_verification_outputs"][0]["output"].as_str(),
        prior_turn.output.as_deref()
    );
    assert_eq!(
        context["prior_verification_outputs"][0]["turn_id"].as_str(),
        Some(prior_turn.id.as_str())
    );
    let reopened = Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap();
    let durable = reopened.store.lock().unwrap().get(&after.id).unwrap();
    assert_eq!(
        serde_json::to_value(&durable.team).unwrap(),
        serde_json::to_value(&after.team).unwrap()
    );
}

#[test]
fn task_recovery_refuses_stale_scope_sources_authorship_and_failed_commit() {
    let f = TeamFixture::new("retry_context_once");
    let executed = step(&f, &f.install_team());
    let blocked = step(&f, &executed);
    let before = reconcile_team(&f, &blocked);
    let command = retry_task(&before);
    let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    std::fs::write(
        std::path::Path::new(&before.spec.workspace).join("source.txt"),
        "external mutation",
    )
    .unwrap();
    assert!(f.runtime.dispatch(&id(), command.clone()).is_err());
    std::fs::write(
        std::path::Path::new(&before.spec.workspace).join("source.txt"),
        "original input",
    )
    .unwrap();
    let mut modified = before.clone();
    modified.diagnostic = Some("new control observation".into());
    f.runtime
        .store
        .lock()
        .unwrap()
        .save(&modified, "test_new_control_observation", None)
        .unwrap();
    assert!(f.runtime.dispatch(&id(), command.clone()).is_err());
    let task = modified
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    let artifact = task.artifact.as_ref().unwrap();
    let executor_id = artifact.session_id.clone();
    modified
        .session_ledger
        .sessions
        .get_mut(&executor_id)
        .unwrap()
        .scope
        .agent_id = "other-agent".into();
    assert!(
        modified
            .task_verification_retry_scope("task-contextanalyst")
            .is_err()
    );
    let mut missing_receipt = before.clone();
    missing_receipt
        .evidence
        .retain(|e| !artifact.evidence_refs.contains(&e.id));
    assert!(
        missing_receipt
            .task_verification_retry_scope("task-contextanalyst")
            .is_err()
    );
    let mut forged_receipt = before.clone();
    forged_receipt
        .evidence
        .iter_mut()
        .find(|e| artifact.evidence_refs.contains(&e.id))
        .unwrap()
        .source = "runner:[]".into();
    assert!(
        forged_receipt
            .task_verification_retry_scope("task-contextanalyst")
            .is_err()
    );
    let mut wrong_stage = before.clone();
    wrong_stage
        .artifacts
        .iter_mut()
        .find(|a| a.id == artifact.id)
        .unwrap()
        .stage = Stage::Delivery;
    assert!(
        wrong_stage
            .task_verification_retry_scope("task-contextanalyst")
            .is_err()
    );
    f.runtime
        .store
        .lock()
        .unwrap()
        .save(&before, "test_restore_snapshot", None)
        .unwrap();
    let connection = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_task_retry BEFORE INSERT ON events WHEN NEW.kind='task_verification_retry_started' BEGIN SELECT RAISE(ABORT, 'injected retry approval failure'); END;").unwrap();
    assert!(f.runtime.dispatch(&id(), command).is_err());
    let after = f.current();
    assert_eq!(
        serde_json::to_value(&after).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
}

#[test]
fn repeated_incomplete_verifier_recovery_preserves_every_turn_in_same_executor_attempt() {
    let f = TeamFixture::new("retry_context_once");
    let executed = step(&f, &f.install_team());
    let blocked = step(&f, &executed);
    let first = reconcile_team(&f, &blocked);
    f.runtime.dispatch(&id(), retry_task(&first)).unwrap();
    let blocked_again = f.wait_for_idle();
    let second = reconcile_team(&f, &blocked_again);
    std::fs::write(f.root.path().join("retry-ready"), "ready").unwrap();
    f.runtime.dispatch(&id(), retry_task(&second)).unwrap();
    let after = f.wait_for_idle();
    let task = after
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    assert_eq!(task.state, TaskState::Verified, "{:?}", after.diagnostic);
    assert_eq!(task.attempt, 1);
    assert!(task.history.is_empty());
    assert_eq!(task.verification_retries.len(), 2);
    assert_ne!(
        task.verification_retries[0].previous_session,
        task.verification_retries[1].previous_session
    );
    let verifier = after
        .team
        .as_ref()
        .unwrap()
        .agents
        .iter()
        .find(|a| a.id == task.verifier_id)
        .unwrap();
    for retry in &task.verification_retries {
        assert!(
            verifier
                .session_history
                .contains(retry.previous_session.as_ref().unwrap())
        );
    }
    for (id, turn) in &second.session_ledger.turns {
        assert_eq!(&after.session_ledger.turns[id], turn);
    }
    assert_eq!(
        serde_json::to_value(&after.artifacts).unwrap(),
        serde_json::to_value(&first.artifacts).unwrap()
    );
}

#[test]
fn failed_native_verifier_before_scheduler_binding_is_retained_during_recovery() {
    let f = TeamFixture::new("fail_context_once");
    let executed = step(&f, &f.install_team());
    let verifier_id = executed
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap()
        .verifier_id
        .clone();
    let queued = f
        .runtime
        .dispatch(
            &id(),
            Command::SendMessage {
                run_id: executed.id.clone(),
                target_agent_id: verifier_id.clone(),
                text: "Deliver this verifier instruction once".into(),
            },
        )
        .unwrap();
    let message_id = queued
        .communication_ledger
        .messages
        .values()
        .find(|m| m.recipient_agent_id == verifier_id)
        .unwrap()
        .id
        .clone();
    let blocked = step(&f, &queued);
    let before = reconcile_team(&f, &blocked);
    let task = before
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    let prior = before
        .session_ledger
        .sessions
        .values()
        .find(|s| {
            s.scope.task_id.as_deref() == Some(&task.id) && s.scope.role == RoleKind::Verifier
        })
        .unwrap();
    let turn = &before.session_ledger.turns[prior.current_turn_id.as_ref().unwrap()];
    assert_ne!(turn.state, architect::sessions::TurnState::Completed);
    assert!(
        before
            .team
            .as_ref()
            .unwrap()
            .agents
            .iter()
            .find(|a| a.id == task.verifier_id)
            .unwrap()
            .session_id
            .is_none()
    );
    std::fs::write(f.root.path().join("retry-ready"), "ready").unwrap();
    f.runtime.dispatch(&id(), retry_task(&before)).unwrap();
    let after = f.wait_for_idle();
    let recovered = after
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == task.id)
        .unwrap();
    assert_eq!(
        recovered.state,
        TaskState::Verified,
        "{:?}",
        after.diagnostic
    );
    let verifier = after
        .team
        .as_ref()
        .unwrap()
        .agents
        .iter()
        .find(|a| a.id == task.verifier_id)
        .unwrap();
    assert!(verifier.session_history.contains(&prior.id));
    assert_eq!(&after.session_ledger.turns[&turn.id], turn);
    assert_eq!(&after.session_ledger.sessions[&prior.id], prior);
    assert_eq!(recovered.attempt, task.attempt);
    assert_eq!(
        before.communication_ledger.messages[&message_id].state,
        architect::communication::DeliveryState::Unknown
    );
    assert_eq!(
        after.communication_ledger.messages[&message_id],
        before.communication_ledger.messages[&message_id]
    );
    let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    let prompts: Vec<serde_json::Value> = calls
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        prompts
            .iter()
            .filter(|call| call["prompt"]
                .as_str()
                .unwrap()
                .contains("Deliver this verifier instruction once"))
            .count(),
        1
    );
    assert_eq!(
        serde_json::to_value(&after.artifacts).unwrap(),
        serde_json::to_value(&before.artifacts).unwrap()
    );
}

#[test]
fn interrupted_team_verifier_is_blocked_on_restart_and_requires_reconciliation() {
    let f = TeamFixture::new("retry_context_once");
    let executed = step(&f, &f.install_team());
    let blocked = step(&f, &executed);
    let before = reconcile_team(&f, &blocked);
    let mut interrupted = before.clone();
    interrupted.status = Status::Running;
    interrupted.reconciliation_required = true;
    interrupted
        .team
        .as_mut()
        .unwrap()
        .tasks
        .iter_mut()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap()
        .state = TaskState::Verifying;
    f.runtime
        .store
        .lock()
        .unwrap()
        .save(&interrupted, "test_crash_boundary", None)
        .unwrap();
    let reopened =
        Arc::new(Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap());
    let recovered = reopened.store.lock().unwrap().get(&before.id).unwrap();
    assert_eq!(recovered.status, Status::Blocked);
    assert!(recovered.reconciliation_required);
    let task = recovered
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    assert_eq!(task.state, TaskState::Blocked);
    assert_eq!(
        serde_json::to_value(&recovered.artifacts).unwrap(),
        serde_json::to_value(&before.artifacts).unwrap()
    );
    assert!(recovered.task_verification_retry_scope(&task.id).is_err());
    let observation = architect::recovery::observe(&recovered).unwrap();
    let reconciled = reopened
        .dispatch(
            &id(),
            Command::Reconcile {
                run_id: recovered.id.clone(),
                scope_hash: recovered.scope_hash("reconcile"),
                input_hash: observation.hash,
            },
        )
        .unwrap();
    assert!(reconciled.task_verification_retry_scope(&task.id).is_ok());
}

#[test]
fn task_recovery_never_erases_rejections_or_forged_evidence() {
    for mode in [
        "reject",
        "false_criterion",
        "accepted_findings",
        "malformed_negative_findings",
        "missing_evidence",
        "wrong_hash",
        "synthetic_quality",
    ] {
        let f = TeamFixture::new(mode);
        let executed = step(&f, &f.install_team());
        let reviewed = step(&f, &executed);
        let stopped = if reviewed.reconciliation_required {
            reconcile_team(&f, &reviewed)
        } else {
            reviewed
        };
        assert!(
            stopped
                .task_verification_retry_scope("task-contextanalyst")
                .is_err(),
            "{mode}"
        );
    }
}

#[test]
fn daemon_exposes_task_recovery_to_tui_and_cli_without_replaying_executors() {
    use architect::tui::backend::{Action, Backend, IpcBackend};
    let f = TeamFixture::new("retry_context_once");
    let executed = step(&f, &f.install_team());
    let blocked = step(&f, &executed);
    let before = reconcile_team(&f, &blocked);
    std::fs::set_permissions(f.root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let daemon = TestDaemon(
        std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("daemon")
            .arg(f.root.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut backend = IpcBackend::new(f.root.path());
    let deadline = Instant::now() + Duration::from_secs(5);
    let snapshot = loop {
        match backend.snapshot(&before.id) {
            Ok(snapshot) => break snapshot,
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "daemon snapshot unavailable: {error}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    assert_eq!(
        snapshot
            .task_verification_retries
            .get("task-contextanalyst"),
        Some(
            &before
                .task_verification_retry_scope("task-contextanalyst")
                .unwrap()
        )
    );
    assert!(
        backend
            .act(
                &before.id,
                Action::RetryTaskVerification {
                    task_id: "task-contextanalyst".into(),
                    scope_hash: "0".repeat(64),
                }
            )
            .is_err()
    );
    assert_eq!(
        serde_json::to_value(f.current()).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    let calls_before = std::fs::read_to_string(f.root.path().join("calls.jsonl"))
        .unwrap()
        .lines()
        .count();
    std::fs::write(f.root.path().join("retry-ready"), "ready").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("retry-task-verification")
        .arg(f.root.path())
        .arg(&before.id)
        .arg("task-contextanalyst")
        .arg("--approve-scoped")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let after = f.wait_for_idle();
    let task = after
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    assert_eq!(task.state, TaskState::Verified, "{:?}", after.diagnostic);
    let snapshot = backend.snapshot(&before.id).unwrap();
    assert!(
        !snapshot
            .task_verification_retries
            .contains_key("task-contextanalyst")
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls.jsonl"))
            .unwrap()
            .lines()
            .count(),
        calls_before + 1
    );
    assert_eq!(
        serde_json::to_value(&after.artifacts).unwrap(),
        serde_json::to_value(&before.artifacts).unwrap()
    );
    drop(daemon);
}

#[test]
fn malformed_or_contradictory_reviews_never_create_passes() {
    for mode in [
        "missing_criteria",
        "false_criterion",
        "accepted_findings",
        "missing_checks",
        "missing_evidence",
        "wrong_hash",
        "synthetic_quality",
        "malformed_review",
    ] {
        let f = TeamFixture::new(mode);
        let accepted = f.install_team();
        let executed = step(&f, &accepted);
        assert_eq!(
            executed.status,
            Status::Ready,
            "{mode}: {:?}",
            executed.diagnostic
        );
        let reviewed = step(&f, &executed);
        assert_eq!(
            reviewed.status,
            Status::Blocked,
            "{mode}: {:?}",
            reviewed.diagnostic
        );
        assert!(
            reviewed
                .team
                .as_ref()
                .unwrap()
                .tasks
                .iter()
                .all(|t| t.state != TaskState::Verified),
            "{mode}"
        );
        assert!(reviewed.quality_gates.is_empty(), "{mode}");
        assert!(
            reviewed
                .session_ledger
                .turns
                .values()
                .filter_map(|t| t.output.as_ref())
                .any(|o| o.contains("accepted") || mode == "malformed_review"),
            "raw native report must survive: {mode}"
        );
    }
}

#[test]
fn a_gate_requires_its_typed_report_even_when_checks_claim_pass() {
    let f = TeamFixture::new("empty_quality");
    let accepted = f.install_team();
    let executed = step(&f, &accepted);
    let reviewed = step(&f, &executed);
    assert_eq!(reviewed.status, Status::Blocked);
    assert!(!reviewed.quality_gates.contains_key("context_ready:design"));
    let task = reviewed
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    assert_eq!(task.state, TaskState::Blocked);
    assert!(task.diagnostic.as_ref().unwrap().contains("ContextAudit"));
}

#[test]
fn executor_cannot_write_outside_its_scope_or_return_empty_artifacts() {
    for mode in ["escape_edit", "outside_scope", "empty_artifact"] {
        let f = TeamFixture::new(mode);
        let accepted = f.install_team();
        let executed = step(&f, &accepted);
        assert_eq!(executed.status, Status::Blocked, "{mode}");
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("workspace/source.txt")).unwrap(),
            "original input"
        );
        assert!(!f.root.path().join("escape.txt").exists());
        assert!(executed.artifacts.is_empty(), "{mode}");
    }
}

#[test]
fn checkpoint_failure_prevents_provider_work_and_preserves_reconciliation() {
    let f = TeamFixture::new("success");
    let accepted = f.install_team();
    let calls_before = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    let connection = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_team_checkpoint BEFORE INSERT ON events WHEN NEW.kind='team_wave_checkpoint' BEGIN SELECT RAISE(ABORT, 'injected task checkpoint failure'); END;").unwrap();
    let blocked = step(&f, &accepted);
    assert_eq!(blocked.status, Status::Blocked);
    assert!(blocked.reconciliation_required);
    assert!(
        blocked
            .diagnostic
            .as_ref()
            .unwrap()
            .contains("checkpoint failure")
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap(),
        calls_before
    );
    assert!(blocked.artifacts.is_empty());
}

#[test]
fn cancelled_wave_cannot_overwrite_cancelled_status() {
    let f = TeamFixture::new("slow_executor");
    let accepted = f.install_team();
    f.runtime
        .dispatch(
            &id(),
            Command::StepTeam {
                run_id: accepted.id.clone(),
                scope_hash: accepted.team_step_scope().unwrap(),
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f.root.path().join("executor-started").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    f.runtime
        .dispatch(
            &id(),
            Command::Cancel {
                run_id: accepted.id.clone(),
            },
        )
        .unwrap();
    // Acquiring the same lease proves the cancelled worker has released it.
    loop {
        if let Ok(_lease) = architect::recovery::WorkspaceLease::acquire(&accepted.spec.workspace) {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(f.current().status, Status::Cancelled);
    assert!(f.current().reconciliation_required);
}

#[test]
fn task_inbox_is_delivered_and_artifact_checkpoints_survive_reattachment() {
    use architect::communication::DeliveryState;
    let f = TeamFixture::new("success");
    let accepted = f.install_team();
    let queued = f
        .runtime
        .dispatch(
            &id(),
            Command::SendMessage {
                run_id: accepted.id.clone(),
                target_agent_id: "executor-contextanalyst".into(),
                text: "Private context analyst instruction".into(),
            },
        )
        .unwrap();
    let message = queued
        .communication_ledger
        .messages
        .values()
        .find(|m| m.recipient_agent_id == "executor-contextanalyst")
        .unwrap()
        .id
        .clone();
    let executed = step(&f, &queued);
    assert_eq!(
        executed.communication_ledger.messages[&message].state,
        DeliveryState::Delivered
    );
    let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    let mut addressed = 0;
    for line in calls.lines() {
        let call: serde_json::Value = serde_json::from_str(line).unwrap();
        let prompt = call["prompt"].as_str().unwrap();
        if prompt.contains("Private context analyst instruction") {
            addressed += 1;
            assert!(prompt.contains("Task ID: task-contextanalyst"));
        }
    }
    assert_eq!(addressed, 1);
    let reopened = Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap();
    let durable = reopened.store.lock().unwrap().get(&executed.id).unwrap();
    for task in durable
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .filter(|t| t.state == TaskState::AwaitingVerification)
    {
        let reference = task.artifact.as_ref().unwrap();
        let artifact = durable
            .artifacts
            .iter()
            .find(|a| a.id == reference.id)
            .unwrap();
        assert_eq!(architect::hash(&artifact.content), reference.content_hash);
        for evidence in &reference.evidence_refs {
            assert!(
                durable
                    .evidence
                    .iter()
                    .any(|e| e.id == *evidence && e.output_hash == architect::hash(&e.output))
            );
        }
    }
}

#[test]
fn element_comments_reach_orchestrator_and_only_reviewed_reactions_are_recorded() {
    use architect::comments::{self, ElementKind};
    for mode in [
        "success",
        "omit_comment_reaction",
        "invent_comment_task",
        "reject_comment_review",
    ] {
        let f = TeamFixture::new(mode);
        let accepted = f.install_team();
        let target = comments::elements(&accepted)
            .into_iter()
            .find(|e| {
                e.reference.kind == ElementKind::Agent
                    && e.reference.id == "executor-contextanalyst"
            })
            .unwrap()
            .reference;
        let calls_before = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
        let annotated = f
            .runtime
            .dispatch(
                &id(),
                Command::AddElementComment {
                    run_id: accepted.id.clone(),
                    target: target.clone(),
                    text: "Explain why this context analyst is needed".into(),
                },
            )
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap(),
            calls_before,
            "a comment does not silently authorize provider work"
        );
        let comment_id = annotated.element_comments[0].id.clone();
        f.runtime
            .dispatch(
                &id(),
                Command::Orchestrate {
                    run_id: annotated.id.clone(),
                    scope_hash: planning::start_scope(&annotated).unwrap(),
                },
            )
            .unwrap();
        let answered = f.wait_for_idle();
        let comment = &answered.element_comments[0];
        assert!(
            !comment.resolved,
            "a delivered answer is not user resolution"
        );
        if mode == "success" {
            assert_eq!(comment.reactions.len(), 1);
            assert_eq!(comment.reactions[0].response.comment_id, comment_id);
            let round = answered.orchestration_rounds.last().unwrap();
            assert_eq!(round.status, planning::RoundStatus::Reviewed);
            assert!(
                round
                    .request_context
                    .contains("Explain why this context analyst is needed")
            );
            assert!(
                round
                    .review
                    .as_ref()
                    .unwrap()
                    .contains(&format!("COMMENT:{comment_id}"))
            );
        } else {
            assert!(comment.reactions.is_empty(), "{mode}");
            assert_eq!(
                answered.orchestration_rounds.last().unwrap().status,
                planning::RoundStatus::Rejected,
                "{mode}"
            );
        }
        let reopened = Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap();
        let restored = reopened.store.lock().unwrap().get(&answered.id).unwrap();
        assert_eq!(
            restored.element_comments[0].reactions.len(),
            comment.reactions.len()
        );
        let resolved = f
            .runtime
            .dispatch(
                &id(),
                Command::ResolveElementComment {
                    run_id: answered.id.clone(),
                    comment_id: comment_id.clone(),
                    resolved: true,
                },
            )
            .unwrap();
        assert!(resolved.element_comments[0].resolved);
        let deleted = f
            .runtime
            .dispatch(
                &id(),
                Command::DeleteElementComment {
                    run_id: answered.id.clone(),
                    comment_id,
                },
            )
            .unwrap();
        assert!(deleted.element_comments[0].deleted);
        assert_eq!(
            deleted.element_comments[0].target, target,
            "deletion retains exact historical binding"
        );
    }
}

#[test]
fn a_failed_session_checkpoint_stops_sibling_native_turns() {
    let f = TeamFixture::new("checkpoint_stop");
    let accepted = f.install_team();
    let connection = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_session_checkpoint BEFORE INSERT ON events WHEN NEW.kind='team_wave_checkpoint' AND (SELECT COUNT(*) FROM events WHERE kind='team_wave_checkpoint') >= 1 BEGIN SELECT RAISE(ABORT, 'injected session checkpoint failure'); END;").unwrap();
    let began = Instant::now();
    let blocked = step(&f, &accepted);
    assert!(
        began.elapsed() < Duration::from_secs(10),
        "sibling native turns must be cancelled, not allowed to run for their 30s fixture delay"
    );
    assert_eq!(blocked.status, Status::Blocked);
    assert!(blocked.reconciliation_required);
    assert!(
        blocked
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .all(|t| t.state != TaskState::Verified)
    );
    assert!(
        blocked
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .all(|t| !matches!(t.state, TaskState::Running | TaskState::Verifying)),
        "a finished failed wave must not retain apparent active tasks"
    );
    assert!(blocked.quality_gates.is_empty());
    assert!(architect::recovery::WorkspaceLease::acquire(&accepted.spec.workspace).is_ok());
}

#[test]
fn terminal_runs_accept_element_comments_and_reviewed_dialogue_without_reopening_tasks() {
    use architect::comments::{self, ElementKind};
    for status in [Status::Completed, Status::Cancelled] {
        let f = TeamFixture::new("success");
        let mut terminal = f.install_team();
        // Explicit synthetic terminal-state fixture: no claim of complete DAG acceptance.
        terminal.status = status;
        terminal.checklist_version = "historical-task-checklist".into();
        f.runtime
            .store
            .lock()
            .unwrap()
            .save(&terminal, "synthetic_terminal_annotation_fixture", None)
            .unwrap();
        let tasks = serde_json::to_value(&terminal.team.as_ref().unwrap().tasks).unwrap();
        let target = comments::elements(&terminal)
            .into_iter()
            .find(|e| e.reference.kind == ElementKind::Agent)
            .unwrap()
            .reference;
        let commented = f
            .runtime
            .dispatch(
                &id(),
                Command::AddElementComment {
                    run_id: terminal.id.clone(),
                    target,
                    text: "Explain this archived agent".into(),
                },
            )
            .unwrap();
        assert_eq!(commented.status, status);
        f.runtime
            .dispatch(
                &id(),
                Command::Orchestrate {
                    run_id: terminal.id.clone(),
                    scope_hash: planning::start_scope(&commented).unwrap(),
                },
            )
            .unwrap();
        let replied = f.wait_for_idle();
        assert_eq!(replied.status, status);
        assert_eq!(replied.spec_revision, terminal.spec_revision);
        assert_eq!(replied.checklist_version, terminal.checklist_version);
        assert_eq!(
            replied
                .orchestration_rounds
                .last()
                .unwrap()
                .checklist_version,
            checklist_version()
        );
        assert_eq!(
            serde_json::to_value(&replied.team.as_ref().unwrap().tasks).unwrap(),
            tasks
        );
        assert_eq!(replied.element_comments[0].reactions.len(), 1);
        let round = replied.orchestration_rounds.last().unwrap();
        for (agent_id, session) in [
            ("orchestrator", &round.executor_session),
            ("orchestrator:verifier", &round.verifier_session),
        ] {
            let agent = replied
                .team
                .as_ref()
                .unwrap()
                .agents
                .iter()
                .find(|a| a.id == agent_id)
                .unwrap();
            assert_eq!(&agent.session_id, session);
            let previous = terminal
                .team
                .as_ref()
                .unwrap()
                .agents
                .iter()
                .find(|a| a.id == agent_id)
                .unwrap()
                .session_id
                .as_ref()
                .unwrap();
            assert!(agent.session_history.contains(previous));
        }
        assert!(!replied.reconciliation_required);
        let old_proposal: planning::Response =
            serde_json::from_str(terminal.orchestration_rounds[0].output.as_ref().unwrap())
                .unwrap();
        assert!(
            planning::validate_response(&replied, &old_proposal)
                .unwrap_err()
                .to_string()
                .contains("terminal-run dialogue")
        );
        assert!(
            f.runtime
                .dispatch(
                    &id(),
                    Command::StepTeam {
                        run_id: terminal.id.clone(),
                        scope_hash: replied.team_step_scope().unwrap()
                    }
                )
                .is_err()
        );
    }
}

#[test]
fn expected_failed_baseline_is_observed_before_executor_and_reviewed_as_historical() {
    let f = TeamFixture::new("baseline_success");
    let executed = step(&f, &f.install_team());
    let record = &executed.task_checks[0];
    assert!(!record.success);
    assert!(record.artifact_id.is_none());
    assert_eq!(
        record.state,
        architect::runtime::team::checks::CheckState::Completed
    );
    assert_eq!(record.before_hash, record.after_hash.clone().unwrap());
    let task = executed
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    let artifact = executed
        .artifacts
        .iter()
        .find(|a| Some(&a.id) == task.artifact.as_ref().map(|r| &r.id))
        .unwrap();
    let receipt = executed
        .evidence
        .iter()
        .find(|e| Some(&e.id) == record.evidence_id.as_ref())
        .unwrap();
    assert!(architect::verification::evidence_is_current(
        &executed, artifact, receipt
    ));
    let verified = step(&f, &executed);
    let task = verified
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    assert_eq!(task.state, TaskState::Verified, "{:?}", task.diagnostic);
    let review = verified
        .reviews
        .iter()
        .find(|r| r.verifier_session == task.review.as_ref().unwrap().session_id)
        .unwrap();
    let check = review
        .checks
        .iter()
        .find(|c| c.check_id == "BASELINE:reproduction")
        .unwrap();
    assert_eq!(check.status, CheckStatus::Pass);
    assert!(
        check
            .evidence_ids
            .contains(record.evidence_id.as_ref().unwrap())
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/baseline-count")).unwrap(),
        "1"
    );
    let mut corrupted = verified.clone();
    corrupted.task_checks[0]
        .input_snapshot
        .as_mut()
        .unwrap()
        .insert("source.txt".into(), "different original input".into());
    assert!(!architect::verification::evidence_is_current(
        &corrupted, artifact, receipt
    ));
    let mut missing = verified.clone();
    missing.task_checks.clear();
    assert!(!architect::verification::evidence_is_current(
        &missing, artifact, receipt
    ));
}

#[test]
fn unexpected_baseline_exit_blocks_executor_and_preserves_actual_outcome() {
    for mode in [
        "baseline_unexpected_success",
        "baseline_wrong_failure",
        "baseline_signal",
    ] {
        let f = TeamFixture::new(mode);
        let stopped = step(&f, &f.install_team());
        let task = stopped
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == "task-contextanalyst")
            .unwrap();
        assert_eq!(
            task.state,
            TaskState::Blocked,
            "{mode}: {:?}",
            task.diagnostic
        );
        assert!(task.artifact.is_none());
        assert!(
            !stopped
                .session_ledger
                .sessions
                .values()
                .any(|s| s.scope.task_id.as_deref() == Some(&task.id))
        );
        assert_eq!(
            stopped.task_checks[0].state,
            architect::runtime::team::checks::CheckState::Completed
        );
        assert_eq!(
            stopped.task_checks[0].success,
            mode == "baseline_unexpected_success"
        );
        assert_eq!(stopped.input_snapshot["source.txt"], "original input");
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("workspace/baseline-count")).unwrap(),
            "1"
        );
    }
}

#[test]
fn baseline_checkpoint_failure_or_mutation_prevents_all_executor_admission_and_replay() {
    for boundary in [
        "task_check_intent",
        "task_check_output_captured",
        "task_check_finished",
        "input_mutation",
    ] {
        let f = TeamFixture::new(if boundary == "input_mutation" {
            "baseline_mutation"
        } else {
            "baseline_success"
        });
        let installed = f.install_team();
        if boundary != "input_mutation" {
            let connection = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
            connection.execute_batch(&format!("CREATE TRIGGER refuse_baseline BEFORE INSERT ON events WHEN NEW.kind='{boundary}' BEGIN SELECT RAISE(ABORT, 'injected baseline checkpoint failure'); END;")).unwrap();
        }
        let stopped = step(&f, &installed);
        if boundary == "task_check_output_captured" {
            assert!(stopped.task_check_outcomes.is_empty());
            let reconciled = reconcile_team(&f, &stopped);
            assert!(architect::runtime::team::check_recovery::pending(&reconciled).is_empty());
        }
        assert!(stopped.reconciliation_required);
        assert!(stopped.team.as_ref().unwrap().checkpoint_uncertain);
        assert_eq!(
            serde_json::to_value(&stopped.session_ledger).unwrap(),
            serde_json::to_value(&installed.session_ledger).unwrap(),
            "{boundary}: baseline must admit no new native session or turn"
        );
        if boundary == "task_check_intent" {
            assert!(stopped.task_checks.is_empty());
            assert!(!f.root.path().join("workspace/baseline-count").exists());
        } else {
            assert_ne!(
                stopped.task_checks[0].state,
                architect::runtime::team::checks::CheckState::Completed
            );
            assert_eq!(
                std::fs::read_to_string(f.root.path().join("workspace/baseline-count")).unwrap(),
                "1"
            );
        }
        assert!(
            f.runtime
                .dispatch(
                    &id(),
                    Command::StepTeam {
                        run_id: stopped.id.clone(),
                        scope_hash: stopped.team_step_scope().unwrap()
                    }
                )
                .is_err()
        );
        let reopened =
            Arc::new(Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap());
        let durable = reopened.store.lock().unwrap().get(&stopped.id).unwrap();
        assert!(
            reopened
                .dispatch(
                    &id(),
                    Command::StepTeam {
                        run_id: durable.id.clone(),
                        scope_hash: durable.team_step_scope().unwrap()
                    }
                )
                .is_err()
        );
        assert!(durable.team.as_ref().unwrap().checkpoint_uncertain);
        assert_eq!(durable.task_checks.len(), stopped.task_checks.len());
    }
}

#[test]
fn managed_write_retains_expected_failed_baseline_across_read_only_refresh() {
    let f = TeamFixture::new("managed_baseline");
    let changed = managed_change(&f);
    let writer = changed
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.deliverable)
        .unwrap();
    let original = writer.artifact.clone().unwrap();
    let baseline = changed
        .task_checks
        .iter()
        .find(|i| i.task_id == writer.id)
        .unwrap()
        .clone();
    assert!(!baseline.success);
    assert_eq!(
        baseline.input_snapshot.as_ref().unwrap()["source.txt"],
        "original input"
    );
    let ready = refresh_prerequisites(&f, changed.clone(), &writer.id);
    let refreshed = refresh_sources(&f, &ready, &writer.id);
    let verified = step(&f, &refreshed);
    let writer = verified
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == writer.id)
        .unwrap();
    assert_eq!(writer.state, TaskState::Verified, "{:?}", writer.diagnostic);
    assert_ne!(writer.artifact.as_ref().unwrap().id, original.id);
    assert_eq!(
        verified
            .task_checks
            .iter()
            .filter(|i| i.plan.phase == architect::team::checks::CheckPhase::BeforeExecution)
            .count(),
        1
    );
    let positive = verified
        .task_checks
        .iter()
        .find(|i| {
            i.task_id == writer.id
                && i.plan.phase == architect::team::checks::CheckPhase::BeforeVerification
        })
        .unwrap();
    assert!(positive.success);
    assert_ne!(positive.before_hash, baseline.before_hash);
    assert_eq!(
        std::fs::read_to_string(
            f.root
                .path()
                .join(format!("workspace/baseline-{}", writer.id))
        )
        .unwrap(),
        "1"
    );
}

#[test]
fn disjoint_writer_wave_captures_every_baseline_before_any_native_writer_starts() {
    let f = TeamFixture::new("managed_disjoint_baseline");
    let ready = before_managed_change(&f);
    let changed = step(&f, &ready);
    assert_eq!(changed.input_snapshot["source.txt"], "managed result");
    assert_eq!(changed.input_snapshot["second.txt"], "managed result");
    let baselines: Vec<_> = changed
        .task_checks
        .iter()
        .filter(|i| i.plan.phase == architect::team::checks::CheckPhase::BeforeExecution)
        .collect();
    assert_eq!(baselines.len(), 2);
    assert_eq!(baselines[0].before_hash, baselines[1].before_hash);
    for baseline in baselines {
        assert_eq!(
            baseline.state,
            architect::runtime::team::checks::CheckState::Completed
        );
        assert!(!baseline.success);
        assert_eq!(
            baseline.input_snapshot.as_ref().unwrap()["source.txt"],
            "original input"
        );
        assert_eq!(
            baseline.input_snapshot.as_ref().unwrap()["second.txt"],
            "second original"
        );
        assert_eq!(
            std::fs::read_to_string(
                f.root
                    .path()
                    .join(format!("workspace/baseline-{}", baseline.task_id))
            )
            .unwrap(),
            "1"
        );
    }
}

#[test]
fn positive_current_checks_cannot_use_only_historical_expected_failure() {
    let f = TeamFixture::new("baseline_only_evidence");
    let executed = step(&f, &f.install_team());
    let stopped = step(&f, &executed);
    let task = stopped
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    assert_eq!(task.state, TaskState::Blocked, "{:?}", task.diagnostic);
    assert!(
        task.diagnostic
            .as_ref()
            .unwrap()
            .contains("historical baseline alone")
    );
    assert!(!stopped.quality_gates.contains_key("context_ready:design"));
    assert!(!stopped.task_checks[0].success);
}

#[test]
fn cancelled_baseline_preserves_unknown_result_and_starts_no_native_wave_worker() {
    let f = TeamFixture::new("baseline_hang");
    let installed = f.install_team();
    f.runtime
        .dispatch(
            &id(),
            Command::StepTeam {
                run_id: installed.id.clone(),
                scope_hash: installed.team_step_scope().unwrap(),
            },
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f.root.path().join("workspace/baseline-started").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    f.runtime
        .dispatch(
            &id(),
            Command::Cancel {
                run_id: installed.id.clone(),
            },
        )
        .unwrap();
    loop {
        if let Ok(_lease) = architect::recovery::WorkspaceLease::acquire(&installed.spec.workspace)
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let cancelled = f.current();
    assert_eq!(cancelled.status, Status::Cancelled);
    assert_eq!(
        cancelled.task_checks[0].state,
        architect::runtime::team::checks::CheckState::Unknown
    );
    assert!(cancelled.task_checks[0].evidence_id.is_none());
    assert_eq!(
        serde_json::to_value(&cancelled.session_ledger).unwrap(),
        serde_json::to_value(&installed.session_ledger).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/baseline-count")).unwrap(),
        "1"
    );
}

#[test]
fn verifier_replacement_reuses_expected_failed_baseline_and_refuses_missing_original() {
    let f = TeamFixture::new("baseline_retry");
    let executed = step(&f, &f.install_team());
    let blocked = step(&f, &executed);
    let stopped = reconcile_team(&f, &blocked);
    assert!(!stopped.task_checks[0].success);
    stopped
        .task_verification_retry_scope("task-contextanalyst")
        .unwrap();
    let mut missing = stopped.clone();
    missing.task_checks.clear();
    assert!(
        missing
            .task_verification_retry_scope("task-contextanalyst")
            .is_err()
    );
    let mut corrupted = stopped.clone();
    corrupted.task_checks[0]
        .input_snapshot
        .as_mut()
        .unwrap()
        .insert("source.txt".into(), "different original".into());
    assert!(
        corrupted
            .task_verification_retry_scope("task-contextanalyst")
            .is_err()
    );
    std::fs::write(f.root.path().join("retry-ready"), "ready").unwrap();
    f.runtime.dispatch(&id(), retry_task(&stopped)).unwrap();
    let after = f.wait_for_idle();
    let task = after
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    assert_eq!(task.state, TaskState::Verified, "{:?}", task.diagnostic);
    assert_eq!(
        serde_json::to_value(&after.task_checks).unwrap(),
        serde_json::to_value(&stopped.task_checks).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/baseline-count")).unwrap(),
        "1"
    );
}

#[test]
fn bugfix_reproduction_and_reviewed_alternative_preserve_original_and_actual_post_change_proof() {
    for mode in [
        "managed_bugfix_success",
        "managed_bugfix_alternative_success",
        "managed_bugfix_measurement",
    ] {
        let f = TeamFixture::new(mode);
        let changed = managed_change(&f);
        let writer_id = changed
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.deliverable)
            .unwrap()
            .id
            .clone();
        let before = changed
            .task_checks
            .iter()
            .find(|i| i.task_id == writer_id)
            .unwrap()
            .clone();
        assert_eq!(
            before.success,
            mode.contains("alternative") || mode.contains("measurement")
        );
        assert_eq!(
            before.input_snapshot.as_ref().unwrap()["source.txt"],
            "original input"
        );
        let ready = refresh_prerequisites(&f, changed.clone(), &writer_id);
        let refreshed = refresh_sources(&f, &ready, &writer_id);
        let verified = step(&f, &refreshed);
        let writer = verified
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == writer_id)
            .unwrap();
        assert_eq!(
            writer.state,
            TaskState::Verified,
            "{mode}: {:?}",
            writer.diagnostic
        );
        let review = verified
            .reviews
            .iter()
            .find(|r| r.verifier_session == writer.review.as_ref().unwrap().session_id)
            .unwrap();
        let inspected = review
            .quality
            .as_ref()
            .unwrap()
            .change_review
            .as_ref()
            .unwrap();
        assert_eq!(
            inspected.contract_hash,
            architect::team::changes::contract_hash(writer.change_contract.as_ref().unwrap())
        );
        let proof = &inspected.bugfixes[0];
        assert!(
            proof
                .before_evidence_ids
                .contains(before.evidence_id.as_ref().unwrap())
        );
        let after = verified
            .task_checks
            .iter()
            .find(|i| {
                i.task_id == writer_id
                    && i.plan.phase == architect::team::checks::CheckPhase::BeforeVerification
            })
            .unwrap();
        assert!(after.success);
        assert!(
            proof
                .after_evidence_ids
                .contains(after.evidence_id.as_ref().unwrap())
        );
        assert_ne!(after.before_hash, before.before_hash);
        assert!(
            review
                .checks
                .iter()
                .any(|c| c.check_id == "BUGFIX:wrong-value" && c.status == CheckStatus::Pass)
        );
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("workspace/bugfix-before-count")).unwrap(),
            "1"
        );
    }
}

#[test]
fn non_defect_classification_preserves_acceptance_checks_and_actual_original_bugfix_proof() {
    let f = TeamFixture::new("managed_bugfix_nondefect_success");
    let changed = managed_change(&f);
    let task = changed
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.deliverable)
        .unwrap();
    let id = task.id.clone();
    let criterion = task
        .change_contract
        .as_ref()
        .unwrap()
        .non_defect_criteria
        .keys()
        .next()
        .unwrap()
        .clone();
    let baseline = changed
        .task_checks
        .iter()
        .find(|i| {
            i.task_id == id && i.plan.phase == architect::team::checks::CheckPhase::BeforeExecution
        })
        .unwrap()
        .clone();
    assert!(!baseline.plan.criterion_ids.contains(&criterion));
    assert!(!baseline.success);
    let ready = refresh_prerequisites(&f, changed, &id);
    let refreshed = refresh_sources(&f, &ready, &id);
    let verified = step(&f, &refreshed);
    let task = verified
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == id)
        .unwrap();
    assert_eq!(task.state, TaskState::Verified, "{:?}", task.diagnostic);
    assert!(task.criterion_ids.contains(&criterion));
    let review = verified
        .reviews
        .iter()
        .find(|r| r.verifier_session == task.review.as_ref().unwrap().session_id)
        .unwrap();
    let independent = review
        .quality
        .as_ref()
        .unwrap()
        .change_review
        .as_ref()
        .unwrap();
    assert!(
        !independent.non_defect_assessments[&criterion]
            .evidence_ids
            .is_empty()
    );
    assert!(
        independent.bugfixes[0]
            .before_evidence_ids
            .contains(baseline.evidence_id.as_ref().unwrap())
    );
    for expected in [
        &criterion,
        &format!("CHANGE:non_defect:{criterion}"),
        &"BUGFIX:wrong-value".into(),
    ] {
        assert!(
            review
                .checks
                .iter()
                .any(|c| &c.check_id == expected && c.status == CheckStatus::Pass),
            "{expected}"
        );
    }
    let proposal = verified.orchestration_rounds.last().unwrap();
    let plan_review: serde_json::Value =
        serde_json::from_str(proposal.review.as_ref().unwrap()).unwrap();
    assert!(
        plan_review["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |c| c["check_id"] == format!("NON_DEFECT_PLAN:{id}:{criterion}")
                    && c["status"] == "pass"
            )
    );
}

#[test]
fn missing_classification_or_bugfix_before_after_plan_cannot_be_approved_or_installed() {
    for mode in [
        "missing_change_classification",
        "managed_bugfix_missing_before",
        "managed_bugfix_bad_after",
        "managed_bugfix_incomplete_criteria",
        "managed_bugfix_alternative_no_reason",
    ] {
        let f = TeamFixture::new(mode);
        let queued = f
            .runtime
            .dispatch(
                &id(),
                Command::SendMessage {
                    run_id: f.run.id.clone(),
                    target_agent_id: "orchestrator".into(),
                    text: "Propose the team with required defect proof".into(),
                },
            )
            .unwrap();
        f.runtime
            .dispatch(
                &id(),
                Command::Orchestrate {
                    run_id: queued.id.clone(),
                    scope_hash: planning::start_scope(&queued).unwrap(),
                },
            )
            .unwrap();
        let stopped = f.wait_for_idle();
        assert!(stopped.team.is_none(), "{mode}");
        let round = stopped.orchestration_rounds.last().unwrap();
        assert_eq!(
            round.status,
            planning::RoundStatus::Rejected,
            "{mode}: {:?}",
            round.diagnostic
        );
        assert!(
            round.verifier_session.is_none(),
            "{mode}: structurally missing proof must stop before proposal review"
        );
        assert!(!f.root.path().join("workspace/bugfix-before-count").exists());
    }
}

#[test]
fn missing_or_unbound_independent_bugfix_proof_never_accepts_a_current_task() {
    for mode in [
        "managed_bugfix_no_review",
        "managed_bugfix_wrong_contract_hash",
        "managed_bugfix_omit_proof",
        "managed_bugfix_wrong_receipt",
        "managed_bugfix_alternative_no_assessment",
        "managed_bugfix_nondefect_omit",
        "managed_bugfix_nondefect_unbound",
    ] {
        let f = TeamFixture::new(mode);
        let changed = managed_change(&f);
        let writer_id = changed
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.deliverable)
            .unwrap()
            .id
            .clone();
        let ready = refresh_prerequisites(&f, changed, &writer_id);
        let refreshed = refresh_sources(&f, &ready, &writer_id);
        let stopped = step(&f, &refreshed);
        let writer = stopped
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == writer_id)
            .unwrap();
        assert_eq!(
            writer.state,
            TaskState::Blocked,
            "{mode}: {:?}",
            writer.diagnostic
        );
        assert!(
            writer.review.is_none(),
            "{mode}: a positive label cannot manufacture proof"
        );
        let reconciled = reconcile_team(&f, &stopped);
        if matches!(
            mode,
            "managed_bugfix_wrong_contract_hash" | "managed_bugfix_wrong_receipt"
        ) {
            assert!(
                reconciled
                    .task_verification_retry_scope(&writer_id)
                    .is_err(),
                "{mode}: replacement must not erase contradictory proof"
            );
        }

        assert_eq!(
            std::fs::read_to_string(f.root.path().join("workspace/bugfix-before-count")).unwrap(),
            "1"
        );
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("workspace/source.txt")).unwrap(),
            "managed result"
        );
    }
}

struct SuspendedGuardian(Option<libc::pid_t>);
impl SuspendedGuardian {
    fn stop(pid: libc::pid_t) -> Self {
        assert!(pid > 1);
        assert_eq!(unsafe { libc::kill(pid, libc::SIGSTOP) }, 0);
        Self(Some(pid))
    }
    fn resume(&mut self) {
        if let Some(pid) = self.0.take() {
            assert_eq!(unsafe { libc::kill(pid, libc::SIGCONT) }, 0);
        }
    }
}
impl Drop for SuspendedGuardian {
    fn drop(&mut self) {
        if let Some(pid) = self.0.take() {
            // Always let this owned guardian finish cleanup, including assertion failures.
            let _ = unsafe { libc::kill(pid, libc::SIGCONT) };
        }
    }
}

#[test]
fn task_worker_guardians_hold_workspace_after_daemon_death_until_group_cleanup() {
    use architect::ipc::{Operation, Request, call};
    for mode in ["lease_executor", "lease_verifier", "task_check_lease"] {
        let f = TeamFixture::new(mode);
        let installed = f.install_team();
        let ready = if mode == "lease_executor" {
            installed
        } else {
            step(&f, &installed)
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Ok(lease) = architect::recovery::WorkspaceLease::acquire(&ready.spec.workspace) {
                drop(lease);
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        std::fs::set_permissions(f.root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut daemon = TestDaemon(
            std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
                .arg("daemon")
                .arg(f.root.path())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        loop {
            if call(
                f.root.path(),
                &Request {
                    version: architect::ipc::PROTOCOL_VERSION,
                    request_id: id(),
                    idempotency_key: None,
                    operation: Operation::Read {
                        run_id: ready.id.clone(),
                        document: "run".into(),
                        offset: 0,
                        expected_hash: None,
                    },
                },
            )
            .is_ok()
            {
                break;
            }
            assert!(Instant::now() < deadline, "{mode}: daemon did not start");
            std::thread::sleep(Duration::from_millis(10));
        }
        let response = call(
            f.root.path(),
            &Request {
                version: architect::ipc::PROTOCOL_VERSION,
                request_id: id(),
                idempotency_key: Some(id()),
                operation: Operation::Mutate {
                    command: Box::new(Command::StepTeam {
                        run_id: ready.id.clone(),
                        scope_hash: ready.team_step_scope().unwrap(),
                    }),
                },
            },
        )
        .unwrap();
        assert!(response.error.is_none(), "{mode}: {:?}", response.error);
        while !f.root.path().join("guardian-info.json").exists() {
            assert!(
                Instant::now() < deadline,
                "{mode}: owned process did not start"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let info: serde_json::Value = serde_json::from_slice(
            &std::fs::read(f.root.path().join("guardian-info.json")).unwrap(),
        )
        .unwrap();
        let guardian = info["guardian"].as_i64().unwrap() as libc::pid_t;
        let child = info["child"].as_i64().unwrap() as libc::pid_t;
        let mut suspended = SuspendedGuardian::stop(guardian);
        daemon.0.kill().unwrap();
        daemon.0.wait().unwrap();
        assert_eq!(
            unsafe { libc::kill(child, 0) },
            0,
            "owned child must still be alive while cleanup is held"
        );
        assert!(
            architect::recovery::WorkspaceLease::acquire(&ready.spec.workspace).is_err(),
            "{mode}: a task-worker guardian released workspace ownership before cleanup"
        );
        let alias = f.root.path().join("workspace-alias");
        std::os::unix::fs::symlink(&ready.spec.workspace, &alias).unwrap();
        assert!(
            architect::recovery::WorkspaceLease::acquire(alias.to_str().unwrap()).is_err(),
            "{mode}: symlink alias bypassed guardian ownership"
        );
        let nested = std::path::Path::new(&ready.spec.workspace).join("nested");
        std::fs::create_dir(&nested).unwrap();
        assert!(
            architect::recovery::WorkspaceLease::acquire(nested.to_str().unwrap()).is_err(),
            "{mode}: nested workspace bypassed the retained ancestor lease"
        );
        suspended.resume();
        loop {
            if let Ok(lease) = architect::recovery::WorkspaceLease::acquire(&ready.spec.workspace) {
                drop(lease);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{mode}: guardian did not release after cleanup"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            unsafe { libc::kill(child, 0) },
            -1,
            "guardian must reap its direct owned child before releasing"
        );
        let heartbeat = f.root.path().join("workspace/child-heartbeat");
        let bytes = std::fs::read(&heartbeat).unwrap_or_default();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            std::fs::read(&heartbeat).unwrap_or_default(),
            bytes,
            "{mode}: descendant continues effects after cleanup"
        );
        let calls_before_restart = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
        let restarted =
            Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap();
        let recovered = restarted.store.lock().unwrap().get(&ready.id).unwrap();
        assert_eq!(recovered.status, Status::Blocked);
        assert!(recovered.reconciliation_required);
        assert_eq!(
            std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
            calls_before_restart,
            "{mode}: startup must not launch a replacement native turn"
        );
        assert_eq!(
            recovered.artifacts.len(),
            ready.artifacts.len(),
            "{mode}: restart must not replay an executor"
        );
        if mode == "task_check_lease" {
            assert_eq!(
                recovered.task_checks[0].state,
                architect::runtime::team::checks::CheckState::Unknown
            );
            assert!(recovered.task_checks[0].evidence_id.is_none());
            assert_eq!(
                std::fs::read_to_string(f.root.path().join("workspace/runner-count")).unwrap(),
                "1",
                "unknown check must not rerun at startup"
            );
        }
    }
}

fn raw_team_step(f: &TeamFixture, run: &Run) -> Run {
    f.runtime
        .dispatch(
            &id(),
            Command::StepTeam {
                run_id: run.id.clone(),
                scope_hash: run.team_step_scope().unwrap(),
            },
        )
        .unwrap();
    f.wait_for_idle()
}

#[test]
fn supervised_stage_results_need_separate_user_acceptance_before_cross_stage_work() {
    let f = TeamFixture::new("success");
    let executed = raw_team_step(&f, &f.install_team());
    let reviewed = raw_team_step(&f, &executed);
    assert_eq!(
        reviewed.status,
        Status::AwaitingResult,
        "{:?}",
        reviewed.diagnostic
    );
    let packages = architect::runtime::team::stage::pending(&reviewed);
    let package = &packages["discovery"];
    assert_eq!(package.task_ids.len(), 3);
    assert!(reviewed.team_result_approvals.is_empty());
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    let waiting = raw_team_step(&f, &reviewed);
    assert_eq!(waiting.status, Status::AwaitingResult);
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls,
        "no planning turn may start on verifier approval alone"
    );
    assert!(
        waiting
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .filter(|t| t.stage == architect::team::TaskStage::Planning)
            .all(|t| t.attempt == 0)
    );
    let accepted = f
        .runtime
        .dispatch(
            &id(),
            Command::AcceptTeamStage {
                run_id: waiting.id.clone(),
                stage: package.stage,
                task_ids: package.task_ids.clone(),
                scope_hash: package.scope_hash.clone(),
            },
        )
        .unwrap();
    assert_eq!(accepted.status, Status::Ready);
    assert_eq!(accepted.team_result_approvals.len(), 1);
    assert_eq!(accepted.team_result_approvals[0].actor, "local_user");
    assert!(
        accepted
            .approvals
            .iter()
            .any(|a| a.action == "accept_team_stage" && a.scope_hash == package.scope_hash)
    );
    let planned = raw_team_step(&f, &accepted);
    assert!(
        planned
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .any(|t| t.stage == architect::team::TaskStage::Planning && t.attempt == 1)
    );
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::AcceptTeamStage {
                    run_id: planned.id.clone(),
                    stage: package.stage,
                    task_ids: package.task_ids.clone(),
                    scope_hash: package.scope_hash.clone()
                }
            )
            .is_err(),
        "one result package cannot be accepted twice"
    );
}

#[test]
fn stage_result_acceptance_refuses_stale_package_forged_review_and_failed_commit() {
    let f = TeamFixture::new("success");
    let executed = raw_team_step(&f, &f.install_team());
    let reviewed = raw_team_step(&f, &executed);
    let package = architect::runtime::team::stage::pending(&reviewed)["discovery"].clone();
    let command = Command::AcceptTeamStage {
        run_id: reviewed.id.clone(),
        stage: package.stage,
        task_ids: package.task_ids.clone(),
        scope_hash: package.scope_hash.clone(),
    };
    let mut forged = reviewed.clone();
    let task = forged
        .team
        .as_mut()
        .unwrap()
        .tasks
        .iter_mut()
        .find(|t| t.id == package.task_ids[0])
        .unwrap();
    task.review
        .as_mut()
        .unwrap()
        .criteria
        .values_mut()
        .for_each(|v| *v = false);
    assert!(
        architect::runtime::team::stage::scope(&forged, package.stage, &package.task_ids).is_err()
    );
    let mut changed = reviewed.clone();
    changed
        .input_snapshot
        .insert("source.txt".into(), "unobserved edit".into());
    assert!(
        architect::runtime::team::stage::scope(&changed, package.stage, &package.task_ids).is_err()
    );
    for mutation in [
        "missing_native_receipt",
        "corrupt_native_receipt",
        "pinned_output",
        "shared_native_author",
        "review_checklist",
        "configuration",
        "dependency",
        "uncertain_checkpoint",
    ] {
        let mut invalid = reviewed.clone();
        let task = invalid
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == package.task_ids[0])
            .unwrap()
            .clone();
        match mutation {
            "missing_native_receipt" => {
                invalid
                    .evidence
                    .retain(|e| !task.review.as_ref().unwrap().evidence_refs.contains(&e.id));
            }
            "corrupt_native_receipt" => {
                invalid
                    .evidence
                    .iter_mut()
                    .find(|e| task.review.as_ref().unwrap().evidence_refs.contains(&e.id))
                    .unwrap()
                    .output
                    .push_str("tampered");
            }
            "pinned_output" => {
                invalid
                    .artifacts
                    .iter_mut()
                    .find(|a| a.id == task.artifact.as_ref().unwrap().id)
                    .unwrap()
                    .content
                    .push_str("tampered");
            }
            "shared_native_author" => {
                let native = invalid.session_ledger.sessions
                    [&task.artifact.as_ref().unwrap().session_id]
                    .native_session_id
                    .clone();
                invalid
                    .session_ledger
                    .sessions
                    .get_mut(&task.review.as_ref().unwrap().session_id)
                    .unwrap()
                    .native_session_id = native;
            }
            "review_checklist" => {
                invalid
                    .reviews
                    .iter_mut()
                    .find(|r| r.verifier_session == task.review.as_ref().unwrap().session_id)
                    .unwrap()
                    .checklist_version = "old-checklist".into();
            }
            "configuration" => {
                invalid.spec.config.as_mut().unwrap().content_hash = "0".repeat(64);
            }
            "dependency" => {
                invalid
                    .team
                    .as_mut()
                    .unwrap()
                    .tasks
                    .iter_mut()
                    .find(|t| t.id == task.id)
                    .unwrap()
                    .artifact
                    .as_mut()
                    .unwrap()
                    .dependency_hashes
                    .insert("invented".into(), "0".repeat(64));
            }
            "uncertain_checkpoint" => {
                invalid.team.as_mut().unwrap().checkpoint_uncertain = true;
            }
            _ => unreachable!(),
        }
        assert!(
            architect::runtime::team::stage::scope(&invalid, package.stage, &package.task_ids)
                .is_err(),
            "{mutation} cannot produce an acceptable package"
        );
    }
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    let connection = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_result BEFORE INSERT ON events WHEN NEW.kind='team_stage_result_accepted' BEGIN SELECT RAISE(ABORT, 'injected acceptance failure'); END;").unwrap();
    assert!(f.runtime.dispatch(&id(), command.clone()).is_err());
    assert_eq!(
        serde_json::to_value(f.current()).unwrap(),
        serde_json::to_value(&reviewed).unwrap()
    );
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    connection
        .execute_batch("DROP TRIGGER refuse_result;")
        .unwrap();
    std::fs::write(f.root.path().join("workspace/source.txt"), "external edit").unwrap();
    assert!(f.runtime.dispatch(&id(), command).is_err());
    assert!(f.current().team_result_approvals.is_empty());
}

#[test]
fn partial_stage_acceptance_survives_daemon_restart_and_cli_accepts_only_remaining_results() {
    use architect::tui::backend::{Action, Backend, IpcBackend};
    let f = TeamFixture::new("success");
    let executed = raw_team_step(&f, &f.install_team());
    let reviewed = raw_team_step(&f, &executed);
    let package = architect::runtime::team::stage::pending(&reviewed)["discovery"].clone();
    let selected = vec![package.task_ids[0].clone()];
    let selected_scope =
        architect::runtime::team::stage::scope(&reviewed, package.stage, &selected).unwrap();
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    std::fs::set_permissions(f.root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let start_daemon = || {
        TestDaemon(
            std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
                .arg("daemon")
                .arg(f.root.path())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    let daemon = start_daemon();
    let mut backend = IpcBackend::new(f.root.path());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match backend.snapshot(&reviewed.id) {
            Ok(snapshot) => {
                assert_eq!(
                    snapshot.team_stage_results["discovery"].scope_hash,
                    package.scope_hash
                );
                break;
            }
            Err(error) => {
                assert!(Instant::now() < deadline, "daemon unavailable: {error}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    let read = |operation| {
        architect::ipc::call(
            f.root.path(),
            &architect::ipc::Request {
                version: architect::ipc::PROTOCOL_VERSION,
                request_id: id(),
                idempotency_key: None,
                operation,
            },
        )
        .unwrap()
    };
    let control = read(architect::ipc::Operation::Snapshot {
        run_id: reviewed.id.clone(),
    })
    .data
    .unwrap();
    assert_eq!(
        control["team_stage_results"]["discovery"]["task_count"],
        package.task_ids.len()
    );
    assert!(
        control["team_stage_results"]["discovery"]
            .get("results")
            .is_none()
    );
    let document: serde_json::Value = serde_json::from_str(
        &architect::ipc::read_document(f.root.path(), &reviewed.id, "stage-results").unwrap(),
    )
    .unwrap();
    assert_eq!(document["run_document_hash"], control["document_hash"]);
    assert_eq!(
        document["packages"]["discovery"]["results"]
            .as_array()
            .unwrap()
            .len(),
        package.task_ids.len()
    );
    let page = read(architect::ipc::Operation::Read {
        run_id: reviewed.id.clone(),
        document: "stage-results".into(),
        offset: 0,
        expected_hash: None,
    })
    .data
    .unwrap();
    backend
        .act(
            &reviewed.id,
            Action::AcceptTeamStage {
                stage: package.stage,
                task_ids: selected.clone(),
                scope_hash: selected_scope.clone(),
            },
        )
        .unwrap();
    let stale_page = read(architect::ipc::Operation::Read {
        run_id: reviewed.id.clone(),
        document: "stage-results".into(),
        offset: 0,
        expected_hash: Some(page["hash"].as_str().unwrap().into()),
    });
    assert!(
        stale_page
            .error
            .as_ref()
            .is_some_and(|e| e.contains("document changed")),
        "package pagination cannot mix pre- and post-acceptance results"
    );
    let partial = f.current();
    assert_eq!(partial.status, Status::AwaitingResult);
    assert_eq!(partial.team_result_approvals.len(), 1);
    assert_eq!(partial.team_result_approvals[0].results.len(), 1);
    let remaining = backend.snapshot(&reviewed.id).unwrap().team_stage_results["discovery"].clone();
    assert_eq!(remaining.task_ids.len(), package.task_ids.len() - 1);
    assert!(!remaining.task_ids.contains(&selected[0]));
    assert!(
        backend
            .act(
                &reviewed.id,
                Action::AcceptTeamStage {
                    stage: package.stage,
                    task_ids: package.task_ids.clone(),
                    scope_hash: package.scope_hash.clone()
                }
            )
            .is_err()
    );
    assert!(
        backend
            .act(
                &reviewed.id,
                Action::StepTeam {
                    scope_hash: reviewed.team_step_scope().unwrap()
                }
            )
            .is_err(),
        "acceptance changes wave authorization"
    );
    drop(daemon);
    let daemon = start_daemon();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match backend.snapshot(&reviewed.id) {
            Ok(snapshot) => {
                assert_eq!(
                    snapshot.team_stage_results["discovery"].scope_hash,
                    remaining.scope_hash
                );
                break;
            }
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "restarted daemon unavailable: {error}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    let no_approval = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("accept-team-stage")
        .arg(f.root.path())
        .arg(&reviewed.id)
        .arg("discovery")
        .output()
        .unwrap();
    assert!(!no_approval.status.success());
    assert_eq!(f.current().team_result_approvals.len(), 1);
    let approved = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("accept-team-stage")
        .arg(f.root.path())
        .arg(&reviewed.id)
        .arg("discovery")
        .arg("--approve-scoped")
        .output()
        .unwrap();
    assert!(
        approved.status.success(),
        "{}",
        String::from_utf8_lossy(&approved.stderr)
    );
    let accepted = f.current();
    assert_eq!(accepted.status, Status::Ready);
    assert_eq!(accepted.team_result_approvals.len(), 2);
    assert!(
        backend
            .snapshot(&reviewed.id)
            .unwrap()
            .team_stage_results
            .is_empty()
    );
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls,
        "result acceptance and restart never launch provider work"
    );
    drop(daemon);
    let reopened = Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap();
    let persisted = reopened.store.lock().unwrap().get(&reviewed.id).unwrap();
    assert_eq!(persisted.team_result_approvals.len(), 2);
    assert!(architect::runtime::team::stage::pending(&persisted).is_empty());
}

fn missing_executor_checkpoint(f: &TeamFixture, installed: &Run, task_id: &str) -> Run {
    let connection = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    connection.execute_batch(&format!("CREATE TRIGGER refuse_executor_result BEFORE UPDATE ON runs WHEN EXISTS (SELECT 1 FROM json_each(NEW.state, '$.team.tasks') WHERE json_extract(value, '$.id')='{task_id}' AND json_extract(value, '$.state')='awaiting_verification') BEGIN SELECT RAISE(ABORT, 'injected executor-result checkpoint failure'); END;")).unwrap();
    let stopped = raw_team_step(f, installed);
    assert_eq!(stopped.status, Status::Blocked, "{:?}", stopped.diagnostic);
    assert!(stopped.reconciliation_required);
    let task = stopped
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == task_id)
        .unwrap();
    assert_eq!(task.state, TaskState::Blocked);
    assert!(task.artifact.is_none());
    assert!(task.review.is_none());
    assert!(stopped.task_executions.iter().any(|e| e.task_id == task_id
        && e.attempt == task.attempt
        && e.state == architect::runtime::team::effects::ExecutionState::Applied));
    connection
        .execute_batch("DROP TRIGGER refuse_executor_result;")
        .unwrap();
    reconcile_team(f, &stopped)
}

#[test]
fn restoring_completed_executor_result_never_repeats_native_turn_and_still_needs_verifier() {
    let f = TeamFixture::new("restore_single_baseline");
    let before = missing_executor_checkpoint(&f, &f.install_team(), "task-contextanalyst");
    let task = before
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    let attempt = task.attempt;
    let execution = before
        .task_executions
        .iter()
        .find(|e| e.task_id == task.id && e.attempt == task.attempt)
        .unwrap();
    let native = before.session_ledger.sessions[&execution.session_id].clone();
    let raw = before.session_ledger.turns[native.current_turn_id.as_ref().unwrap()]
        .output
        .clone()
        .unwrap();
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    let mut missing_baseline = before.clone();
    missing_baseline.task_checks.clear();
    assert!(
        missing_baseline
            .task_result_restore_scope(&task.id)
            .is_err(),
        "missing original baseline cannot be reconstructed by restoration"
    );
    let mut unknown_baseline = before.clone();
    unknown_baseline.task_checks[0].state = architect::runtime::team::checks::CheckState::Unknown;
    assert!(
        unknown_baseline
            .task_result_restore_scope(&task.id)
            .is_err(),
        "unknown baseline outcome must not become a known result"
    );
    let scope = before.task_result_restore_scope(&task.id).unwrap();
    let command = Command::RestoreTaskResult {
        run_id: before.id.clone(),
        task_id: task.id.clone(),
        scope_hash: scope,
    };
    let key = id();
    let restored = f.runtime.dispatch(&key, command.clone()).unwrap();
    let repeated = f.runtime.dispatch(&key, command).unwrap();
    assert_eq!(
        serde_json::to_value(&restored).unwrap(),
        serde_json::to_value(&repeated).unwrap()
    );
    assert_eq!(restored.status, Status::Ready, "{:?}", restored.diagnostic);
    assert!(!restored.team.as_ref().unwrap().checkpoint_uncertain);
    let task = restored
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    assert_eq!(task.state, TaskState::AwaitingVerification);
    assert_eq!(task.attempt, attempt);
    assert!(task.review.is_none());
    assert_eq!(restored.artifacts.last().unwrap().content, raw);
    assert_eq!(restored.artifacts.last().unwrap().session_id, native.id);
    assert_eq!(
        serde_json::to_value(&restored.session_ledger).unwrap(),
        serde_json::to_value(&before.session_ledger).unwrap()
    );
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/baseline-count")).unwrap(),
        "1"
    );
    assert!(restored.quality_gates.is_empty());
    assert!(restored.team_result_approvals.is_empty());
    assert_eq!(restored.task_result_restorations.len(), 1);
    assert_eq!(restored.task_result_restorations[0].actor, "local_user");
    assert_eq!(
        restored.task_result_restorations[0].turn_id,
        native.current_turn_id.unwrap()
    );
    let reviewed = raw_team_step(&f, &restored);
    let task = reviewed
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == "task-contextanalyst")
        .unwrap();
    assert_eq!(task.state, TaskState::Verified, "{:?}", reviewed.diagnostic);
    assert!(task.review.as_ref().unwrap().accepted);
    assert_eq!(reviewed.task_result_restorations.len(), 1);
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/baseline-count")).unwrap(),
        "1",
        "independent verification must retain the original baseline rather than rerun it"
    );
    assert!(
        reviewed.team_result_approvals.is_empty(),
        "verification must not auto-accept restored stage results"
    );
}

#[test]
fn executor_result_restore_refuses_corruption_uncertainty_stale_scope_and_failed_commit() {
    let f = TeamFixture::new("restore_single");
    let before = missing_executor_checkpoint(&f, &f.install_team(), "task-contextanalyst");
    let scope = before
        .task_result_restore_scope("task-contextanalyst")
        .unwrap();
    let mut ambiguous = before.clone();
    ambiguous.task_executions.push(
        ambiguous
            .task_executions
            .iter()
            .find(|e| e.task_id == "task-contextanalyst")
            .unwrap()
            .clone(),
    );
    assert!(
        ambiguous
            .task_result_restore_scope("task-contextanalyst")
            .is_err(),
        "two retained execution candidates cannot be guessed into one outcome"
    );
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    for mutation in [
        "missing_output",
        "native_unobserved",
        "original_inputs",
        "uncertain_effect",
        "original_stage",
        "configuration",
    ] {
        let mut invalid = before.clone();
        let execution = invalid
            .task_executions
            .iter_mut()
            .find(|e| e.task_id == "task-contextanalyst")
            .unwrap();
        match mutation {
            "missing_output" => {
                let turn = invalid.session_ledger.sessions[&execution.session_id]
                    .current_turn_id
                    .as_ref()
                    .unwrap()
                    .clone();
                invalid.session_ledger.turns.get_mut(&turn).unwrap().output = None;
            }
            "native_unobserved" => {
                invalid
                    .session_ledger
                    .sessions
                    .get_mut(&execution.session_id)
                    .unwrap()
                    .native_observed = false
            }
            "original_inputs" => {
                execution
                    .input_snapshot
                    .insert("source.txt".into(), "invented input".into());
            }
            "uncertain_effect" => {
                execution.state = architect::runtime::team::effects::ExecutionState::Intent
            }
            "original_stage" => {
                invalid
                    .session_ledger
                    .sessions
                    .get_mut(&execution.session_id)
                    .unwrap()
                    .scope
                    .stage = Stage::Delivery
            }
            "configuration" => invalid.spec.config.as_mut().unwrap().content_hash = "0".repeat(64),
            _ => unreachable!(),
        }
        assert!(
            invalid
                .task_result_restore_scope("task-contextanalyst")
                .is_err(),
            "{mutation}"
        );
    }
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::RestoreTaskResult {
                    run_id: before.id.clone(),
                    task_id: "task-contextanalyst".into(),
                    scope_hash: "0".repeat(64)
                }
            )
            .is_err()
    );
    let connection = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_restore BEFORE INSERT ON events WHEN NEW.kind='task_executor_result_restored' BEGIN SELECT RAISE(ABORT, 'injected restoration failure'); END;").unwrap();
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::RestoreTaskResult {
                    run_id: before.id.clone(),
                    task_id: "task-contextanalyst".into(),
                    scope_hash: scope.clone()
                }
            )
            .is_err()
    );
    assert_eq!(
        serde_json::to_value(f.current()).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    connection
        .execute_batch("DROP TRIGGER refuse_restore;")
        .unwrap();
    std::fs::write(f.root.path().join("workspace/source.txt"), "external edit").unwrap();
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::RestoreTaskResult {
                    run_id: before.id.clone(),
                    task_id: "task-contextanalyst".into(),
                    scope_hash: scope
                }
            )
            .is_err()
    );
    assert!(f.current().task_result_restorations.is_empty());
}

#[test]
fn restoring_applied_writer_keeps_original_effect_and_requires_current_source_revalidation() {
    let f = TeamFixture::new("managed_success");
    let ready = before_managed_change(&f);
    let task_id = ready
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.deliverable)
        .unwrap()
        .id
        .clone();
    let before = missing_executor_checkpoint(&f, &ready, &task_id);
    let execution = before
        .task_executions
        .iter()
        .find(|e| e.task_id == task_id)
        .unwrap();
    assert_eq!(execution.input_snapshot["source.txt"], "original input");
    assert_eq!(before.input_snapshot["source.txt"], "managed result");
    assert_eq!(execution.receipts.len(), 1);
    let receipts = serde_json::to_value(&execution.receipts).unwrap();
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    let restored = f
        .runtime
        .dispatch(
            &id(),
            Command::RestoreTaskResult {
                run_id: before.id.clone(),
                task_id: task_id.clone(),
                scope_hash: before.task_result_restore_scope(&task_id).unwrap(),
            },
        )
        .unwrap();
    let task = restored
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == task_id)
        .unwrap();
    assert_eq!(task.state, TaskState::Stale);
    assert_eq!(restored.status, Status::Blocked);
    assert!(!restored.reconciliation_required);
    assert!(!restored.team.as_ref().unwrap().checkpoint_uncertain);
    assert!(task.review.is_none());
    let committed = restored
        .task_executions
        .iter()
        .find(|e| e.task_id == task_id)
        .unwrap();
    assert_eq!(
        committed.state,
        architect::runtime::team::effects::ExecutionState::Committed
    );
    assert_eq!(serde_json::to_value(&committed.receipts).unwrap(), receipts);
    assert_eq!(committed.input_snapshot["source.txt"], "original input");
    assert_eq!(
        restored.artifacts.last().unwrap().session_id,
        execution.session_id
    );
    assert_eq!(
        restored.artifacts.last().unwrap().hash,
        execution.artifact_hash
    );
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/source.txt")).unwrap(),
        "managed result"
    );
    assert!(
        restored
            .task_source_refresh_scope("task-contextanalyst")
            .is_ok()
    );
    assert!(
        restored.task_source_refresh_scope(&task_id).is_err(),
        "writer refresh must wait for independently current prerequisites"
    );
}

#[test]
fn completed_executor_restoration_is_exposed_through_daemon_tui_and_cli_without_replay() {
    use architect::tui::backend::{Backend, IpcBackend};
    let f = TeamFixture::new("restore_single");
    let before = missing_executor_checkpoint(&f, &f.install_team(), "task-contextanalyst");
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    std::fs::set_permissions(f.root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let daemon = TestDaemon(
        std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("daemon")
            .arg(f.root.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut backend = IpcBackend::new(f.root.path());
    let deadline = Instant::now() + Duration::from_secs(5);
    let snapshot = loop {
        match backend.snapshot(&before.id) {
            Ok(snapshot) => break snapshot,
            Err(error) => {
                assert!(Instant::now() < deadline, "daemon unavailable: {error}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    assert_eq!(
        snapshot.task_result_restores["task-contextanalyst"],
        before
            .task_result_restore_scope("task-contextanalyst")
            .unwrap()
    );
    assert!(
        snapshot
            .task_result_restore_details
            .values()
            .any(|text| text
                .contains("Independent verification or source refresh is still required"))
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("restore-task-result")
        .arg(f.root.path())
        .arg(&before.id)
        .arg("task-contextanalyst")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(f.current().task_result_restorations.is_empty());
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("restore-task-result")
        .arg(f.root.path())
        .arg(&before.id)
        .arg("task-contextanalyst")
        .arg("--approve-scoped")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let after = f.current();
    assert_eq!(after.task_result_restorations.len(), 1);
    assert!(
        backend
            .snapshot(&after.id)
            .unwrap()
            .task_result_restores
            .is_empty()
    );
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    assert_eq!(
        serde_json::to_value(&after.session_ledger).unwrap(),
        serde_json::to_value(&before.session_ledger).unwrap()
    );
    drop(daemon);
    let reopened = Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap();
    let durable = reopened.store.lock().unwrap().get(&before.id).unwrap();
    assert_eq!(durable.task_result_restorations.len(), 1);
    assert_eq!(
        durable.artifacts.last().unwrap().hash,
        after.artifacts.last().unwrap().hash
    );
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
}

#[test]
fn restoring_one_completed_executor_keeps_uncertainty_for_incomplete_siblings() {
    let f = TeamFixture::new("checkpoint_stop");
    let before = missing_executor_checkpoint(&f, &f.install_team(), "task-contextanalyst");
    assert!(
        before
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .any(|t| t.id != "task-contextanalyst"
                && t.state == TaskState::Blocked
                && t.artifact.is_none())
    );
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    let restored = f
        .runtime
        .dispatch(
            &id(),
            Command::RestoreTaskResult {
                run_id: before.id.clone(),
                task_id: "task-contextanalyst".into(),
                scope_hash: before
                    .task_result_restore_scope("task-contextanalyst")
                    .unwrap(),
            },
        )
        .unwrap();
    assert_eq!(restored.task_result_restorations.len(), 1);
    assert!(
        restored.team.as_ref().unwrap().checkpoint_uncertain,
        "one known result cannot settle incomplete sibling outcomes"
    );
    assert_eq!(restored.status, Status::Blocked);
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    for sibling in before
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .filter(|t| t.id != "task-contextanalyst")
    {
        let retained = restored
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == sibling.id)
            .unwrap();
        assert_eq!(
            serde_json::to_value(retained).unwrap(),
            serde_json::to_value(sibling).unwrap()
        );
    }
    assert_eq!(
        serde_json::to_value(&restored.session_ledger).unwrap(),
        serde_json::to_value(&before.session_ledger).unwrap()
    );
}

fn interrupted_owned_write(f: &TeamFixture) -> (Run, String) {
    let ready = before_managed_change(f);
    let task_id = ready
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.deliverable)
        .unwrap()
        .id
        .clone();
    let connection = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_write_receipt BEFORE INSERT ON events WHEN NEW.kind='task_execution_edit_applied' BEGIN SELECT RAISE(ABORT, 'injected after-write receipt failure'); END;").unwrap();
    let stopped = raw_team_step(f, &ready);
    connection
        .execute_batch("DROP TRIGGER refuse_write_receipt;")
        .unwrap();
    assert_eq!(stopped.status, Status::Blocked);
    assert!(stopped.reconciliation_required);
    let execution = stopped
        .task_executions
        .iter()
        .find(|e| e.task_id == task_id)
        .unwrap();
    assert_eq!(
        execution.state,
        architect::runtime::team::effects::ExecutionState::Intent
    );
    assert!(execution.after_snapshot.is_none());
    assert_eq!(stopped.input_snapshot["source.txt"], "original input");
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/source.txt")).unwrap(),
        "managed result"
    );
    (stopped, task_id)
}

#[test]
fn uncertain_write_after_state_is_observed_without_replay_or_relabeling_original_sources() {
    use architect::runtime::team::effects::ExecutionState;
    let f = TeamFixture::new("managed_success");
    let (before, task_id) = interrupted_owned_write(&f);
    let original = before
        .task_executions
        .iter()
        .find(|e| e.task_id == task_id)
        .unwrap();
    let original_receipts = serde_json::to_value(&original.receipts).unwrap();
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    let scope = before.task_effect_reconciliation_scope(&task_id).unwrap();
    let command = Command::ReconcileTaskEffects {
        run_id: before.id.clone(),
        task_id: task_id.clone(),
        scope_hash: scope.clone(),
    };
    let key = id();
    let observed = f.runtime.dispatch(&key, command.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(&observed).unwrap(),
        serde_json::to_value(f.runtime.dispatch(&key, command).unwrap()).unwrap()
    );
    assert_eq!(observed.spec_revision, before.spec_revision);
    assert!(observed.team.is_some());
    assert!(!observed.reconciliation_required);
    assert!(observed.pending_edits.is_empty());
    assert_eq!(
        observed.team.as_ref().unwrap().checkpoint_uncertain,
        before.team.as_ref().unwrap().checkpoint_uncertain,
        "effect observation must not clear or manufacture scheduler uncertainty"
    );
    let execution = observed
        .task_executions
        .iter()
        .find(|e| e.task_id == task_id)
        .unwrap();
    assert_eq!(execution.state, ExecutionState::ObservedApplied);
    assert_eq!(execution.input_snapshot["source.txt"], "original input");
    assert_eq!(execution.source_hashes, original.source_hashes);
    assert_eq!(
        execution.after_snapshot.as_ref().unwrap()["source.txt"],
        "managed result"
    );
    let receipt = execution.reconciliation.as_ref().unwrap();
    assert_eq!(receipt.actor, "local_user");
    assert_eq!(receipt.scope_hash, scope);
    assert_eq!(
        serde_json::to_value(&receipt.original_receipts).unwrap(),
        original_receipts
    );
    assert_eq!(observed.input_snapshot["source.txt"], "managed result");
    assert_eq!(
        serde_json::to_value(&observed.session_ledger).unwrap(),
        serde_json::to_value(&before.session_ledger).unwrap()
    );
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    for mutation in ["receipt", "approval", "evidence"] {
        let mut invalid = observed.clone();
        match mutation {
            "receipt" => {
                invalid
                    .task_executions
                    .iter_mut()
                    .find(|e| e.task_id == task_id)
                    .unwrap()
                    .reconciliation = None
            }
            "approval" => invalid
                .approvals
                .retain(|a| a.action != "reconcile_task_effects"),
            "evidence" => invalid
                .evidence
                .retain(|e| !e.source.starts_with("runtime:observed-task-effects:")),
            _ => unreachable!(),
        }
        assert!(
            invalid.task_result_restore_scope(&task_id).is_err(),
            "{mutation} cannot authorize restored observed effects"
        );
    }
    let restored = f
        .runtime
        .dispatch(
            &id(),
            Command::RestoreTaskResult {
                run_id: observed.id.clone(),
                task_id: task_id.clone(),
                scope_hash: observed.task_result_restore_scope(&task_id).unwrap(),
            },
        )
        .unwrap();
    assert_eq!(restored.task_result_restorations.len(), 1);
    let execution = restored
        .task_executions
        .iter()
        .find(|e| e.task_id == task_id)
        .unwrap();
    assert_eq!(execution.state, ExecutionState::Committed);
    assert!(
        execution.reconciliation.is_some(),
        "observation origin must survive result restoration"
    );
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
        TaskState::Stale
    );
    assert!(!restored.reconciliation_required);
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/source.txt")).unwrap(),
        "managed result"
    );
}

#[test]
fn observed_effect_adjudication_refuses_unapplied_conflicting_stale_and_uncommitted_states() {
    let f = TeamFixture::new("managed_other_input");
    let (before, task_id) = interrupted_owned_write(&f);
    let scope = before.task_effect_reconciliation_scope(&task_id).unwrap();
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    for text in ["original input", "conflicting user content"] {
        std::fs::write(f.root.path().join("workspace/source.txt"), text).unwrap();
        assert!(
            before.task_effect_reconciliation_scope(&task_id).is_err(),
            "{text}: observation must not silently apply pending writes"
        );
        assert!(
            f.runtime
                .dispatch(
                    &id(),
                    Command::ReconcileTaskEffects {
                        run_id: before.id.clone(),
                        task_id: task_id.clone(),
                        scope_hash: scope.clone()
                    }
                )
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("workspace/source.txt")).unwrap(),
            text
        );
    }
    std::fs::write(f.root.path().join("workspace/source.txt"), "managed result").unwrap();
    std::fs::write(
        f.root.path().join("workspace/second.txt"),
        "unowned user edit",
    )
    .unwrap();
    assert!(
        before.task_effect_reconciliation_scope(&task_id).is_err(),
        "an unrelated pinned input cannot be adopted as this task's effect"
    );
    std::fs::write(
        f.root.path().join("workspace/second.txt"),
        "second original",
    )
    .unwrap();
    let mut invalid = before.clone();
    invalid.pending_edits.clear();
    assert!(invalid.task_effect_reconciliation_scope(&task_id).is_err());
    let mut invalid = before.clone();
    invalid
        .task_executions
        .iter_mut()
        .find(|e| e.task_id == task_id)
        .unwrap()
        .receipts[0]
        .after_hash = "0".repeat(64);
    assert!(invalid.task_effect_reconciliation_scope(&task_id).is_err());
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::ReconcileTaskEffects {
                    run_id: before.id.clone(),
                    task_id: task_id.clone(),
                    scope_hash: "0".repeat(64)
                }
            )
            .is_err()
    );
    let connection = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_observation BEFORE INSERT ON events WHEN NEW.kind='task_effects_observed' BEGIN SELECT RAISE(ABORT, 'injected observed-effect transaction failure'); END;").unwrap();
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::ReconcileTaskEffects {
                    run_id: before.id.clone(),
                    task_id,
                    scope_hash: scope
                }
            )
            .is_err()
    );
    assert_eq!(
        serde_json::to_value(f.current()).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/source.txt")).unwrap(),
        "managed result"
    );
}

#[test]
fn observed_task_effects_are_exposed_through_daemon_tui_cli_and_survive_restart() {
    use architect::tui::backend::{Backend, IpcBackend};
    let f = TeamFixture::new("managed_success");
    let (before, task_id) = interrupted_owned_write(&f);
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    std::fs::set_permissions(f.root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let daemon = TestDaemon(
        std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("daemon")
            .arg(f.root.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut backend = IpcBackend::new(f.root.path());
    let deadline = Instant::now() + Duration::from_secs(5);
    let snapshot = loop {
        match backend.snapshot(&before.id) {
            Ok(snapshot) => break snapshot,
            Err(error) => {
                assert!(Instant::now() < deadline, "daemon unavailable: {error}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    assert_eq!(
        snapshot.task_effect_reconciliations[&task_id],
        before.task_effect_reconciliation_scope(&task_id).unwrap()
    );
    assert!(
        snapshot
            .task_effect_reconciliation_details
            .values()
            .any(|text| text.contains("without claiming who performed the write"))
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("reconcile-task-effects")
        .arg(f.root.path())
        .arg(&before.id)
        .arg(&task_id)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(f.current().reconciliation_required);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("reconcile-task-effects")
        .arg(f.root.path())
        .arg(&before.id)
        .arg(&task_id)
        .arg("--approve-scoped")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let observed = f.current();
    assert!(!observed.reconciliation_required);
    let after = backend.snapshot(&before.id).unwrap();
    assert!(after.task_effect_reconciliations.is_empty());
    assert!(after.task_result_restores.contains_key(&task_id));
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/source.txt")).unwrap(),
        "managed result"
    );
    drop(daemon);
    let restarted = Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap();
    let retained = restarted.store.lock().unwrap().get(&before.id).unwrap();
    assert_eq!(retained.spec_revision, before.spec_revision);
    assert!(
        retained
            .task_executions
            .iter()
            .find(|e| e.task_id == task_id)
            .unwrap()
            .reconciliation
            .is_some()
    );
    assert!(retained.task_result_restore_scope(&task_id).is_ok());
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
}

fn two_missing_writer_receipts(f: &TeamFixture) -> (Run, Vec<String>) {
    let ready = before_managed_change(f);
    let applied = raw_team_step(f, &ready);
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/source.txt")).unwrap(),
        "managed result"
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/second.txt")).unwrap(),
        "managed result"
    );
    let mut interrupted = applied.clone();
    let writers = interrupted
        .task_executions
        .iter()
        .filter(|e| !e.receipts.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(writers.len(), 2);
    assert_eq!(writers[0].wave_attempt, writers[1].wave_attempt);
    interrupted.input_snapshot = writers[0].input_snapshot.clone();
    interrupted.pending_edits.clear();
    let mut ids = vec![];
    // Fault model: retain the actual native completions, captured originals and
    // physically completed writes, but restore both saved intents before their
    // receipt/outcome checkpoints. No session/output/after-state is invented.
    for writer in &writers {
        ids.push(writer.task_id.clone());
        let execution = interrupted
            .task_executions
            .iter_mut()
            .find(|e| e.id == writer.id)
            .unwrap();
        execution.state = architect::runtime::team::effects::ExecutionState::Intent;
        execution.after_snapshot = None;
        execution.reconciliation = None;
        for receipt in &mut execution.receipts {
            receipt.state = architect::recovery::EditState::Pending;
        }
        interrupted.pending_edits.extend(execution.receipts.clone());
        let task = interrupted
            .team
            .as_mut()
            .unwrap()
            .tasks
            .iter_mut()
            .find(|t| t.id == writer.task_id)
            .unwrap();
        let reference = task.artifact.take().unwrap();
        interrupted.artifacts.retain(|a| a.id != reference.id);
        interrupted
            .evidence
            .retain(|e| !reference.evidence_refs.contains(&e.id));
        assert!(task.review.is_none());
        task.state = TaskState::Blocked;
    }
    ids.sort();
    interrupted.team.as_mut().unwrap().checkpoint_uncertain = true;
    interrupted.status = Status::Blocked;
    interrupted.reconciliation_required = true;
    f.runtime
        .store
        .lock()
        .unwrap()
        .save(&interrupted, "test_two_missing_receipt_checkpoints", None)
        .unwrap();
    (interrupted, ids)
}

#[test]
fn observed_wave_effects_require_complete_exact_cohort_and_never_repeat_writes() {
    let f = TeamFixture::new("managed_disjoint");
    let (before, task_ids) = two_missing_writer_receipts(&f);
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    for task in &task_ids {
        assert!(
            before.task_effect_reconciliation_scope(task).is_err(),
            "one intent cannot silently adopt the other uncertain writer's file"
        );
    }
    assert!(
        before
            .wave_effect_reconciliation_scope(&task_ids[..1])
            .is_err()
    );
    assert!(
        before
            .wave_effect_reconciliation_scope(&[task_ids[0].clone(), task_ids[0].clone()])
            .is_err()
    );
    let scope = before.wave_effect_reconciliation_scope(&task_ids).unwrap();
    let mut reversed = task_ids.clone();
    reversed.reverse();
    assert_eq!(
        before.wave_effect_reconciliation_scope(&reversed).unwrap(),
        scope,
        "cohort ordering cannot change its meaning"
    );
    let key = id();
    let command = Command::ReconcileWaveEffects {
        run_id: before.id.clone(),
        task_ids: task_ids.clone(),
        scope_hash: scope.clone(),
    };
    let observed = f.runtime.dispatch(&key, command.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(&observed).unwrap(),
        serde_json::to_value(f.runtime.dispatch(&key, command).unwrap()).unwrap()
    );
    assert!(!observed.reconciliation_required);
    assert!(observed.pending_edits.is_empty());
    assert_eq!(observed.spec_revision, before.spec_revision);
    assert!(observed.team.as_ref().unwrap().checkpoint_uncertain);
    assert_eq!(
        observed
            .approvals
            .iter()
            .filter(|a| a.action == "reconcile_wave_effects")
            .count(),
        1
    );
    for task_id in &task_ids {
        let execution = observed
            .task_executions
            .iter()
            .find(|e| e.task_id == *task_id)
            .unwrap();
        let receipt = execution.reconciliation.as_ref().unwrap();
        assert_eq!(receipt.cohort.as_ref().unwrap().scope_hash, scope);
        assert_eq!(receipt.cohort.as_ref().unwrap().members.len(), 2);
        assert_eq!(
            execution.state,
            architect::runtime::team::effects::ExecutionState::ObservedApplied
        );
        assert_eq!(execution.input_snapshot, before.input_snapshot);
        assert!(observed.task_result_restore_scope(task_id).is_ok());
    }
    let mut missing = observed.clone();
    missing
        .task_executions
        .iter_mut()
        .find(|e| e.task_id == task_ids[1])
        .unwrap()
        .reconciliation = None;
    assert!(
        missing.task_result_restore_scope(&task_ids[0]).is_err(),
        "a missing cohort member invalidates the remaining member's proof"
    );
    let mut altered = observed.clone();
    altered
        .task_executions
        .iter_mut()
        .find(|e| e.task_id == task_ids[1])
        .unwrap()
        .artifact_hash = "0".repeat(64);
    assert!(altered.task_result_restore_scope(&task_ids[0]).is_err());
    let mut current = observed;
    for task_id in &task_ids {
        current = f
            .runtime
            .dispatch(
                &id(),
                Command::RestoreTaskResult {
                    run_id: current.id.clone(),
                    task_id: task_id.clone(),
                    scope_hash: current.task_result_restore_scope(task_id).unwrap(),
                },
            )
            .unwrap();
    }
    assert!(!current.team.as_ref().unwrap().checkpoint_uncertain);
    assert_eq!(current.task_result_restorations.len(), 2);
    assert_eq!(
        serde_json::to_value(&current.session_ledger).unwrap(),
        serde_json::to_value(&before.session_ledger).unwrap()
    );
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    for path in ["source.txt", "second.txt"] {
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("workspace").join(path)).unwrap(),
            "managed result"
        );
    }
}

#[test]
fn observed_wave_effects_reject_one_conflicting_member_and_roll_back_entire_cohort() {
    let f = TeamFixture::new("managed_disjoint");
    let (before, task_ids) = two_missing_writer_receipts(&f);
    let scope = before.wave_effect_reconciliation_scope(&task_ids).unwrap();
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    let command = Command::ReconcileWaveEffects {
        run_id: before.id.clone(),
        task_ids: task_ids.clone(),
        scope_hash: scope,
    };
    std::fs::write(
        f.root.path().join("workspace/second.txt"),
        "conflicting user edit",
    )
    .unwrap();
    assert!(before.wave_effect_reconciliation_scope(&task_ids).is_err());
    assert!(f.runtime.dispatch(&id(), command.clone()).is_err());
    assert_eq!(
        serde_json::to_value(f.current()).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/second.txt")).unwrap(),
        "conflicting user edit"
    );
    std::fs::write(f.root.path().join("workspace/second.txt"), "managed result").unwrap();
    let connection = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_cohort BEFORE INSERT ON events WHEN NEW.kind='wave_effects_observed' BEGIN SELECT RAISE(ABORT, 'injected cohort commit failure'); END;").unwrap();
    assert!(f.runtime.dispatch(&id(), command).is_err());
    assert_eq!(
        serde_json::to_value(f.current()).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    assert!(
        f.current()
            .task_executions
            .iter()
            .filter(|e| !e.receipts.is_empty())
            .all(|e| e.reconciliation.is_none())
    );
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
}

#[test]
fn observed_wave_cohort_is_offered_in_daemon_tui_cli_and_retained_after_restart() {
    use architect::tui::backend::{Backend, IpcBackend};
    let f = TeamFixture::new("managed_disjoint");
    let (before, task_ids) = two_missing_writer_receipts(&f);
    let wave = before
        .task_executions
        .iter()
        .find(|e| e.task_id == task_ids[0])
        .unwrap()
        .wave_attempt;
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    std::fs::set_permissions(f.root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let daemon = TestDaemon(
        std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("daemon")
            .arg(f.root.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut backend = IpcBackend::new(f.root.path());
    let deadline = Instant::now() + Duration::from_secs(5);
    let snapshot = loop {
        match backend.snapshot(&before.id) {
            Ok(snapshot) => break snapshot,
            Err(error) => {
                assert!(Instant::now() < deadline, "daemon unavailable: {error}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let package = &snapshot.wave_effect_reconciliations[&wave.to_string()];
    assert_eq!(package.task_ids, task_ids);
    assert_eq!(
        package.scope_hash,
        before.wave_effect_reconciliation_scope(&task_ids).unwrap()
    );
    assert!(
        snapshot.task_effect_reconciliations.is_empty(),
        "partial per-task approval must not hide the missing peer"
    );
    let card = &snapshot.wave_effect_reconciliation_details[&package.scope_hash];
    assert!(task_ids.iter().all(|id| card.contains(id)));
    let command = || {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"));
        command
            .arg("reconcile-wave-effects")
            .arg(f.root.path())
            .arg(&before.id)
            .arg(wave.to_string());
        command
    };
    assert!(!command().output().unwrap().status.success());
    assert!(f.current().reconciliation_required);
    let approved = command().arg("--approve-scoped").output().unwrap();
    assert!(
        approved.status.success(),
        "{}",
        String::from_utf8_lossy(&approved.stderr)
    );
    let after = f.current();
    assert!(!after.reconciliation_required);
    assert!(
        backend
            .snapshot(&before.id)
            .unwrap()
            .wave_effect_reconciliations
            .is_empty()
    );
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    drop(daemon);
    let reopened = Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap();
    let persisted = reopened.store.lock().unwrap().get(&before.id).unwrap();
    for id in &task_ids {
        assert!(persisted.task_result_restore_scope(id).is_ok());
    }
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
}

#[test]
fn exact_native_resume_preserves_executor_and_verifier_attempts_and_history_after_restart() {
    for (mode, verifier) in [("resume_executor_once", false), ("fail_context_once", true)] {
        let f = TeamFixture::new(mode);
        let installed = f.install_team();
        let executed = step(&f, &installed);
        let blocked = if verifier {
            let verifier_id = executed
                .team
                .as_ref()
                .unwrap()
                .tasks
                .iter()
                .find(|t| t.id == "task-contextanalyst")
                .unwrap()
                .verifier_id
                .clone();
            let queued = f
                .runtime
                .dispatch(
                    &id(),
                    Command::SendMessage {
                        run_id: executed.id.clone(),
                        target_agent_id: verifier_id,
                        text: "Native verifier instruction delivered at most once".into(),
                    },
                )
                .unwrap();
            step(&f, &queued)
        } else {
            executed
        };
        let before = reconcile_team(&f, &blocked);
        let package = architect::runtime::team::resume::for_task(&before, "task-contextanalyst")
            .unwrap_or_else(|error| panic!("{mode}: continuation refused: {error}"));
        let previous = before.session_ledger.turns[&package.previous_turn_id].clone();
        let session_count = before.session_ledger.sessions.len();
        let task = before
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == package.task_id)
            .unwrap();
        let attempt = task.attempt;
        let original_artifact = serde_json::to_value(&task.artifact).unwrap();
        let old_calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
        assert!(
            f.runtime
                .dispatch(
                    &id(),
                    Command::ResumeTaskSession {
                        run_id: before.id.clone(),
                        task_id: package.task_id.clone(),
                        scope_hash: "old-scope".into()
                    }
                )
                .is_err()
        );
        std::fs::write(f.root.path().join("retry-ready"), "ready").unwrap();
        let runtime =
            Arc::new(Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap());
        let key = id();
        let command = Command::ResumeTaskSession {
            run_id: before.id.clone(),
            task_id: package.task_id.clone(),
            scope_hash: package.scope_hash.clone(),
        };
        runtime.dispatch(&key, command.clone()).unwrap();
        let after = f.wait_for_idle();
        let task = after
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == package.task_id)
            .unwrap();
        assert_eq!(
            task.attempt, attempt,
            "native continuation must not create another executor attempt"
        );
        assert_eq!(
            task.state,
            if verifier {
                TaskState::Verified
            } else {
                TaskState::AwaitingVerification
            },
            "{:?}",
            after.diagnostic
        );
        assert_eq!(
            after.session_ledger.sessions.len(),
            session_count,
            "reuse managed/native identity"
        );
        assert_eq!(
            &after.session_ledger.turns[&package.previous_turn_id],
            &previous
        );
        let session = &after.session_ledger.sessions[&package.session_id];
        assert_eq!(
            session.native_session_id.as_deref(),
            Some(package.native_session_id.as_str())
        );
        assert_ne!(
            session.current_turn_id.as_deref(),
            Some(package.previous_turn_id.as_str())
        );
        assert_eq!(
            after.session_ledger.turns[session.current_turn_id.as_ref().unwrap()].state,
            architect::sessions::TurnState::Completed
        );
        if verifier {
            assert_eq!(
                serde_json::to_value(&task.artifact).unwrap(),
                original_artifact
            );
            assert_eq!(
                serde_json::to_value(&after.communication_ledger).unwrap(),
                serde_json::to_value(&before.communication_ledger).unwrap(),
                "unknown previous delivery must remain unknown, without replay"
            );
        }
        let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
        assert_eq!(
            calls.lines().count(),
            old_calls.lines().count() + 1,
            "only the selected task continues"
        );
        let last: serde_json::Value = serde_json::from_str(calls.lines().last().unwrap()).unwrap();
        let args = last["argv"].as_array().unwrap();
        let index = args.iter().position(|a| a == "--resume").unwrap();
        assert_eq!(args[index + 1], package.native_session_id);
        assert!(!args.iter().any(|a| a == "--session-id"));
        assert!(
            last["prompt"]
                .as_str()
                .unwrap()
                .contains("Native continuation checkpoint")
        );
        assert!(
            !last["prompt"]
                .as_str()
                .unwrap()
                .contains("Native verifier instruction delivered at most once")
        );
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("workspace/source.txt")).unwrap(),
            "original input"
        );
        runtime.dispatch(&key, command).unwrap();
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap(),
            calls,
            "idempotent acknowledgment must not resume twice"
        );
    }
}

#[test]
fn native_resume_refuses_source_identity_checkpoint_drift_and_rolls_back_failed_approval() {
    let f = TeamFixture::new("fail_context_once");
    let executed = step(&f, &f.install_team());
    let before = reconcile_team(&f, &step(&f, &executed));
    let package = architect::runtime::team::resume::pending(&before)["task-contextanalyst"].clone();
    let source = f.root.path().join("workspace/source.txt");
    std::fs::write(&source, "changed outside the observed session").unwrap();
    assert!(architect::runtime::team::resume::pending(&before).is_empty());
    std::fs::write(&source, "original input").unwrap();
    for kind in 0..7 {
        let mut drift = before.clone();
        match kind {
            0 => {
                drift
                    .session_ledger
                    .sessions
                    .get_mut(&package.session_id)
                    .unwrap()
                    .native_observed = false
            }
            1 => {
                drift
                    .session_ledger
                    .sessions
                    .get_mut(&package.session_id)
                    .unwrap()
                    .native_session_id = Some("latest".into())
            }
            2 => drift.team.as_mut().unwrap().checkpoint_uncertain = true,
            3 => {
                drift
                    .session_ledger
                    .sessions
                    .get_mut(&package.session_id)
                    .unwrap()
                    .scope
                    .configuration_hash = "0".repeat(64)
            }
            4 => {
                drift
                    .session_ledger
                    .turn_order
                    .retain(|id| id != &package.previous_turn_id);
                drift
                    .session_ledger
                    .turn_order
                    .insert(0, package.previous_turn_id.clone());
            }
            5 => {
                let turn = drift
                    .session_ledger
                    .turns
                    .get_mut(&package.previous_turn_id)
                    .unwrap();
                turn.output = Some("retained committed output".into());
                turn.output_ref = Some(architect::sessions::OutputRef {
                    document: format!("turn:{}", turn.id),
                    hash: architect::hash(turn.output.as_ref().unwrap()),
                });
            }
            _ => {
                let task = drift
                    .team
                    .as_mut()
                    .unwrap()
                    .tasks
                    .iter_mut()
                    .find(|t| t.id == package.task_id)
                    .unwrap();
                let artifact = task.artifact.as_ref().unwrap();
                task.review = Some(architect::team::ReviewDecision {
                    verifier_id: task.verifier_id.clone(),
                    session_id: package.session_id.clone(),
                    artifact_hash: artifact.content_hash.clone(),
                    spec_revision: before.spec_revision,
                    criteria: task
                        .criterion_ids
                        .iter()
                        .map(|id| (id.clone(), false))
                        .collect(),
                    evidence_refs: artifact.evidence_refs.clone(),
                    finding_ids: std::collections::BTreeSet::from(["unresolved".into()]),
                    accepted: false,
                });
            }
        }
        assert!(
            !architect::runtime::team::resume::pending(&drift).contains_key(&package.task_id),
            "accepted drift {kind}"
        );
    }
    let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    let db = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_native_resume BEFORE INSERT ON events WHEN NEW.kind='task_native_resume_started' BEGIN SELECT RAISE(ABORT, 'injected native resume approval failure'); END;").unwrap();
    let key = id();
    let command = Command::ResumeTaskSession {
        run_id: before.id.clone(),
        task_id: package.task_id.clone(),
        scope_hash: package.scope_hash.clone(),
    };
    assert!(f.runtime.dispatch(&key, command.clone()).is_err());
    let retained = f.current();
    assert_eq!(
        serde_json::to_value(&retained).unwrap(),
        serde_json::to_value(&before).unwrap(),
        "failed approval must retain the complete prior state"
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    db.execute_batch("DROP TRIGGER refuse_native_resume")
        .unwrap();
    std::fs::write(f.root.path().join("retry-ready"), "ready").unwrap();
    f.runtime.dispatch(&key, command).unwrap();
    assert_eq!(
        f.wait_for_idle()
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == package.task_id)
            .unwrap()
            .state,
        TaskState::Verified
    );
}

#[test]
fn daemon_and_cli_expose_native_resume_and_preserve_it_after_restart() {
    use architect::tui::backend::{Backend, IpcBackend};
    let f = TeamFixture::new("fail_context_once");
    let executed = step(&f, &f.install_team());
    let before = reconcile_team(&f, &step(&f, &executed));
    let package = architect::runtime::team::resume::pending(&before)["task-contextanalyst"].clone();
    let state = f.root.path();
    let mut daemon = TestDaemon(
        architect::tui::connect_or_start_daemon(
            state,
            std::path::Path::new(env!("CARGO_BIN_EXE_antislop")),
        )
        .unwrap()
        .unwrap(),
    );
    let mut backend = IpcBackend::new(state);
    let snapshot = backend.snapshot(&before.id).unwrap();
    let projected = &snapshot.task_session_resumes[&package.task_id];
    assert_eq!(projected.session_id, package.session_id);
    assert_eq!(projected.previous_turn_id, package.previous_turn_id);
    assert_eq!(projected.native_session_id, package.native_session_id);
    assert_eq!(projected.scope_hash, package.scope_hash);
    assert!(
        snapshot.task_session_resume_details[&package.scope_hash]
            .contains(&package.native_session_id)
    );
    let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    let refused = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("resume-task-session")
        .arg(state)
        .arg(&before.id)
        .arg(&package.task_id)
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    std::fs::write(f.root.path().join("retry-ready"), "ready").unwrap();
    let accepted = std::process::Command::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("resume-task-session")
        .arg(state)
        .arg(&before.id)
        .arg(&package.task_id)
        .arg("--approve-scoped")
        .output()
        .unwrap();
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let after = f.wait_for_idle();
    let task = after
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == package.task_id)
        .unwrap();
    assert_eq!(task.state, TaskState::Verified, "{:?}", after.diagnostic);
    assert_eq!(task.native_resumes.len(), 1);
    assert_eq!(
        after.session_ledger.turns[&package.previous_turn_id],
        before.session_ledger.turns[&package.previous_turn_id]
    );
    let current_calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    assert_eq!(current_calls.lines().count(), calls.lines().count() + 1);
    daemon.0.kill().unwrap();
    daemon.0.wait().unwrap();
    daemon.0 = architect::tui::connect_or_start_daemon(
        state,
        std::path::Path::new(env!("CARGO_BIN_EXE_antislop")),
    )
    .unwrap()
    .unwrap();
    let restored = backend.snapshot(&after.id).unwrap();
    assert!(restored.task_session_resumes.is_empty());
    let saved: Run = architect::transport::decode(
        architect::ipc::read_document(state, &after.id, "run")
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(
        saved.session_ledger.sessions[&package.session_id]
            .native_session_id
            .as_deref(),
        Some(package.native_session_id.as_str())
    );
    assert_eq!(
        saved
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == package.task_id)
            .unwrap()
            .native_resumes
            .len(),
        1
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap(),
        current_calls
    );
}

#[test]
fn replacement_handoff_starts_distinct_sessions_for_unobserved_executor_and_failed_verifier() {
    for (mode, verifier) in [
        ("replacement_executor_unobserved", false),
        ("fail_context_once", true),
    ] {
        let f = TeamFixture::new(mode);
        let executed = step(&f, &f.install_team());
        let blocked = if verifier {
            let target = executed
                .team
                .as_ref()
                .unwrap()
                .tasks
                .iter()
                .find(|t| t.id == "task-contextanalyst")
                .unwrap()
                .verifier_id
                .clone();
            let queued = f
                .runtime
                .dispatch(
                    &id(),
                    Command::SendMessage {
                        run_id: executed.id.clone(),
                        target_agent_id: target,
                        text: "Preserve this historical constraint in the replacement handoff"
                            .into(),
                    },
                )
                .unwrap();
            step(&f, &queued)
        } else {
            executed
        };
        let before = reconcile_team(&f, &blocked);
        let package =
            architect::runtime::team::replacement::for_task(&before, "task-contextanalyst")
                .unwrap();
        if !verifier {
            assert!(
                !architect::runtime::team::resume::pending(&before).contains_key(&package.task_id)
            );
        }
        let old_session =
            before.session_ledger.sessions[package.previous_session.as_ref().unwrap()].clone();
        let old_turn = before.session_ledger.turns[package.previous_turn.as_ref().unwrap()].clone();
        let old_artifact = serde_json::to_value(
            &before
                .team
                .as_ref()
                .unwrap()
                .tasks
                .iter()
                .find(|t| t.id == package.task_id)
                .unwrap()
                .artifact,
        )
        .unwrap();
        let old_calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
        package.validate_for_run(&before).unwrap();
        let mut wrong = package.clone();
        wrong.operation = if verifier {
            architect::team::TaskOperation::Execute
        } else {
            architect::team::TaskOperation::Verify
        };
        assert!(wrong.validate_for_run(&before).is_err());
        let mut forged = package.clone();
        forged.handoff.contract.objective = "different approved objective".into();
        forged.handoff_hash = forged.handoff.hash().unwrap();
        assert!(forged.validate_for_run(&before).is_err());
        assert_eq!(package.handoff.hash().unwrap(), package.handoff_hash);
        assert_eq!(package.handoff.contract.objective, before.spec.objective);
        assert_eq!(package.handoff.source_hashes, before.source_hashes());
        assert!(
            package
                .handoff
                .prior_turns
                .iter()
                .any(|t| t.id == old_turn.id)
        );
        if verifier {
            assert!(
                serde_json::to_string(&package.handoff.message_history)
                    .unwrap()
                    .contains("Preserve this historical constraint")
            );
        }
        std::fs::write(f.root.path().join("retry-ready"), "ready").unwrap();
        let runtime =
            Arc::new(Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap());
        let key = id();
        let command = Command::ReplaceTaskSession {
            run_id: before.id.clone(),
            task_id: package.task_id.clone(),
            scope_hash: package.scope_hash.clone(),
        };
        runtime.dispatch(&key, command.clone()).unwrap();
        let after = f.wait_for_idle();
        let task = after
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == package.task_id)
            .unwrap();
        assert_eq!(
            task.state,
            if verifier {
                TaskState::Verified
            } else {
                TaskState::AwaitingVerification
            },
            "{mode}: {:?}",
            after.diagnostic
        );
        assert_eq!(task.attempt, package.handoff.task_attempt);
        assert_eq!(after.session_ledger.sessions[&old_session.id], old_session);
        assert_eq!(after.session_ledger.turns[&old_turn.id], old_turn);
        assert_eq!(
            after.session_ledger.sessions.len(),
            before.session_ledger.sessions.len() + 1
        );
        let replacement = task.native_replacements.last().unwrap();
        let fresh = &after.session_ledger.sessions[&replacement.new_session_id];
        assert_ne!(fresh.id, old_session.id);
        assert!(fresh.native_observed);
        assert_ne!(fresh.native_session_id, old_session.native_session_id);
        assert_eq!(replacement.handoff_hash, package.handoff_hash);
        if verifier {
            assert_eq!(serde_json::to_value(&task.artifact).unwrap(), old_artifact);
            assert_eq!(
                after.communication_ledger, before.communication_ledger,
                "historical handoff is not old-message redelivery"
            );
        }
        let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
        assert_eq!(calls.lines().count(), old_calls.lines().count() + 1);
        let last: serde_json::Value = serde_json::from_str(calls.lines().last().unwrap()).unwrap();
        assert!(
            last["argv"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a == "--session-id")
        );
        assert!(
            !last["argv"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a == "--resume")
        );
        let prompt = last["prompt"].as_str().unwrap();
        let captured: serde_json::Value = serde_json::Deserializer::from_str(
            prompt
                .split("Immutable replacement handoff: ")
                .nth(1)
                .unwrap(),
        )
        .into_iter()
        .next()
        .unwrap()
        .unwrap();
        assert_eq!(captured, serde_json::to_value(&package.handoff).unwrap());
        assert!(prompt.contains("DISTINCT NEW SESSION") && prompt.contains(&package.handoff_hash));
        runtime.dispatch(&key, command).unwrap();
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap(),
            calls
        );
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("workspace/source.txt")).unwrap(),
            "original input"
        );
    }
}

#[test]
fn replacement_handoff_refuses_drift_uncertainty_and_failed_commit_without_new_native_work() {
    let f = TeamFixture::new("replacement_executor_unobserved");
    let blocked = step(&f, &f.install_team());
    let before = reconcile_team(&f, &blocked);
    let package =
        architect::runtime::team::replacement::for_task(&before, "task-contextanalyst").unwrap();
    let mut corrupt = before.clone();
    if let Some(body) = corrupt.message_bodies.values_mut().next() {
        *body = "corrupted historical user instruction".into();
    }
    assert!(architect::runtime::team::replacement::for_task(&corrupt, &package.task_id).is_err());
    let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    let source = f.root.path().join("workspace/source.txt");
    std::fs::write(&source, "changed").unwrap();
    assert!(
        !architect::runtime::team::replacement::pending(&before).contains_key(&package.task_id)
    );
    std::fs::write(&source, "original input").unwrap();
    let mut uncertain = before.clone();
    uncertain.team.as_mut().unwrap().checkpoint_uncertain = true;
    assert!(architect::runtime::team::replacement::pending(&uncertain).is_empty());
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::ReplaceTaskSession {
                    run_id: before.id.clone(),
                    task_id: package.task_id.clone(),
                    scope_hash: "stale handoff".into()
                }
            )
            .is_err()
    );
    let db = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_replacement BEFORE INSERT ON events WHEN NEW.kind='task_session_replacement_started' BEGIN SELECT RAISE(ABORT, 'injected handoff approval failure'); END;").unwrap();
    let key = id();
    let command = Command::ReplaceTaskSession {
        run_id: before.id.clone(),
        task_id: package.task_id.clone(),
        scope_hash: package.scope_hash.clone(),
    };
    assert!(f.runtime.dispatch(&key, command.clone()).is_err());
    assert_eq!(
        serde_json::to_value(f.current()).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    db.execute_batch("DROP TRIGGER refuse_replacement").unwrap();
    std::fs::write(f.root.path().join("retry-ready"), "ready").unwrap();
    f.runtime.dispatch(&key, command).unwrap();
    let after = f.wait_for_idle();
    let replacement = after
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == package.task_id)
        .unwrap()
        .native_replacements
        .last()
        .unwrap();
    let mut forged = after.clone();
    forged
        .team
        .as_mut()
        .unwrap()
        .tasks
        .iter_mut()
        .find(|t| t.id == package.task_id)
        .unwrap()
        .native_replacements
        .last_mut()
        .unwrap()
        .handoff
        .contract
        .objective = "invented scope".into();
    assert!(
        forged
            .team
            .as_ref()
            .unwrap()
            .validate(&forged.spec)
            .is_err()
    );
    assert_eq!(
        replacement.handoff.contract.objective,
        before.spec.objective
    );
}

#[test]
fn replacement_handoff_is_reviewable_through_daemon_tui_cli_and_survives_restart() {
    use architect::tui::backend::{Backend, IpcBackend};
    let f = TeamFixture::new("replacement_executor_unobserved");
    let before = reconcile_team(&f, &step(&f, &f.install_team()));
    let p =
        architect::runtime::team::replacement::for_task(&before, "task-contextanalyst").unwrap();
    let state = f.root.path();
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_antislop"));
    let mut daemon = TestDaemon(
        architect::tui::connect_or_start_daemon(state, executable)
            .unwrap()
            .unwrap(),
    );
    let mut backend = IpcBackend::new(state);
    let snapshot = backend.snapshot(&before.id).unwrap();
    let projected = &snapshot.task_session_replacements[&p.task_id];
    assert_eq!(projected.handoff_hash, p.handoff_hash);
    assert_eq!(projected.scope_hash, p.scope_hash);
    assert!(snapshot.task_session_replacement_details[&p.scope_hash].contains(&p.handoff_hash));
    assert!(!snapshot.task_session_resumes.contains_key(&p.task_id));
    let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    let refused = std::process::Command::new(executable)
        .arg("replace-task-session")
        .arg(state)
        .arg(&before.id)
        .arg(&p.task_id)
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    std::fs::write(f.root.path().join("retry-ready"), "ready").unwrap();
    let accepted = std::process::Command::new(executable)
        .arg("replace-task-session")
        .arg(state)
        .arg(&before.id)
        .arg(&p.task_id)
        .arg("--approve-scoped")
        .output()
        .unwrap();
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let after = f.wait_for_idle();
    let task = after
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == p.task_id)
        .unwrap();
    assert_eq!(
        task.state,
        TaskState::AwaitingVerification,
        "{:?}",
        after.diagnostic
    );
    let record = task.native_replacements.last().unwrap().clone();
    assert_eq!(record.handoff_hash, p.handoff_hash);
    let current_calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    assert_eq!(current_calls.lines().count(), calls.lines().count() + 1);
    daemon.0.kill().unwrap();
    daemon.0.wait().unwrap();
    daemon.0 = architect::tui::connect_or_start_daemon(state, executable)
        .unwrap()
        .unwrap();
    let restored = backend.snapshot(&after.id).unwrap();
    assert!(restored.task_session_replacements.is_empty());
    let saved: Run = architect::transport::decode(
        architect::ipc::read_document(state, &after.id, "run")
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    let stored = saved
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == p.task_id)
        .unwrap()
        .native_replacements
        .last()
        .unwrap();
    assert_eq!(stored.handoff_hash, p.handoff_hash);
    assert_eq!(stored.new_session_id, record.new_session_id);
    assert!(saved.session_ledger.sessions[&stored.new_session_id].native_observed);
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap(),
        current_calls
    );
}

#[test]
fn replacement_handoff_follows_actual_invalid_session_error_without_silent_fallback() {
    let f = TeamFixture::new("replacement_lost_session");
    let executed = step(&f, &f.install_team());
    let before = reconcile_team(&f, &step(&f, &executed));
    let resume =
        architect::runtime::team::resume::for_task(&before, "task-contextanalyst").unwrap();
    std::fs::write(f.root.path().join("retry-ready"), "ready").unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::ResumeTaskSession {
                run_id: before.id.clone(),
                task_id: resume.task_id.clone(),
                scope_hash: resume.scope_hash,
            },
        )
        .unwrap();
    let blocked = f.wait_for_idle();
    assert_eq!(blocked.status, Status::Blocked);
    let current = &blocked.session_ledger.sessions[&resume.session_id];
    let turn = &blocked.session_ledger.turns[current.current_turn_id.as_ref().unwrap()];
    assert_eq!(
        turn.failure,
        Some(architect::providers::FailureKind::InvalidSession)
    );
    assert_eq!(
        blocked.session_ledger.sessions.len(),
        before.session_ledger.sessions.len(),
        "failed resume must not silently allocate a new session"
    );
    let reconciled = reconcile_team(&f, &blocked);
    assert!(!architect::runtime::team::resume::pending(&reconciled).contains_key(&resume.task_id));
    let p = architect::runtime::team::replacement::for_task(&reconciled, &resume.task_id).unwrap();
    let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::ReplaceTaskSession {
                run_id: reconciled.id.clone(),
                task_id: p.task_id.clone(),
                scope_hash: p.scope_hash,
            },
        )
        .unwrap();
    let after = f.wait_for_idle();
    let task = after
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == resume.task_id)
        .unwrap();
    assert_eq!(task.state, TaskState::Verified, "{:?}", after.diagnostic);
    assert_eq!(task.attempt, 1);
    assert_eq!(
        after.session_ledger.sessions.len(),
        reconciled.session_ledger.sessions.len() + 1
    );
    assert_eq!(after.session_ledger.turns[&turn.id], *turn);
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls.jsonl"))
            .unwrap()
            .lines()
            .count(),
        calls.lines().count() + 1
    );
}

#[test]
fn captured_check_recovery_restores_actual_success_or_failure_without_repeating_command() {
    use architect::runtime::team::{check_recovery, checks::CheckState};
    for mode in ["task_check_success", "task_check_failure"] {
        let f = TeamFixture::new(mode);
        let executed = step(&f, &f.install_team());
        let db = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
        db.execute_batch("CREATE TRIGGER refuse_check_finished BEFORE INSERT ON events WHEN NEW.kind='task_check_finished' BEGIN SELECT RAISE(ABORT, 'injected check outcome checkpoint failure'); END;").unwrap();
        let blocked = step(&f, &executed);
        let before = reconcile_team(&f, &blocked);
        assert_eq!(before.task_checks.len(), 1);
        assert_eq!(before.task_check_outcomes.len(), 1);
        let inv = &before.task_checks[0];
        let p = check_recovery::for_invocation(&before, &inv.id).unwrap();
        let actual: serde_json::Value = serde_json::from_str(&p.outcome.output).unwrap();
        assert_eq!(
            actual["exit_code"],
            if mode == "task_check_success" { 0 } else { 1 }
        );
        let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
        db.execute_batch("DROP TRIGGER refuse_check_finished")
            .unwrap();
        let runtime =
            Arc::new(Runtime::new(Store::open(&f.root.path().join("state.db")).unwrap()).unwrap());
        let key = id();
        let command = Command::RestoreTaskCheckResult {
            run_id: before.id.clone(),
            invocation_id: inv.id.clone(),
            scope_hash: p.scope_hash,
        };
        let restored = runtime.dispatch(&key, command.clone()).unwrap();
        assert_eq!(restored.task_checks[0].state, CheckState::Completed);
        assert_eq!(
            restored.task_checks[0].success,
            mode == "task_check_success"
        );
        assert_eq!(
            restored.task_checks[0]
                .restoration
                .as_ref()
                .unwrap()
                .original_state,
            inv.state
        );
        let task = restored
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == inv.task_id)
            .unwrap();
        assert_eq!(task.state, TaskState::AwaitingVerification);
        assert!(task.review.is_none());
        assert_eq!(
            serde_json::to_value(&restored.artifacts).unwrap(),
            serde_json::to_value(&before.artifacts).unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap(),
            calls
        );
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("workspace/runner-count")).unwrap(),
            "1"
        );
        runtime.dispatch(&key, command).unwrap();
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("workspace/runner-count")).unwrap(),
            "1"
        );
        let reviewed = step(&f, &restored);
        let task = reviewed
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == inv.task_id)
            .unwrap();
        assert_eq!(
            task.state,
            if mode == "task_check_success" {
                TaskState::Verified
            } else {
                TaskState::ChangesRequested
            },
            "{mode}: {:?}",
            reviewed.diagnostic
        );
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("workspace/runner-count")).unwrap(),
            "1"
        );
        assert_eq!(
            std::fs::read_to_string(f.root.path().join("calls.jsonl"))
                .unwrap()
                .lines()
                .count(),
            calls.lines().count() + 1,
            "independent verifier only"
        );
    }
}

#[test]
fn captured_check_recovery_refuses_missing_corrupt_changed_and_failed_approval() {
    use architect::runtime::team::check_recovery;
    let f = TeamFixture::new("task_check_success");
    let executed = step(&f, &f.install_team());
    let db = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_check_finished BEFORE INSERT ON events WHEN NEW.kind='task_check_finished' BEGIN SELECT RAISE(ABORT, 'injected checkpoint failure'); END;").unwrap();
    let before = reconcile_team(&f, &step(&f, &executed));
    let inv = before.task_checks[0].id.clone();
    let p = check_recovery::for_invocation(&before, &inv).unwrap();
    for kind in 0..5 {
        let mut forged = before.clone();
        match kind {
            0 => forged.task_check_outcomes.clear(),
            1 => forged.task_check_outcomes[0].output = "invented result".into(),
            2 => forged.task_check_outcomes[0].observed_after = None,
            3 => forged.task_checks[0]
                .plan
                .argv
                .push("unapproved argument".into()),
            _ => forged.checklist_version = "outdated check contract".into(),
        };
        assert!(
            check_recovery::pending(&forged).is_empty(),
            "accepted drift {kind}"
        );
    }
    let source = f.root.path().join("workspace/source.txt");
    std::fs::write(&source, "changed").unwrap();
    assert!(check_recovery::pending(&before).is_empty());
    std::fs::write(&source, "original input").unwrap();
    let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    db.execute_batch("DROP TRIGGER refuse_check_finished; CREATE TRIGGER refuse_check_restore BEFORE INSERT ON events WHEN NEW.kind='task_check_result_restored' BEGIN SELECT RAISE(ABORT, 'injected restore approval failure'); END;").unwrap();
    let key = id();
    let command = Command::RestoreTaskCheckResult {
        run_id: before.id.clone(),
        invocation_id: inv,
        scope_hash: p.scope_hash,
    };
    assert!(f.runtime.dispatch(&key, command.clone()).is_err());
    assert_eq!(
        serde_json::to_value(f.current()).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    db.execute_batch("DROP TRIGGER refuse_check_restore")
        .unwrap();
    f.runtime.dispatch(&key, command).unwrap();
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/runner-count")).unwrap(),
        "1"
    );
}

#[test]
fn captured_check_recovery_keeps_original_expected_failing_baseline_without_replay() {
    use architect::runtime::team::{check_recovery, checks::CheckState, replacement};
    let f = TeamFixture::new("baseline_expected_failure");
    let installed = f.install_team();
    let db = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_baseline_result BEFORE INSERT ON events WHEN NEW.kind='task_check_finished' BEGIN SELECT RAISE(ABORT, 'injected baseline result failure'); END;").unwrap();
    let stopped = step(&f, &installed);
    let before = reconcile_team(&f, &stopped);
    let inv = &before.task_checks[0];
    assert!(before.team.as_ref().unwrap().checkpoint_uncertain);
    for kind in 0..3 {
        let mut forged = before.clone();
        match kind {
            0 => forged.task_check_outcomes[0].baseline_barrier = None,
            1 => {
                forged.task_check_outcomes[0]
                    .baseline_barrier
                    .as_mut()
                    .unwrap()
                    .native_ledger_hash = "unobserved native work".into()
            }
            _ => {
                forged.task_check_outcomes[0]
                    .baseline_barrier
                    .as_mut()
                    .unwrap()
                    .team
                    .tasks[0]
                    .attempt += 1
            }
        }
        assert!(
            check_recovery::pending(&forged).is_empty(),
            "accepted invalid baseline barrier {kind}"
        );
    }
    let p = check_recovery::for_invocation(&before, &inv.id).unwrap();
    let actual: serde_json::Value = serde_json::from_str(&p.outcome.output).unwrap();
    assert_eq!(actual["exit_code"], 1);
    assert!(!p.outcome.process_success);
    let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    db.execute_batch("DROP TRIGGER refuse_baseline_result")
        .unwrap();
    let restored = f
        .runtime
        .dispatch(
            &id(),
            Command::RestoreTaskCheckResult {
                run_id: before.id.clone(),
                invocation_id: inv.id.clone(),
                scope_hash: p.scope_hash,
            },
        )
        .unwrap();
    assert_eq!(restored.task_checks[0].state, CheckState::Completed);
    assert!(!restored.task_checks[0].success);
    assert!(!restored.team.as_ref().unwrap().checkpoint_uncertain);
    assert!(restored.quality_gates.is_empty());
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/baseline-count")).unwrap(),
        "1"
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    let replacement = replacement::for_task(&restored, &inv.task_id).unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::ReplaceTaskSession {
                run_id: restored.id.clone(),
                task_id: inv.task_id.clone(),
                scope_hash: replacement.scope_hash,
            },
        )
        .unwrap();
    let executed = f.wait_for_idle();
    assert_eq!(
        executed
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == inv.task_id)
            .unwrap()
            .state,
        TaskState::AwaitingVerification,
        "{:?}",
        executed.diagnostic
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/baseline-count")).unwrap(),
        "1"
    );
    assert_eq!(executed.task_checks.len(), 1);
    assert_eq!(
        executed.task_checks[0].input_snapshot,
        before.task_checks[0].input_snapshot
    );
}

#[test]
fn captured_check_recovery_is_reviewable_through_daemon_tui_cli_and_survives_restart() {
    use architect::runtime::team::check_recovery;
    use architect::tui::backend::{Backend, IpcBackend};
    let f = TeamFixture::new("task_check_failure");
    let executed = step(&f, &f.install_team());
    let db = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_check_finished BEFORE INSERT ON events WHEN NEW.kind='task_check_finished' BEGIN SELECT RAISE(ABORT, 'injected check result failure'); END;").unwrap();
    let before = reconcile_team(&f, &step(&f, &executed));
    db.execute_batch("DROP TRIGGER refuse_check_finished")
        .unwrap();
    let invocation_id = before.task_checks[0].id.clone();
    let p = check_recovery::for_invocation(&before, &invocation_id).unwrap();
    p.validate_for_run(&before).unwrap();
    for kind in 0..3 {
        let mut forged = p.clone();
        match kind {
            0 => forged.outcome.output = "fictitious success".into(),
            1 => forged.task_id = "other-task".into(),
            _ => forged.scope_hash = "stale-scope".into(),
        }
        assert!(forged.validate_for_run(&before).is_err());
    }
    let state = f.root.path();
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_antislop"));
    let mut daemon = TestDaemon(
        architect::tui::connect_or_start_daemon(state, executable)
            .unwrap()
            .unwrap(),
    );
    let mut backend = IpcBackend::new(state);
    let snapshot = backend.snapshot(&before.id).unwrap();
    let projected = &snapshot.task_check_restores[&invocation_id];
    assert_eq!(projected.scope_hash, p.scope_hash);
    assert_eq!(projected.outcome.output, p.outcome.output);
    assert!(!projected.outcome.process_success);
    assert!(snapshot.task_check_restore_details[&p.scope_hash].contains(&p.outcome.output_hash));
    assert!(snapshot.task_check_restore_details[&p.scope_hash].contains("actual task check"));
    let calls = std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap();
    let refused = std::process::Command::new(executable)
        .arg("restore-task-check-result")
        .arg(state)
        .arg(&before.id)
        .arg(&invocation_id)
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert_eq!(
        serde_json::to_value(f.current()).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    let accepted = std::process::Command::new(executable)
        .arg("restore-task-check-result")
        .arg(state)
        .arg(&before.id)
        .arg(&invocation_id)
        .arg("--approve-scoped")
        .output()
        .unwrap();
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let after = f.current();
    assert!(!after.task_checks[0].success);
    assert_eq!(
        after.task_checks[0]
            .restoration
            .as_ref()
            .unwrap()
            .outcome_id,
        p.outcome.id
    );
    daemon.0.kill().unwrap();
    daemon.0.wait().unwrap();
    daemon.0 = architect::tui::connect_or_start_daemon(state, executable)
        .unwrap()
        .unwrap();
    assert!(
        backend
            .snapshot(&after.id)
            .unwrap()
            .task_check_restores
            .is_empty()
    );
    let saved: Run = architect::transport::decode(
        architect::ipc::read_document(state, &after.id, "run")
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    let evidence = saved
        .evidence
        .iter()
        .find(|e| Some(&e.id) == saved.task_checks[0].evidence_id.as_ref())
        .unwrap();
    assert_eq!(evidence.output, p.outcome.output);
    assert!(!evidence.success);
    assert_eq!(saved.task_check_outcomes[0].output, p.outcome.output);
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/runner-count")).unwrap(),
        "1"
    );
}

#[test]
fn captured_check_recovery_preserves_original_baseline_after_restored_writer_changes_sources() {
    use architect::runtime::team::{check_recovery, replacement};
    let f = TeamFixture::new("managed_baseline");
    let ready = before_managed_change(&f);
    let db = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    db.execute_batch("CREATE TRIGGER refuse_baseline_result BEFORE INSERT ON events WHEN NEW.kind='task_check_finished' BEGIN SELECT RAISE(ABORT, 'injected baseline result failure'); END;").unwrap();
    let stopped = reconcile_team(&f, &step(&f, &ready));
    let inv = stopped
        .task_checks
        .iter()
        .find(|c| c.plan.phase == architect::team::checks::CheckPhase::BeforeExecution)
        .unwrap();
    let p = check_recovery::for_invocation(&stopped, &inv.id).unwrap();
    db.execute_batch("DROP TRIGGER refuse_baseline_result")
        .unwrap();
    let restored = f
        .runtime
        .dispatch(
            &id(),
            Command::RestoreTaskCheckResult {
                run_id: stopped.id.clone(),
                invocation_id: inv.id.clone(),
                scope_hash: p.scope_hash,
            },
        )
        .unwrap();
    let replacement = replacement::for_task(&restored, &inv.task_id).unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::ReplaceTaskSession {
                run_id: restored.id.clone(),
                task_id: inv.task_id.clone(),
                scope_hash: replacement.scope_hash,
            },
        )
        .unwrap();
    let changed = f.wait_for_idle();
    assert_eq!(changed.input_snapshot["source.txt"], "managed result");
    let ready = refresh_prerequisites(&f, changed, &inv.task_id);
    let refreshed = refresh_sources(&f, &ready, &inv.task_id);
    let reviewed = step(&f, &refreshed);
    let task = reviewed
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == inv.task_id)
        .unwrap();
    assert_eq!(task.state, TaskState::Verified, "{:?}", task.diagnostic);
    let baseline = reviewed
        .task_checks
        .iter()
        .find(|c| c.id == inv.id)
        .unwrap();
    assert_eq!(
        baseline.input_snapshot.as_ref().unwrap()["source.txt"],
        "original input"
    );
    assert!(!baseline.success);
    assert!(baseline.restoration.is_some());
    assert_eq!(
        reviewed
            .task_checks
            .iter()
            .filter(|c| c.plan.phase == architect::team::checks::CheckPhase::BeforeExecution)
            .count(),
        1
    );
    assert_eq!(
        std::fs::read_to_string(
            f.root
                .path()
                .join(format!("workspace/baseline-{}", inv.task_id))
        )
        .unwrap(),
        "1"
    );
}

#[test]
fn guided_chat_offers_the_actual_wave_then_its_review_and_stage_acceptance() {
    use architect::runtime::team::{stage, wave};
    use architect::tui::{
        app::{App, Effect, View},
        backend::Action,
        model::Snapshot,
    };
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let f = TeamFixture::new("success");
    let installed = f.install_team();
    let package = wave::pending(&installed).unwrap();
    assert_eq!(package.scope_hash, installed.team_step_scope().unwrap());
    assert!(!package.actions.is_empty());
    assert!(
        package
            .actions
            .iter()
            .all(|a| a.operation == architect::team::TaskOperation::Execute)
    );
    let control = serde_json::json!({"scope_hashes":{"step_team":package.scope_hash}});
    let mut app = App::default();
    app.update(Snapshot::from_document(
        &serde_json::to_value(&installed).unwrap(),
        &control,
    ));
    let (action, _) = app.next_approval().unwrap();
    assert!(matches!(action, Action::StepTeam { .. }));
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::NONE,
    )));
    let View::Confirm { details, .. } = &app.view else {
        panic!("wave card missing")
    };
    assert!(details.contains(&package.actions[0].task.description));
    assert!(details.contains(&package.actions[0].profile.profile));
    assert!(details.contains(&installed.spec.workspace));
    assert!(details.contains(&package.scope_hash));
    let frozen = details.clone();
    let mut running = installed.clone();
    running.status = Status::Running;
    app.update(Snapshot::from_document(
        &serde_json::to_value(&running).unwrap(),
        &control,
    ));
    assert!(app.next_approval().is_none());
    assert!(app.snapshot.pending_wave.is_none());
    let View::Confirm { details, .. } = &app.view else {
        panic!("confirmation replaced")
    };
    assert_eq!(details, &frozen);
    assert_eq!(
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        ))),
        vec![Effect::Action(action)]
    );
    let executed = raw_team_step(&f, &installed);
    let reviews = wave::pending(&executed).unwrap();
    assert!(
        reviews
            .actions
            .iter()
            .all(|a| a.operation == architect::team::TaskOperation::Verify)
    );
    assert!(
        reviews
            .actions
            .iter()
            .all(|a| a.write_scopes.is_empty() && a.profile.role == RoleKind::Verifier)
    );
    let reviewed = raw_team_step(&f, &executed);
    let packages = stage::pending(&reviewed);
    assert!(!packages.is_empty());
    let control = serde_json::json!({"team_stage_results":packages,"scope_hashes":{"step_team":reviewed.team_step_scope().unwrap()}});
    app.update(Snapshot::from_document(
        &serde_json::to_value(&reviewed).unwrap(),
        &control,
    ));
    assert!(matches!(
        app.next_approval().unwrap().0,
        Action::AcceptTeamStage { .. }
    ));
    let mut uncertain = executed.clone();
    uncertain.team.as_mut().unwrap().checkpoint_uncertain = true;
    assert!(wave::pending(&uncertain).is_err());
    let mut uncertain = executed;
    uncertain.reconciliation_required = true;
    assert!(wave::pending(&uncertain).is_err());
}

#[test]
fn accepted_complete_managed_team_delivers_only_its_owned_change_with_separate_source_approval() {
    complete_managed_delivery(false);
}
#[test]
fn accepted_complete_managed_team_file_operations_deliver_authenticated_creation_and_deletion() {
    complete_managed_delivery(true);
}
fn complete_managed_delivery(file_operations: bool) {
    let mut f = TeamFixture::new_options(
        if file_operations {
            "managed_delivery_files"
        } else {
            "managed_delivery"
        },
        false,
        true,
    );
    let original = f.run.spec.workspace.clone();
    f.runtime
        .dispatch(
            &id(),
            Command::InspectWorkspace {
                run_id: f.run.id.clone(),
            },
        )
        .unwrap();
    let inspected = f.wait_for_idle();
    let plan = inspected.workspace_plan.as_ref().unwrap();
    f.runtime
        .dispatch(
            &id(),
            Command::PrepareWorkspace {
                run_id: f.run.id.clone(),
                selected: plan.selectors.clone(),
                scope_hash: plan.scope_hash.clone(),
            },
        )
        .unwrap();
    let captured = f.wait_for_idle();
    f.runtime
        .dispatch(
            &id(),
            Command::ActivateWorkspace {
                run_id: f.run.id.clone(),
                scope_hash: architect::workspace::managed::activation_scope(&captured).unwrap(),
            },
        )
        .unwrap();
    f.run = f.wait_for_idle();
    let mut run = f.install_team();
    for _ in 0..120 {
        run = accept_fixture_results(&f, run);
        assert!(
            !run.reconciliation_required,
            "complete managed fixture requires reconciliation: {:?}; tasks {:?}",
            run.diagnostic,
            run.team
                .as_ref()
                .unwrap()
                .tasks
                .iter()
                .filter(|t| t.state != TaskState::Verified)
                .map(|t| (&t.id, &t.state, &t.diagnostic))
                .collect::<Vec<_>>()
        );
        if run.status == Status::Completed {
            break;
        }
        let refresh = run
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|task| run.task_source_refresh_scope(&task.id).is_ok())
            .map(|task| task.id.clone());
        if let Some(task) = refresh {
            run = refresh_sources(&f, &run, &task);
        } else {
            run = step(&f, &run);
        }
        assert!(
            matches!(
                run.status,
                Status::Ready
                    | Status::AwaitingResult
                    | Status::Completed
                    | Status::ChangesRequested
                    | Status::Blocked
            ),
            "{:?}",
            run.diagnostic
        );
    }
    assert_eq!(run.status, Status::Completed, "{:?}", run.diagnostic);
    assert_eq!(
        architect::workspace::inputs::captured_text(
            std::path::Path::new(&run.spec.workspace),
            &run.input_snapshot,
            "source.txt"
        )
        .unwrap()
        .as_deref(),
        Some("managed result")
    );
    if file_operations {
        assert_eq!(
            std::fs::read_to_string(std::path::Path::new(&original).join("obsolete.txt")).unwrap(),
            "original obsolete"
        );
        assert!(!std::path::Path::new(&original).join("new/sub.txt").exists());
    }
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&original).join("source.txt")).unwrap(),
        "original input"
    );
    f.runtime
        .dispatch(
            &id(),
            Command::InspectDelivery {
                run_id: run.id.clone(),
            },
        )
        .unwrap();
    let inspected = f.wait_for_idle();
    let proposal = architect::runtime::delivery::pending(&inspected).unwrap();
    assert!(proposal.conflicts.is_empty(), "{:?}", proposal.conflicts);
    assert_eq!(proposal.changes.len(), if file_operations { 3 } else { 1 });
    assert!(proposal.changes.iter().any(|c| c.path == "source.txt"));
    f.runtime
        .dispatch(
            &id(),
            Command::DeliverSource {
                run_id: run.id.clone(),
                scope_hash: proposal.scope_hash.clone(),
            },
        )
        .unwrap();
    let delivered = f.wait_for_idle();
    assert_eq!(
        delivered.delivery.as_ref().unwrap().state,
        architect::runtime::delivery::State::Delivered,
        "{:?}",
        delivered.diagnostic
    );
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&original).join("source.txt")).unwrap(),
        "managed result"
    );
    if file_operations {
        assert_eq!(
            std::fs::read_to_string(std::path::Path::new(&original).join("new/sub.txt")).unwrap(),
            "created result"
        );
        assert!(
            !std::path::Path::new(&original)
                .join("obsolete.txt")
                .exists()
        );
        let deleted = &delivered.delivery.as_ref().unwrap().effects["obsolete.txt"];
        assert_eq!(
            std::fs::read_to_string(std::path::Path::new(&original).join(&deleted.backup)).unwrap(),
            "original obsolete"
        );
    }
    let effect = &delivered.delivery.as_ref().unwrap().effects["source.txt"];
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&original).join(&effect.backup)).unwrap(),
        "original input"
    );
    assert!(
        delivered
            .approvals
            .iter()
            .any(|a| a.action == "deliver-source")
    );
    assert!(
        f.runtime
            .dispatch(
                &id(),
                Command::DeliverSource {
                    run_id: run.id.clone(),
                    scope_hash: proposal.scope_hash.clone()
                }
            )
            .is_err()
    );
}

#[test]
fn native_create_delete_outcome_loss_is_observed_and_restored_without_repeating_effects() {
    let f = TeamFixture::new_options("managed_delivery_files", false, false);
    let ready = before_managed_change(&f);
    let connection = rusqlite::Connection::open(f.root.path().join("state.db")).unwrap();
    connection.execute_batch("CREATE TRIGGER refuse_file_outcome BEFORE INSERT ON events WHEN NEW.kind='task_execution_applied' BEGIN SELECT RAISE(ABORT, 'injected completed file-operation outcome failure'); END;").unwrap();
    let interrupted = raw_team_step(&f, &ready);
    assert_eq!(interrupted.status, Status::Blocked);
    assert!(interrupted.reconciliation_required);
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/new/sub.txt")).unwrap(),
        "created result"
    );
    assert!(!f.root.path().join("workspace/obsolete.txt").exists());
    let task_id = interrupted
        .task_executions
        .iter()
        .find(|e| !e.receipts.is_empty())
        .unwrap()
        .task_id
        .clone();
    let scope_hash = interrupted
        .task_effect_reconciliation_scope(&task_id)
        .unwrap();
    let calls = std::fs::read(f.root.path().join("calls.jsonl")).unwrap();
    std::fs::write(f.root.path().join("workspace/unowned.txt"), "new user work").unwrap();
    assert!(
        f.current()
            .task_effect_reconciliation_scope(&task_id)
            .is_err()
    );
    std::fs::remove_file(f.root.path().join("workspace/unowned.txt")).unwrap();
    connection
        .execute_batch("DROP TRIGGER refuse_file_outcome")
        .unwrap();
    let observed = f
        .runtime
        .dispatch(
            &id(),
            Command::ReconcileTaskEffects {
                run_id: interrupted.id.clone(),
                task_id: task_id.clone(),
                scope_hash,
            },
        )
        .unwrap();
    assert!(!observed.reconciliation_required);
    let restored = f
        .runtime
        .dispatch(
            &id(),
            Command::RestoreTaskResult {
                run_id: observed.id.clone(),
                task_id: task_id.clone(),
                scope_hash: observed.task_result_restore_scope(&task_id).unwrap(),
            },
        )
        .unwrap();
    assert!(
        restored
            .team
            .unwrap()
            .tasks
            .iter()
            .any(|t| t.id == task_id && t.state == TaskState::Stale && t.artifact.is_some())
    );
    assert_eq!(
        restored
            .task_executions
            .iter()
            .find(|e| e.task_id == task_id)
            .unwrap()
            .state,
        architect::runtime::team::effects::ExecutionState::Committed
    );
    assert_eq!(restored.task_result_restorations.len(), 1);
    assert_eq!(
        std::fs::read(f.root.path().join("calls.jsonl")).unwrap(),
        calls
    );
    assert_eq!(
        std::fs::read_to_string(f.root.path().join("workspace/new/sub.txt")).unwrap(),
        "created result"
    );
    assert!(!f.root.path().join("workspace/obsolete.txt").exists());
}
