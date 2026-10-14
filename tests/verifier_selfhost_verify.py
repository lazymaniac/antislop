"""Independent adversarial checks; disposable offline fixtures only."""
import copy
import importlib.util
from pathlib import Path
import unittest

FIXTURE = importlib.util.spec_from_file_location('base_selfhost_acceptance', Path(__file__).with_name('selfhost_verify.py'))
base = importlib.util.module_from_spec(FIXTURE)
FIXTURE.loader.exec_module(base)
verify = base.verify


class IndependentAcceptance(unittest.TestCase):
    def setUp(self):
        self.fixture = base.Acceptance()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.run = self.fixture.run

    def rebind_delivery(self, edit):
        """Keep hashes/native-output/gates consistent to test the missing rule."""
        review = self.run['reviews'][-1]
        old_hash = verify.digest(verify.compact(review))
        edit(review['checks'])
        review_hash = verify.digest(verify.compact(review))
        self.run['provenance_reviews'].pop(old_hash)
        self.run['provenance_reviews'][review_hash] = copy.deepcopy(review)
        turn = self.run['session_ledger']['turns']['delivery-verifier-turn']
        raw = verify.load_json(turn['output'])
        raw['checks'] = copy.deepcopy(review['checks'])
        turn['output'] = verify.compact(raw)
        turn['output_ref']['hash'] = verify.digest(turn['output'])
        for gate in self.run['quality_gates'].values():
            gate['review_hash'] = review_hash

    @staticmethod
    def check(checks, check_id):
        return next(check for check in checks if check['check_id'] == check_id)

    def test_existing_identity_failure_cannot_hide_behind_consistent_hashes(self):
        self.rebind_delivery(lambda checks: self.check(checks, 'V01').update(status='fail'))
        with self.assertRaisesRegex(verify.Rejected, 'not passed|failed or blocked'):
            verify.validate_export(self.root, self.run)

    def test_existing_mandatory_check_blocked_or_not_applicable_is_rejected(self):
        original = copy.deepcopy(self.run)
        for status in ('blocked', 'not_applicable'):
            with self.subTest(status=status):
                self.run = copy.deepcopy(original)
                self.rebind_delivery(lambda checks: self.check(checks, 'V02').update(status=status))
                with self.assertRaisesRegex(verify.Rejected, 'not passed|failed or blocked|not applicable'):
                    verify.validate_export(self.root, self.run)

    def test_empty_receipts_cannot_support_an_otherwise_passing_check(self):
        self.rebind_delivery(lambda checks: self.check(checks, 'V03').update(evidence_ids=[]))
        with self.assertRaisesRegex(verify.Rejected, 'no evidence'):
            verify.validate_export(self.root, self.run)

    def test_blank_method_or_rationale_is_rejected(self):
        original = copy.deepcopy(self.run)
        for field in ('method', 'rationale'):
            with self.subTest(field=field):
                self.run = copy.deepcopy(original)
                self.rebind_delivery(lambda checks: self.check(checks, 'V04').update({field: ' \t\n'}))
                with self.assertRaisesRegex(verify.Rejected, 'method or rationale'):
                    verify.validate_export(self.root, self.run)

    def test_missing_plan_check_is_rejected(self):
        self.rebind_delivery(lambda checks: checks.remove(self.check(checks, 'PLAN:PLAN-VERSION')))
        with self.assertRaisesRegex(verify.Rejected, 'not passed'):
            verify.validate_export(self.root, self.run)

    def test_existing_stale_runner_cannot_support_current_selected_review(self):
        original = copy.deepcopy(self.run)
        for revision, input_hash in ((0, self.fixture.input_hash), (1, 'old-input-hash')):
            with self.subTest(revision=revision, input_hash=input_hash):
                self.run = copy.deepcopy(original)
                receipt = copy.deepcopy(next(e for e in self.run['evidence'] if e['id'] == 'runner-execution'))
                receipt.update(id='runner-history', spec_revision=revision, input_hash=input_hash)
                self.run['evidence'].append(receipt)
                self.rebind_delivery(lambda checks: self.check(checks, 'V05').update(evidence_ids=['runner-history']))
                with self.assertRaisesRegex(verify.Rejected, 'stale|current|receipt|eligible'):
                    verify.validate_export(self.root, self.run)

    def test_unselected_inflight_native_turn_is_not_completed_history(self):
        self.run['session_ledger']['sessions']['still-running'] = dict(id='still-running',
            scope=dict(provider='codex', stage='execution'),
            native_session_id='00000000-0000-4000-8000-000000000099',
            state='sent', current_turn_id='still-running-turn')
        self.run['session_ledger']['turns']['still-running-turn'] = dict(id='still-running-turn',
            session_id='still-running', state='sent', failure=None, provider_completed=False)
        with self.assertRaisesRegex(verify.Rejected, 'flight|pending|sent|terminal|completed'):
            verify.validate_export(self.root, self.run)

    def test_supplemental_runner_needs_matching_exact_argv_approval(self):
        original = copy.deepcopy(self.run)
        for variant in ('approved', 'unapproved', 'argv-swapped', 'exit-failed', 'truncated'):
            with self.subTest(variant=variant):
                self.run = copy.deepcopy(original)
                authorized = ['python3', '-c', 'print("actual supplemental probe")']
                argv = authorized if variant != 'argv-swapped' else ['python3', '-c', 'print("different probe")']
                output = verify.compact(dict(argv=argv, exit_code=1 if variant == 'exit-failed' else 0,
                    truncated=variant == 'truncated', stdout='probe result', stderr=''))
                receipt = dict(id='supplemental-runner', source='runner:' + verify.compact(argv),
                    spec_revision=1, input_hash=self.fixture.input_hash,
                    artifact_hash=self.run['artifacts'][-1]['hash'], success=True,
                    output=output, output_hash=verify.digest(output))
                self.run['evidence'].append(receipt)
                if variant != 'unapproved':
                    self.run['approvals'].append(dict(action='check:' + verify.digest(verify.compact(authorized)), spec_revision=1))
                self.rebind_delivery(lambda checks: self.check(checks, 'V07').update(evidence_ids=[receipt['id']]))
                if variant == 'approved':
                    verify.validate_export(self.root, self.run)
                else:
                    with self.assertRaisesRegex(verify.Rejected, 'stale|failed|receipt|runner|successful'):
                        verify.validate_export(self.root, self.run)

    def test_frozen_orchestration_receipt_uses_artifact_attempt_not_final_attempt(self):
        self.run['attempt'] = 99
        self.run['artifacts'][-1]['attempt'] = 5
        context = dict(run_id=self.run['id'], spec_revision=1, attempt=5,
                       checklist_version=self.run['checklist_version'])
        output = verify.compact(context)
        receipt = dict(id='review-time-orchestration', spec_revision=1,
            source='runtime:orchestration', artifact_hash=self.run['artifacts'][-1]['hash'],
            input_hash=self.fixture.input_hash, success=True, output=output, output_hash=verify.digest(output))
        self.run['evidence'].append(receipt)
        self.rebind_delivery(lambda checks: self.check(checks, 'V06').update(evidence_ids=[receipt['id']]))
        verify.validate_export(self.root, self.run)
        context['attempt'] = 99
        receipt['output'] = verify.compact(context)
        receipt['output_hash'] = verify.digest(receipt['output'])
        with self.assertRaisesRegex(verify.Rejected, 'stale|receipt'):
            verify.validate_export(self.root, self.run)

    def test_delivery_unknown_history_needs_explicit_reconciliation_evidence(self):
        self.run['session_ledger']['sessions']['unknown-history'] = dict(id='unknown-history',
            scope=dict(provider='codex', stage='execution'),
            native_session_id='00000000-0000-4000-8000-000000000099',
            state='delivery_unknown', current_turn_id='unknown-history-turn')
        self.run['session_ledger']['turns']['unknown-history-turn'] = dict(id='unknown-history-turn',
            session_id='unknown-history', state='delivery_unknown', failure=None, provider_completed=False)
        with self.assertRaisesRegex(verify.Rejected, 'unresolved'):
            verify.validate_export(self.root, self.run)
        self.run['approvals'].append(dict(action='reconcile', spec_revision=1))
        with self.assertRaisesRegex(verify.Rejected, 'unresolved'):
            verify.validate_export(self.root, self.run)
        self.run['evidence'].append(dict(id='reconciled-effects', source='runtime:edit-reconciliation',
            spec_revision=1, artifact_hash=verify.digest('{}'), input_hash=self.fixture.input_hash,
            success=True, output='{}', output_hash=verify.digest('{}')))
        report = verify.validate_export(self.root, self.run)
        self.assertEqual(report['historical_noncompleted_turns'][0]['state'], 'delivery_unknown')

    def test_allowed_risk_not_applicable_keeps_a_valid_completed_replacement(self):
        self.rebind_delivery(lambda checks: checks.append(dict(check_id='RISK:R001', status='not_applicable',
            evidence_ids=['snapshot'], method='Inspect pinned scope',
            rationale='Risk is outside this isolated version-dispatch change.')))
        report = verify.validate_export(self.root, self.run)
        self.assertEqual(len(report['native_session_pairs']), 6)
        self.assertFalse(report['final_acceptance_passed'], 'pure audit must not claim checker execution')

    def test_failed_historical_turn_does_not_replace_the_completed_native_pair(self):
        self.run['session_ledger']['sessions']['failed-history'] = dict(id='failed-history',
            scope=dict(provider='codex', stage='execution'), native_session_id='00000000-0000-4000-8000-000000000099', state='failed')
        self.run['session_ledger']['turns']['failed-history-turn'] = dict(id='failed-history-turn',
            session_id='failed-history', state='failed', failure='process', provider_completed=False)
        report = verify.validate_export(self.root, self.run)
        pair = next(pair for pair in report['native_session_pairs'] if pair['stage'] == 'execution')
        self.assertNotIn('00000000-0000-4000-8000-000000000099', [pair['executor'][1], pair['verifier'][1]])
        self.assertEqual(report['historical_noncompleted_turns'][0]['id'], 'failed-history-turn')


if __name__ == '__main__':
    unittest.main()
