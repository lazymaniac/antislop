"""Seed omissions and qualifier loss; test behavior, not the extractor's output shape."""
import copy
import importlib.util
import json
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


def module(name):
    spec = importlib.util.spec_from_file_location(name, ROOT/'scripts'/f'{name}.py')
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


extractor = module('requirements')
auditor = module('audit_requirements')


class RequirementsAuditTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = (ROOT/'design.md').read_text()
        cls.ledger, _ = extractor.extract(cls.source)
        cls.atomic = extractor.atomic_extract(cls.source, cls.ledger)

    def mutate(self):
        return copy.deepcopy(self.ledger), copy.deepcopy(self.atomic)

    def errors(self, ledger, atomic, source=None):
        return auditor.audit(source or self.source, ledger, atomic)['errors']

    def test_pinned_source_and_critical_interpretations_pass(self):
        result = auditor.audit(self.source, self.ledger, self.atomic)
        self.assertEqual([], result['errors'])
        self.assertFalse(result['completeness_verified'])
        self.assertGreater(result['scope']['remaining_parent_reviews'], 0)

    def test_omitted_middle_source_fragment_fails(self):
        ledger, atomic = self.mutate()
        del atomic['parents'][len(atomic['parents'])//2]
        self.assertTrue(any('omission' in error for error in self.errors(ledger, atomic)))

    def test_orphan_child_fails(self):
        ledger, atomic = self.mutate()
        atomic['requirements'][0]['parent_id'] = 'absent'
        self.assertTrue(self.errors(ledger, atomic))

    def test_missing_semantic_obligation_fails_even_with_source_preserved(self):
        ledger, atomic = self.mutate()
        child = next(c for c in atomic['requirements'] if c['obligation'] == 'Preserve negative requirements and acceptance details from the source.')
        child['obligation'] = 'Preserve some requirements.'
        self.assertTrue(any('semantic obligation/qualifier missing' in error for error in self.errors(ledger, atomic)))

    def test_context_packet_qualifiers_cannot_be_lost_in_an_otherwise_preserved_source(self):
        for phrase in [
            'references to the complete source materials',
            'prohibition on self-authorized permission changes',
            'After compaction, verify',
            'examples, not an exhaustive list',
            'explicit decision',
            'discovered document or webpage',
        ]:
            with self.subTest(phrase=phrase):
                ledger, atomic = self.mutate()
                child = next(c for c in atomic['requirements'] if phrase in c['obligation'])
                child['obligation'] = 'Retain some context.'
                self.assertTrue(any('semantic obligation/qualifier missing' in error for error in self.errors(ledger, atomic)))

    def test_autonomy_interpretation_preserves_approvals_gate_conditions_and_no_retroactivity(self):
        for phrase in [
            'optional combined approval, not implicit authorization',
            'do not automatically open a correction round after a verifier rejects',
            'opens successive stages after their applicable gates are satisfied',
            'opens correction rounds after their applicable gates are satisfied',
            'local commits within the policy',
            'necessary product decision may require the user',
            'Do not retrospectively approve prior changes',
            'Do not remove verifier concerns',
            'scope invalidates the affected approvals',
            'approval of a stage from approval of an action',
            'must not include unrelated user changes',
        ]:
            with self.subTest(phrase=phrase):
                ledger,atomic=self.mutate()
                child=next(c for c in atomic['requirements'] if phrase in c['obligation'])
                child['obligation']='A selected mode allows the workflow to proceed.'
                self.assertTrue(any('semantic obligation/qualifier missing' in error for error in self.errors(ledger,atomic)))

    def test_team_adaptation_preserves_mandatory_pairs_preparatory_exceptions_and_no_numeric_limits(self):
        for phrase in [
            'available CLIs', 'agents without a task', 'appropriate stage in supervised mode',
            'Do not require a completed implementation plan', 'Do not waive implementation gates',
            'code and final-integration evidence before delivery', 'management delegation graph is acyclic',
            'Every active write has a defined integration method', 'no numerical round limit',
            'no numerical agent limit', 'must not create an infinite execution loop',
            'Every run includes a compliance pair', 'test-implementation pair',
            'brief but remain mandatory', 'does not require fictitious compilation tests',
            'Do not share one verifier agent', 'mandatory refactoring assessment',
            'cannot remove required verification', 'composition change has a history event',
        ]:
            with self.subTest(phrase=phrase):
                ledger,atomic=self.mutate()
                child=next(c for c in atomic['requirements'] if phrase in c['obligation'])
                child['obligation']='A generic team may do some work.'
                self.assertTrue(any('semantic obligation/qualifier missing' in error for error in self.errors(ledger,atomic)))

    def test_team_catalog_interpretation_retains_source_authority_without_claiming_implementation(self):
        prefix='To katalog kompetencji. Każdy run obejmuje'
        parent=next(p for p in self.atomic['parents'] if p['original_text'].startswith(prefix))
        children=[c for c in self.atomic['requirements'] if c['parent_id']==parent['id']]
        self.assertEqual(20,len(children))
        self.assertTrue(all(c['authority']==parent['authority'] and c['atomicity']=='reviewed'
                            and c['status']=='identified' and not c['evidence_refs'] for c in children))

    def test_bugfix_requires_actual_before_after_proof_or_an_explicit_gap(self):
        for phrase in ['reproduce the problem before changing', 'check the result after the change',
                       'explicit gap requiring other evidence', 'inability to reproduce as implicit success']:
            with self.subTest(phrase=phrase):
                ledger,atomic=self.mutate()
                child=next(c for c in atomic['requirements'] if phrase in c['obligation'])
                child['obligation']='An assumed test result is enough.'
                self.assertTrue(any('semantic obligation/qualifier missing' in error for error in self.errors(ledger,atomic)))

    def test_role_catalog_review_retains_dimensions_and_illustrative_artifact_qualifiers(self):
        for phrase in ['substantive strategy changes', 'closure of unknowns', 'acceptance-criteria completeness',
                       'independent simulation', 'comparison fairness', 'reuse of existing solutions',
                       'dependency maintenance cost', 'absence of unnecessary additions',
                       'absence of speculative abstractions', 'absence of rule-weakening attempts',
                       'absence of extraction gaps', 'scenarios detect probable defects', 'test resilience',
                       'inspect false alarms', 'absence of semantic regression', 'entire change set',
                       'facts from inferences', 'inspect units', 'alignment with the brief',
                       'instructions with the actual result', 'ComplianceAudit and RequirementLedger accounting']:
            with self.subTest(phrase=phrase):
                ledger,atomic=self.mutate()
                child=next(c for c in atomic['requirements'] if phrase in c['obligation'])
                child['obligation']='Perform a generic role review.'
                self.assertTrue(any('semantic obligation/qualifier missing' in error for error in self.errors(ledger,atomic)))
        parents=[p for p in self.atomic['parents'] if p['section']=='7.2' and p['original_text'].startswith('|') and p['child_ids']]
        self.assertEqual(20,len(parents))
        for parent in parents:
            children=[c for c in self.atomic['requirements'] if c['parent_id']==parent['id']]
            self.assertTrue(all(c['atomicity']=='reviewed' and c['status']=='identified'
                                and c['authority']==parent['authority'] and not c['evidence_refs'] for c in children))
            examples=next(c for c in children if 'illustrative artifact form' in c['obligation'])
            ledger,atomic=self.mutate()
            next(c for c in atomic['requirements'] if c['id']==examples['id'])['obligation']='Require this exact artifact storage format for every task.'
            self.assertTrue(any('semantic obligation/qualifier missing' in error for error in self.errors(ledger,atomic)))

    def test_role_catalog_cannot_force_every_catalog_competency_into_every_run(self):
        ledger,atomic=self.mutate()
        child=next(c for c in atomic['requirements'] if c['obligation'].startswith('For the selected Security analyst competency, independently inspect the threat model.'))
        child['obligation']=child['obligation'].replace('For the selected Security analyst competency,','For every run regardless of selected competencies,')
        self.assertTrue(any('selected-competency applicability' in error for error in self.errors(ledger,atomic)))

    def test_context_source_and_packet_fields_are_individually_traceable_without_claiming_implementation(self):
        for prefix, count in [('Trwałe źródła: wymagania', 14), ('Pakiet zawiera obowiązkowo:', 14), ('Instrukcje projektu (np.', 3)]:
            parent = next(p for p in self.atomic['parents'] if p['original_text'].startswith(prefix))
            children = [c for c in self.atomic['requirements'] if c['parent_id'] == parent['id']]
            self.assertEqual(count, len(children))
            self.assertTrue(all(c['atomicity'] == 'reviewed' and c['status'] == 'identified' and not c['evidence_refs'] for c in children))

    def test_communication_receipt_uncertainty_and_scope_qualifiers_survive_atomization(self):
        parents = {p['id'] for p in self.atomic['parents'] if p['original_text'].startswith(
            ('Wiadomości przechodzą przez daemon;', 'Nie udajemy gwarancji exactly-once', 'Zakresy komunikacji i subskrypcji'))}
        for phrase in ['before delivery', 'backpressure', 'user_instruction message type',
                       'provider exactly-once', 'uncertain delivery with the actual session',
                       'external actions with unknown outcomes', 'subscription scopes',
                       'Limit status storms', 'Limit copying complete transcripts']:
            with self.subTest(phrase=phrase):
                ledger, atomic = self.mutate()
                child = next(c for c in atomic['requirements'] if c['parent_id'] in parents and phrase in c['obligation'])
                child['obligation'] = 'Send some messages.'
                self.assertTrue(any('semantic obligation/qualifier missing' in error for error in self.errors(ledger, atomic)))

    def test_small_task_qualifier_cannot_disappear_from_interpretation(self):
        ledger, atomic = self.mutate()
        child = next(c for c in atomic['requirements'] if 'including small tasks' in c['obligation'])
        child['obligation'] = child['obligation'].replace(', including small tasks', '')
        self.assertTrue(any('semantic obligation/qualifier missing' in error for error in self.errors(ledger, atomic)))

    def test_exception_is_preserved_and_oracle_detects_its_loss(self):
        ledger, atomic = self.mutate()
        child = next(c for c in atomic['requirements'] if 'same provider is allowed' in c['obligation'])
        child['obligation'] = 'Always require a different provider.'
        self.assertTrue(any('semantic obligation/qualifier missing' in error for error in self.errors(ledger, atomic)))

    def test_inherited_condition_cannot_be_disabled(self):
        ledger, atomic = self.mutate()
        atomic['requirements'][20]['qualifiers']['inherit_full_parent'] = False
        self.assertTrue(any('dropped inherited' in error for error in self.errors(ledger, atomic)))

    def test_changed_original_qualifier_fails(self):
        ledger, atomic = self.mutate()
        atomic['requirements'][20]['original_text'] = 'Weakened summary'
        self.assertTrue(any('lost original qualifier' in error for error in self.errors(ledger, atomic)))

    def test_duplicate_semantic_id_fails(self):
        ledger, atomic = self.mutate()
        atomic['requirements'][1]['semantic_id'] = atomic['requirements'][0]['semantic_id']
        self.assertTrue(any('duplicate semantic_id' in error for error in self.errors(ledger, atomic)))

    def test_stale_source_fails(self):
        ledger, atomic = self.mutate()
        self.assertTrue(any('stale source' in error for error in self.errors(ledger, atomic, self.source+'\nNew obligation.\n')))

    def test_proposal_is_not_promoted_to_approved_requirement(self):
        parent = next(p for p in self.atomic['parents'] if p['original_text'].startswith('Proponowane profile:'))
        self.assertEqual('proposal', parent['kind'])
        self.assertEqual([], parent['child_ids'])

    def test_code_comment_is_not_a_heading_or_requirement(self):
        parent = next(p for p in self.atomic['parents'] if p['original_text'].startswith('# profile ='))
        self.assertEqual('code', parent['kind'])
        self.assertEqual([], parent['child_ids'])
        self.assertTrue(all(c['text'] != parent['original_text'] for c in parent['context']))

    def test_role_components_are_individual_acceptance_units(self):
        parent = next(p for p in self.atomic['parents'] if p['original_text'].startswith('| Requirements verifier | Całe'))
        self.assertEqual(6, len(parent['child_ids']))
        ledger, atomic = self.mutate()
        child_id = parent['child_ids'][-1]
        atomic['requirements'] = [c for c in atomic['requirements'] if c['id'] != child_id]
        next(p for p in atomic['parents'] if p['id'] == parent['id'])['child_ids'].remove(child_id)
        self.assertTrue(any('aggregate role checkbox' in error for error in self.errors(ledger, atomic)))

    def test_risk_check_cannot_omit_detection_or_mitigation(self):
        ledger, atomic = self.mutate()
        child = next(c for c in atomic['requirements'] if c['obligation'].startswith('Check R001 '))
        child['mandatory_criteria'] = ['Evidence exists.']
        self.assertTrue(any('risk detection or mitigation obligation missing' in error for error in self.errors(ledger, atomic)))

    def test_table_contract_is_inherited(self):
        ledger, atomic = self.mutate()
        child = next(c for c in atomic['requirements'] if c['obligation'].startswith('Check R001 '))
        child['qualifiers']['inherited_table_header'] = ''
        self.assertTrue(any('dropped inherited qualifier' in error for error in self.errors(ledger, atomic)))

    def test_unapproved_design_is_not_established_user_authority(self):
        ledger, atomic = self.mutate()
        parent = next(p for p in atomic['parents'] if p['section'] == '9.2' and p['child_ids'])
        parent['authority'] = 'established_user_requirement'
        self.assertTrue(any('promoted to established user requirement' in error for error in self.errors(ledger, atomic)))

    def test_gap_inventory_accounts_for_every_candidate(self):
        gaps = module('requirements_gaps').inventory(self.atomic)
        self.assertEqual({c['id'] for c in self.atomic['requirements']},
                         {record['requirement_id'] for record in gaps['requirements']})
        self.assertTrue(all(record['acceptance_criteria'] and record['assessment_reason']
                            and record['verification_status'] == 'not_independently_verified'
                            for record in gaps['requirements']))

    def test_regeneration_is_deterministic(self):
        self.assertEqual(self.atomic, extractor.atomic_extract(self.source, self.ledger))

    def test_line_movement_retains_semantic_identity(self):
        moved = '\n'+self.source
        ledger, _ = extractor.extract(moved)
        atomic = extractor.atomic_extract(moved, ledger)
        self.assertEqual([c['semantic_id'] for c in self.atomic['requirements']],
                         [c['semantic_id'] for c in atomic['requirements']])


if __name__ == '__main__':
    unittest.main()
