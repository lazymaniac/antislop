"""Offline adversarial fixtures; no models, daemon, compiler or live state."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'scripts/selfhost_verify.py'
MODULE = importlib.util.spec_from_file_location('selfhost_verify', SCRIPT)
verify = importlib.util.module_from_spec(MODULE)
MODULE.loader.exec_module(verify)


def encoded(value, **kwargs):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':'), **kwargs)


def sha(value):
    return hashlib.sha256(value.encode() if isinstance(value, str) else value).hexdigest()


class Acceptance(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        source = self.root / 'source'
        (source / 'src').mkdir(parents=True)
        (self.root / 'state').mkdir()
        original = b'fn run() {\n    let verb = "help";\n    println!("old command");\n}\n'
        (source / 'src/main.rs').write_bytes(original)
        (source / 'Cargo.toml').write_text('[package]\nversion="0.1.0"\n')
        (source / 'selfhost_environment.json').write_text('{}\n')
        (source / 'selfhost_check.py').write_text(
            'from pathlib import Path\n(Path(__file__).resolve().parent.parent / "checker-executed").write_text("yes")\n')
        self.before = {str(path.relative_to(source)): sha(path.read_bytes()) for path in source.rglob('*') if path.is_file()}
        (self.root / 'source-hashes.json').write_text(json.dumps(self.before))
        (source / 'src/main.rs').write_bytes(original.replace(b'    println!', verify.VERSION_PATCH + b'    println!', 1))
        spec = dict(objective='Implement version', workspace=str(source), provider='codex',
                    requirements=[dict(id='REQ-VERSION', criteria=dict(version='exact', regression='preserved'))],
                    plan_items=[dict(id='PLAN-VERSION')], editable_files=['src/main.rs'],
                    input_files=sorted(self.before), check_command=[sys.executable, str(source / 'selfhost_check.py')])
        (self.root / 'spec.json').write_text(json.dumps(spec))
        (self.root / 'baseline.json').write_text('{"exit_code":1}')
        snapshot = {path: (source / path).read_text() for path in spec['input_files']}
        snapshot_text = encoded(snapshot, sort_keys=True)
        self.input_hash = sha(snapshot_text)
        sources = {f'file:{path}': sha(text) for path, text in snapshot.items()}
        sources['objective'] = sha(spec['objective'])
        sources['requirement:REQ-VERSION'] = sha(encoded(spec['requirements'][0]))
        sources['plan:PLAN-VERSION'] = sha(encoded(spec['plan_items'][0]))
        self.run = dict(id='00000000-0000-4000-8000-000000000001', status='completed', stage='delivery',
                        spec_revision=1, reconciliation_required=False, active_session=None, pending_edits=[],
                        spec=spec, input_snapshot=snapshot, checklist_version='real-checklist',
                        artifacts=[], evidence=[], reviews=[], approvals=[], provenance_reviews={}, quality_gates={},
                        session_ledger=dict(sessions={}, turns={}))
        self.run['evidence'].append(dict(id='snapshot', source='runtime:input-snapshot', spec_revision=1,
                                        artifact_hash=self.input_hash, input_hash=self.input_hash,
                                        success=True, output=snapshot_text, output_hash=self.input_hash))
        for stage in verify.STAGES:
            content = encoded(dict(artifact=f'{stage} model output', edits=[]))
            self.run['artifacts'].append(dict(stage=stage, spec_revision=1, session_id=f'{stage}-executor',
                                              hash=sha(content), content=content))
        for artifact in self.run['artifacts']:
            if artifact['stage'] in ('execution', 'validation', 'integration'):
                output = encoded(dict(argv=spec['check_command'], exit_code=0, truncated=False, stdout='tests passed', stderr=''))
                self.run['evidence'].append(dict(id=f'runner-{artifact["stage"]}', source='runner:' + encoded(spec['check_command']),
                                                spec_revision=1, artifact_hash=artifact['hash'], input_hash=self.input_hash,
                                                success=True, output=output, output_hash=sha(output)))
        for index, artifact in enumerate(self.run['artifacts']):
            stage = artifact['stage']
            quality = dict(synthetic=False, spec_revision=1,
                           source_hashes=[dict(source_ref=key, source_hash=value) for key, value in sources.items()])
            ids = ['REQ:REQ-VERSION:version', 'REQ:REQ-VERSION:regression', 'PLAN:PLAN-VERSION']
            ids.extend(f'V{number:02}' for number in range(1, 9))
            checks = [dict(check_id=check_id, status='pass', evidence_ids=['runner-execution'],
                           method='Inspect actual pinned source and runner', rationale='The recorded evidence supports this scoped check.')
                      for check_id in ids]
            review = dict(artifact_hash=artifact['hash'], spec_revision=1, executor_session=f'{stage}-executor',
                          verifier_session=f'{stage}-verifier', quality=quality, checks=checks, findings=[],
                          checklist_version='real-checklist')
            for offset, role in enumerate(('executor', 'verifier')):
                logical = f'{stage}-{role}'
                turn_id = logical + '-turn'
                output = artifact['content'] if role == 'executor' else encoded(dict(checks=checks, findings=[], quality=quality))
                scope = dict(provider='codex', role=role, stage=stage, spec_revision=1,
                             workspace=str(source), input_hash=self.input_hash)
                self.run['session_ledger']['sessions'][logical] = dict(id=logical, scope=scope,
                    state='completed', native_observed=True, native_session_id=f'00000000-0000-4000-8000-{index*2+offset+1:012d}', current_turn_id=turn_id)
                self.run['session_ledger']['turns'][turn_id] = dict(id=turn_id, session_id=logical,
                    state='completed', provider_completed=True, output=output,
                    output_ref=dict(document=f'turn:{turn_id}', hash=sha(output)),
                    failure=None, cancellation_requested=False)
            self.run['reviews'].append(review)
            review_hash = sha(encoded(review))
            self.run['provenance_reviews'][review_hash] = copy.deepcopy(review)
            self.run['approvals'].append(dict(action='accept', spec_revision=1))
            if stage == 'delivery':
                for gate in ('requirements_complete:delivery', 'review_complete:delivery'):
                    self.run['quality_gates'][gate] = dict(status='pass', review_hash=review_hash,
                        artifact_hash=artifact['hash'], source_hashes=sources)

    def rejected(self, expected):
        with self.assertRaisesRegex(verify.Rejected, expected):
            verify.validate_export(self.root, self.run)

    def test_validated_replacement_can_complete_after_failed_attempt_five(self):
        sessions = self.run['session_ledger']['sessions']
        turns = self.run['session_ledger']['turns']
        sessions['failed-attempt-5'] = dict(id='failed-attempt-5', scope=dict(provider='codex'),
            native_session_id='00000000-0000-4000-8000-000000000099', state='failed')
        turns['failed-attempt-5-turn'] = dict(id='failed-attempt-5-turn', session_id='failed-attempt-5',
            state='failed', failure='process')
        report = verify.validate_export(self.root, self.run)
        self.assertEqual(len(report['native_session_pairs']), 6)
        self.assertEqual(report['historical_noncompleted_turns'][0]['failure'], 'process')

    def test_not_completed_never_executes_checker(self):
        self.run['status'] = 'blocked'
        exported = self.root / 'run.json'
        exported.write_text(json.dumps(self.run))
        output = subprocess.run([sys.executable, str(SCRIPT), str(self.root), str(exported),
                                 '--output-dir', str(self.root / 'report')], capture_output=True)
        self.assertNotEqual(output.returncode, 0)
        self.assertFalse((self.root / 'checker-executed').exists())
        self.assertFalse(json.loads((self.root / 'report/verification-report.json').read_text())['final_acceptance_passed'])

    def test_no_change_is_rejected(self):
        main = self.root / 'source/src/main.rs'
        main.write_bytes(main.read_bytes().replace(verify.VERSION_PATCH, b''))
        self.rejected('unexpected source changes')

    def test_foreign_source_change_is_rejected(self):
        (self.root / 'source/Cargo.toml').write_text('foreign edit')
        self.rejected('unexpected source changes')

    def test_extra_edit_in_main_is_rejected(self):
        with (self.root / 'source/src/main.rs').open('a') as handle:
            handle.write('// unrelated patch\n')
        self.rejected('exactly the isolated')

    def test_forged_receipt_hash_or_exit_is_rejected(self):
        receipt = self.run['evidence'][1]
        original = copy.deepcopy(receipt)
        receipt['output'] += ' forged'
        self.rejected('receipt output hash mismatch')
        receipt.update(original)
        result = json.loads(receipt['output'])
        result['exit_code'] = 1
        receipt['output'] = encoded(result)
        receipt['output_hash'] = sha(receipt['output'])
        self.rejected('forged successful runner')

    def test_reused_native_session_is_rejected(self):
        sessions = self.run['session_ledger']['sessions']
        sessions['delivery-verifier']['native_session_id'] = sessions['delivery-executor']['native_session_id']
        self.rejected('native session identity reused')

    def test_forged_durable_output_is_rejected(self):
        self.run['session_ledger']['turns']['delivery-verifier-turn']['output'] = '{}'
        self.rejected('durable output binding')

    def test_retained_latest_session_reference_is_completed_history(self):
        self.run['active_session'] = 'delivery-verifier'
        verify.validate_export(self.root, self.run)
        self.run['session_ledger']['sessions']['delivery-verifier']['state'] = 'failed'
        self.rejected('latest session reference is not completed')

    def test_refresh_session_keeps_actual_artifact_stage_binding(self):
        self.run['session_ledger']['sessions']['planning-verifier']['scope']['stage'] = 'execution'
        self.rejected('differently scoped session')

    def test_raw_quality_set_order_follows_serde_without_ignoring_vector_order(self):
        review = self.run['reviews'][1]
        self.run['provenance_reviews'].pop(sha(encoded(review)))
        review['quality']['planning'] = dict(scenario_kinds=['counterexample', 'success'])
        self.run['provenance_reviews'][sha(encoded(review))] = copy.deepcopy(review)
        turn = self.run['session_ledger']['turns']['planning-verifier-turn']
        raw = dict(checks=copy.deepcopy(review['checks']), findings=[], quality=copy.deepcopy(review['quality']))
        raw['quality']['planning']['scenario_kinds'].reverse()
        turn['output'] = encoded(raw)
        turn['output_ref']['hash'] = sha(turn['output'])
        verify.validate_export(self.root, self.run)
        raw['checks'].reverse()
        turn['output'] = encoded(raw)
        turn['output_ref']['hash'] = sha(turn['output'])
        self.rejected('review differs from durable')

    def test_final_checker_runs_only_after_valid_export(self):
        exported = self.root / 'run.json'
        exported.write_text(json.dumps(self.run))
        result = subprocess.run([sys.executable, str(SCRIPT), str(self.root), str(exported),
                                 '--output-dir', str(self.root / 'report')], capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout.decode() + result.stderr.decode())
        self.assertTrue((self.root / 'checker-executed').exists())


if __name__ == '__main__':
    unittest.main()
