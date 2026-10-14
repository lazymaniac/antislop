#!/usr/bin/env python3
"""Independently check an exported self-host run; never resume or mutate its state."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import uuid

STAGES = ('discovery', 'planning', 'execution', 'validation', 'integration', 'delivery')
VERSION_PATCH = '''    if verb == "--version" {
        println!("antislop {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
'''.encode()


class Rejected(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise Rejected(message)


def digest(value):
    return hashlib.sha256(value.encode() if isinstance(value, str) else value).hexdigest()


def compact(value, *, sort_keys=False):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':'), sort_keys=sort_keys)


def load(path):
    return load_json(Path(path).read_text())


def fingerprints(workspace):
    hashes = {}
    for path in workspace.rglob('*'):
        relative = path.relative_to(workspace)
        if relative.parts[0] == 'target' or '__pycache__' in relative.parts:
            continue
        require(not path.is_symlink(), f'source symlink is outside the accepted patch: {relative}')
        if path.is_file():
            hashes[str(relative)] = digest(path.read_bytes())
    return hashes


def current_sources(run):
    sources = {f'file:{path}': digest(text) for path, text in run['input_snapshot'].items()}
    sources['objective'] = digest(run['spec']['objective'])
    for name, prefix in (('requirements', 'requirement'), ('plan_items', 'plan')):
        for item in run['spec'][name]:
            sources[f'{prefix}:{item["id"]}'] = digest(compact(item))
    return sources


def typed_quality(value):
    """Apply only the documented BTreeSet normalization in quality.rs.

    Vector order and every report value remain significant. The CLI's raw JSON
    output stays hash-bound; serde legitimately sorts/deduplicates these sets.
    """
    sets = {'source_refs', 'option_ids', 'scenario_kinds', 'requirement_ids',
            'targets', 'configurations', 'inventory', 'scenario_ids', 'input_ids',
            'features', 'dataset_sources', 'brief_coverage', 'risk_ids'}
    if isinstance(value, dict):
        result = {}
        for key, item in value.items():
            if key in sets:
                require(isinstance(item, list) and all(isinstance(entry, str) for entry in item),
                        'quality set is not a string array')
                result[key] = sorted(set(item))
            else:
                result[key] = typed_quality(item)
        return result
    if isinstance(value, list):
        return [typed_quality(item) for item in value]
    return value


def validate_export(root, run, expected_run_id=None):
    """Pure export/source validation, with no checker execution or state writes.

    Hashes and ledger bindings detect inconsistent/forged fields. They are not a
    provider-signed attestation against wholesale fabrication of an entire export.
    """
    return _audit_export(root, run, expected_run_id, final=True)


def audit_saved_evidence(root, run, expected_run_id=None):
    """Check existing source/receipt/session bindings without asserting completion.

    This read-only diagnostic never runs the checker or changes the run status.
    Missing future stages remain explicit; main() uses only validate_export().
    """
    return _audit_export(root, run, expected_run_id, final=False)


def _audit_export(root, run, expected_run_id, *, final):
    root = Path(root).resolve()
    workspace = root / 'source'
    if final:
        require(run['status'] == 'completed' and run['stage'] == 'delivery', 'run is not Completed')
    require(expected_run_id is None or run['id'] == expected_run_id, 'unexpected run ID')
    require(run['spec_revision'] == 1, 'task revision changed')
    require(not run['reconciliation_required'], 'effects still require reconciliation')
    require(all(edit['state'] == 'applied' for edit in run['pending_edits']), 'unresolved edit receipt')
    original_spec = load(root / 'spec.json')
    actual_spec = dict(run['spec'])
    for spec in (original_spec, actual_spec):
        for key, default in (('config', None), ('model', None), ('task_family', 'software')):
            spec.setdefault(key, default)
        # Match RunSpec's serde default without rewriting the audited export.
        spec['plan_items'] = [dict(dependencies=[], **item) if 'dependencies' not in item else dict(item)
                              for item in spec['plan_items']]
    for key, value in original_spec.items():
        if key == 'workspace':
            require(Path(run['spec'][key]).resolve() == workspace, 'workspace changed')
        else:
            require(actual_spec[key] == value, f'immutable task spec changed: {key}')
    require(run['spec']['provider'] == 'codex', 'self-host requires real Codex')
    require(run['spec']['editable_files'] == ['src/main.rs'], 'unexpected editable scope')
    command = run['spec']['check_command']
    require(len(command) == 2 and Path(command[1]).resolve() == workspace / 'selfhost_check.py',
            'runner is not the immutable self-host checker')
    before = load(root / 'source-hashes.json')
    after = fingerprints(workspace)
    changed = sorted(path for path in before.keys() | after.keys() if before.get(path) != after.get(path))
    require(changed == ['src/main.rs'], f'unexpected source changes: {changed}')
    main = (workspace / 'src/main.rs').read_bytes()
    require(main.count(VERSION_PATCH) == 1 and digest(main.replace(VERSION_PATCH, b'', 1)) == before['src/main.rs'],
            'source does not contain exactly the isolated env! version patch')
    baseline = load(root / 'baseline.json')
    require(baseline['exit_code'] != 0, 'baseline already passed the task')
    require(set(run['input_snapshot']) == set(run['spec']['input_files']), 'pinned input set changed')
    for path, text in run['input_snapshot'].items():
        require((workspace / path).read_bytes() == text.encode(), f'input snapshot differs from source: {path}')
    input_hash = digest(compact(run['input_snapshot'], sort_keys=True))
    sources = current_sources(run)

    artifacts = {}
    for artifact in run['artifacts']:
        require(artifact['hash'] == digest(artifact['content']), 'artifact hash mismatch')
        artifacts.setdefault(artifact['hash'], []).append(artifact)
    evidence = {}
    for receipt in run['evidence']:
        require(receipt['id'] not in evidence, 'duplicate receipt ID')
        require(receipt['output_hash'] == digest(receipt['output']), 'receipt output hash mismatch')
        evidence[receipt['id']] = receipt
    require(any(receipt['source'] == 'runtime:input-snapshot'
                and receipt['spec_revision'] == 1 and receipt['success'] is True
                and receipt['input_hash'] == input_hash and receipt['output_hash'] == input_hash
                for receipt in evidence.values()), 'missing actual current input snapshot receipt')
    runners = {}
    runner_source = 'runner:' + compact(command)
    for receipt in evidence.values():
        if receipt['source'] != runner_source or receipt['input_hash'] != input_hash or receipt['spec_revision'] != 1:
            continue
        result = load_json(receipt['output'])
        require(result['argv'] == command, 'runner argv mismatch')
        require(isinstance(result['stdout'], str) and isinstance(result['stderr'], str), 'runner output missing')
        if receipt['success'] is True:
            require(type(result['exit_code']) is int and result['exit_code'] == 0
                    and result['truncated'] is False, 'forged successful runner receipt')
            runners[receipt['id']] = receipt
    require(runners, 'no successful actual runner against current inputs')

    ledger = run['session_ledger']
    sessions, turns = ledger['sessions'], ledger['turns']
    reconciled = any(approval['action'] == 'reconcile' and approval['spec_revision'] == 1
                     for approval in run['approvals']) and any(
        receipt['source'] == 'runtime:edit-reconciliation' and receipt['spec_revision'] == 1
        and receipt['success'] is True for receipt in evidence.values())
    if final:
        for record in (*sessions.values(), *turns.values()):
            require(record['state'] in ('completed', 'failed', 'stopped')
                    or (record['state'] == 'delivery_unknown' and reconciled),
                    'completed run retains an unresolved session or turn')
    native_owners = {}
    for logical_id, session in sessions.items():
        require(logical_id == session['id'], 'managed session map identity mismatch')
        raw = session.get('native_session_id')
        if raw:
            identity = (session['scope']['provider'], str(uuid.UUID(raw)))
            require(identity not in native_owners, 'native session identity reused')
            native_owners[identity] = logical_id
    for turn_id, turn in turns.items():
        require(turn_id == turn['id'] and turn['session_id'] in sessions, 'turn/session binding mismatch')
        if turn['state'] == 'completed':
            require(turn['provider_completed'] is True and isinstance(turn['output'], str)
                    and turn['output_ref'] == {'document': f'turn:{turn_id}', 'hash': digest(turn['output'])},
                    'completed turn has no authentic durable output binding')
    if final and run.get('active_session'):
        active = sessions[run['active_session']]
        require(active['state'] == 'completed'
                and turns[active['current_turn_id']]['state'] == 'completed',
                'latest session reference is not completed')

    def session_turn(logical_id, role, artifact):
        session = sessions[logical_id]
        scope = session['scope']
        turn = turns[session['current_turn_id']]
        require(session['native_observed'] is True and session['state'] == 'completed'
                and scope['provider'] == 'codex' and scope['role'] == role
                and scope['stage'] == artifact['stage'] and scope['spec_revision'] == 1
                and Path(scope['workspace']).resolve() == workspace
                and scope['input_hash'] == input_hash
                and turn['session_id'] == logical_id and turn['state'] == 'completed'
                and turn['failure'] is None and not turn['cancellation_requested'],
                'selected review uses an incomplete, fake or differently scoped session')
        identity = ('codex', str(uuid.UUID(session['native_session_id'])))
        return identity, turn

    selected = {}
    registered = run['provenance_reviews']
    for review in run['reviews']:
        review_hash = digest(compact(review))
        if registered.get(review_hash) != review or review['findings'] or review['spec_revision'] != 1:
            continue
        candidates = artifacts[review['artifact_hash']]
        artifact = next((candidate for candidate in reversed(candidates)
                         if candidate['session_id'] == review['executor_session']), None)
        require(artifact is not None, 'review has no matching artifact author')
        selected[artifact['stage']] = (artifact, review, review_hash)
    if final:
        require(set(selected) == set(STAGES), 'not every stage has a validated independent review')
        require(sum(approval['action'] == 'accept' and approval['spec_revision'] == 1
                    for approval in run['approvals']) == 6, 'missing six explicit result acceptances')
    pairs = []

    def eligible_receipt(receipt, artifact):
        if receipt['spec_revision'] != 1:
            return False
        source = receipt['source']
        if source.startswith('runner:baseline:'):
            return True
        if receipt['success'] is not True:
            return False
        if source == 'runtime:artifact-read':
            return receipt['artifact_hash'] == receipt['output_hash'] and receipt['artifact_hash'] in artifacts
        if receipt['input_hash'] != input_hash:
            return False
        if source == 'runtime:input-snapshot':
            return receipt['output_hash'] == input_hash
        if source == 'runtime:orchestration':
            context = load_json(receipt['output'])
            return (receipt['artifact_hash'] == artifact['hash'] and context['run_id'] == run['id']
                    and context['spec_revision'] == 1 and context['attempt'] == artifact['attempt']
                    and context['checklist_version'] == run['checklist_version'])
        if source.startswith('runner:'):
            if source == runner_source:
                return True
            observed = load_json(receipt['output'])
            argv = observed.get('argv')
            action = 'check:' + digest(compact(argv))
            return (isinstance(argv, list) and argv and all(isinstance(item, str) for item in argv)
                    and argv[0] and source == 'runner:' + compact(argv)
                    and type(observed.get('exit_code')) is int and observed['exit_code'] == 0
                    and observed.get('truncated') is False
                    and isinstance(observed.get('stdout'), str) and isinstance(observed.get('stderr'), str)
                    and any(approval['action'] == action and approval['spec_revision'] == 1
                            for approval in run['approvals']))
        return True

    for stage in STAGES:
        if stage not in selected:
            continue
        artifact, review, review_hash = selected[stage]
        require(artifact['spec_revision'] == 1 and artifact['session_id'] == review['executor_session'],
                'artifact/review authorship mismatch')
        quality = review['quality']
        require(quality and quality['synthetic'] is False and quality['spec_revision'] == 1,
                'selected review is synthetic or stale')
        review_sources = {entry['source_ref']: entry['source_hash'] for entry in quality['source_hashes']}
        require(len(review_sources) == len(quality['source_hashes']) and review_sources == sources,
                'selected review does not bind current sources')
        require(review['checklist_version'] == run['checklist_version'], 'stale review checklist')
        executor, execution = session_turn(review['executor_session'], 'executor', artifact)
        verifier, verification = session_turn(review['verifier_session'], 'verifier', artifact)
        require(executor != verifier, 'review is not independent')
        require(execution['output'] == artifact['content'], 'executor output differs from stored artifact')
        raw_review = load_json(verification['output'])
        require(all(raw_review[key] == review[key] for key in ('checks', 'findings'))
                and typed_quality(raw_review['quality']) == typed_quality(review['quality']),
                'review differs from durable verifier output')
        checks = {check['check_id']: check for check in review['checks']}
        require(len(checks) == len(review['checks']), 'duplicate review check ID')
        mandatory = ('REQ:REQ-VERSION:version', 'REQ:REQ-VERSION:regression',
                     'PLAN:PLAN-VERSION', *(f'V{number:02}' for number in range(1, 9)))
        for check_id in mandatory:
            require(checks.get(check_id, {}).get('status') == 'pass', 'task acceptance criterion not passed')
        for check in review['checks']:
            require(check['status'] in ('pass', 'not_applicable'), 'failed or blocked review check')
            require(check['status'] != 'not_applicable'
                    or check['check_id'].startswith(('RISK:', 'SM', 'ROLE:security:', 'ROLE:performance:')),
                    'mandatory review check is not applicable')
            require(isinstance(check.get('method'), str) and check['method'].strip()
                    and isinstance(check.get('rationale'), str) and check['rationale'].strip(),
                    'review check has no method or rationale')
            require(check['evidence_ids'], 'review check has no evidence')
            require(all(evidence_id in evidence for evidence_id in check['evidence_ids']), 'unknown review receipt')
            require(all(eligible_receipt(evidence[evidence_id], artifact)
                        for evidence_id in check['evidence_ids']), 'stale or failed review receipt')
        if stage in ('execution', 'validation', 'integration'):
            require(any(receipt['artifact_hash'] == artifact['hash'] for receipt in runners.values()),
                    f'{stage} has no actual current artifact-bound runner receipt')
        if stage == 'delivery':
            require(any(evidence_id in runners for check in review['checks'] for evidence_id in check['evidence_ids']),
                    'delivery review does not cite actual current runner evidence')
            for gate in ('requirements_complete:delivery', 'review_complete:delivery'):
                record = run['quality_gates'].get(gate, {})
                require(record.get('status') == 'pass' and record.get('review_hash') == review_hash
                        and record.get('artifact_hash') == artifact['hash']
                        and record.get('source_hashes') == sources, 'final quality gate is not bound to this review')
        pairs.append(dict(stage=stage, executor=executor, verifier=verifier))
    failed = []
    for turn_id, turn in turns.items():
        if turn['state'] != 'completed':
            session = sessions[turn['session_id']]
            failed.append(dict(id=turn_id, session_id=turn['session_id'], state=turn['state'],
                               native_session_id=session.get('native_session_id'),
                               native_observed=session.get('native_observed', False),
                               stage=session['scope'].get('stage'), failure=turn.get('failure'),
                               provider_completed=turn.get('provider_completed', False),
                               output_ref=turn.get('output_ref'), usage=turn.get('usage')))
    return dict(schema_version=1, run_id=run['id'], status=run['status'], changed_files=changed,
                completion_required=final, unvalidated_stages=[stage for stage in STAGES if stage not in selected],
                current_input_hash=input_hash, current_runner_ids=sorted(runners),
                native_session_pairs=pairs, historical_noncompleted_turns=failed,
                historical_reviews=len(run['reviews']), final_acceptance_passed=False,
                attestation='Export/source consistency; final checker pending; no provider-signed attestation.')


def load_json(text):
    # Reuse duplicate-key rejection without reading or writing a temporary file.
    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, 'duplicate JSON key')
            result[key] = value
        return result
    return json.loads(text, object_pairs_hook=unique)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('run_json', type=Path)
    parser.add_argument('--run-id')
    parser.add_argument('--output-dir', required=True, type=Path)
    args = parser.parse_args()
    root, output = args.root.resolve(), args.output_dir.resolve()
    for protected in (root / 'source', root / 'state'):
        require(not output.is_relative_to(protected), 'report output must be outside source/state')
    output.mkdir(parents=True, exist_ok=True)
    try:
        run = load(args.run_json)
        report = validate_export(root, run, args.run_id)
        before = fingerprints(root / 'source')
        result = subprocess.run(run['spec']['check_command'], cwd=root / 'source', capture_output=True)
        (output / 'final-check.log').write_bytes(result.stdout + result.stderr)
        require(result.returncode == 0, 'post-completion immutable checker failed')
        require(fingerprints(root / 'source') == before, 'final checker changed source files')
        report['final_acceptance_passed'] = True
        report['final_checker_exit_code'] = result.returncode
        report['attestation'] = 'Export/source consistency and executed immutable checker; no provider-signed attestation.'
    except (Rejected, KeyError, TypeError, ValueError, OSError) as error:
        report = dict(schema_version=1, final_acceptance_passed=False, error=str(error))
        (output / 'verification-report.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(report, indent=2))
        return 1
    (output / 'verification-report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
