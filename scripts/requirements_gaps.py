#!/usr/bin/env python3
"""Source-linked work inventory. Assessment is provisional, never compliance proof."""
import argparse
import collections
import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[1]

PACKAGES = {
    'quality_contracts': dict(priority='P1', milestone='M2Q', status='partial',
        reason='Typed requirements, context/model output and review validators exist; complete role-specific artifacts, semantic source compliance and all design/delivery gates are not yet represented.',
        implementation_refs=['src/domain.rs', 'src/verification.rs', 'tests/core.rs'],
        acceptance='A seeded omission or stale/contradictory criterion prevents delivery; every applicable design and delivery gate has current independently reviewed evidence.'),
    'requirements_review': dict(priority='P1', milestone='M2Q', status='partial',
        reason='The full original source is pinned; established decisions, critical M2Q contracts, role-checklist components and risk checks are atomized. Other sentence candidates still need semantic review; runtime does not ingest the design ledger as task requirements.',
        implementation_refs=['scripts/requirements.py', 'scripts/audit_requirements.py', 'tests/test_requirements.py'],
        acceptance='Review all remaining parents, preserve their qualifiers, and independently compare actual final behavior against the original source and all acceptance criteria.'),
    'runtime_recovery': dict(priority='P0', milestone='M1/M4', status='partial',
        reason='Durable transactions, approvals, idempotency, pause/cancel and restart blocking exist. Process/partial-edit reconciliation is being strengthened; provider-specific native resume is not general automatic recovery.',
        implementation_refs=['src/runtime.rs', 'src/storage.rs', 'src/process.rs', 'src/recovery.rs', 'tests/daemon.rs'],
        acceptance='Crash at each persisted effect boundary, reconcile child processes and workspace effects, preserve approvals, and resume the exact provider session or report the documented replacement path.'),
    'ipc_transport': dict(priority='P1', milestone='M1/M4', status='partial',
        reason='Versioned local socket, request IDs, durable mutation deduplication and event polling exist. General push subscription, controlling-client arbitration and delivery receipts remain incomplete.',
        implementation_refs=['src/ipc.rs', 'src/transport.rs', 'src/storage.rs', 'tests/daemon.rs'],
        acceptance='Bounded snapshot/event/artifact reads work at required scale; reconnect replays exact sequence and concurrent control cannot silently overwrite decisions.'),
    'provider_adapters': dict(priority='P1', milestone='M0/M4', status='partial',
        reason='Codex exec has the first real adapter; Claude and Gemini are researched/probed. General send/subscribe/interrupt receipts, provider version matrices and all required capabilities remain incomplete.',
        implementation_refs=['src/providers/mod.rs', 'src/providers/codex.rs', 'scripts/probe.py', 'scripts/probe_claude.py'],
        acceptance='Contract tests pass on explicitly supported versions of all three CLIs, including exact-session resume, partial payloads, cancellation, authentication errors and unsupported capabilities.'),
    'scheduler_roles': dict(priority='P1', milestone='M2Q/M3', status='partial',
        reason='The runtime executes six sequential stages with executor/verifier sessions and validates a plan DAG; it does not yet schedule arbitrary DAG tasks, role expansion or the full template catalog.',
        implementation_refs=['src/domain.rs', 'src/runtime.rs', 'src/providers/mod.rs'],
        acceptance='Execute independent DAG nodes concurrently, serialize collisions, preserve dedicated verifier pairs and all required competencies, and record every team change.'),
    'tui': dict(priority='P2', milestone='M3', status='pending',
        reason='Only a diagnostic CLI exists; Cargo.toml has no Ratatui/Crossterm dependency or TUI module.',
        implementation_refs=['src/main.rs', 'Cargo.toml'],
        acceptance='Two-panel TUI provides stable keyboard selection, chat, session/requirements/checks views, honest receipts and accessibility without color/Unicode.'),
    'configuration_autonomy': dict(priority='P1', milestone='M4', status='pending',
        reason='RunSpec currently supports the supervised diagnostic path. Persisted TOML profile assignment, explicit orchestrator selection, effort mapping and autonomous policy are not implemented.',
        implementation_refs=['src/domain.rs', 'src/main.rs', 'Cargo.toml'],
        acceptance='Persist per-run TOML-derived profiles and mode revisions; autonomous local work respects policy and separately approved external/destructive actions.'),
    'workspace_integration': dict(priority='P1', milestone='M4', status='partial',
        reason='Pinned input reads, controlled full-file replacement and a worktree helper exist. General per-task worktree lifecycle, dirty-input ownership, conflict integration and resource leases remain incomplete.',
        implementation_refs=['src/workspace.rs', 'src/runtime.rs', 'tests/core.rs'],
        acceptance='Exercise dirty/untracked files, concurrent user edits, overlapping tasks, partial edits and conflict resolutions; identify and reverify the integrated candidate without losing user changes.'),
    'security_isolation': dict(priority='P1', milestone='M0/M4', status='partial',
        reason='Codex runs in a restricted read-only profile and controlled edits are runtime-mediated; local arbitrary check commands are not a general sandbox, and cross-platform provider enforcement is unverified.',
        implementation_refs=['src/providers/codex.rs', 'src/workspace.rs', 'src/ipc.rs', 'src/process.rs'],
        acceptance='Enforce each configured restriction outside model text on macOS/Linux; reject unavailable enforcement, sanitize terminal output and preserve authenticated IPC and secret provenance.'),
    'dependencies_licenses': dict(priority='P1', milestone='M2Q/M4', status='partial',
        reason='Dependency inventory and source/license checks exist. Product license/distribution is an open design decision; final obligations, complete vulnerability checks and general per-task reuse decisions remain incomplete.',
        implementation_refs=['scripts/dependencies.py', 'docs/dependencies.json', 'Cargo.toml'],
        acceptance='Document real alternatives and full final dependency graph, resolve license/distribution conditions, fulfill all notices/obligations and repeat relevant scans after graph changes.'),
    'research_documents_analysis': dict(priority='P1', milestone='M4', status='pending',
        reason='The diagnostic slice demonstrates a software bugfix. General research/document/analysis templates, source access evidence, claim maps and independent calculations are not implemented.',
        implementation_refs=['src/domain.rs', 'src/providers/mod.rs', 'scripts/demo.py'],
        acceptance='Complete research with conflicting sources, an integrated document and an analysis with seeded unit/data errors, using independent source reads/calculations and all common quality gates.'),
    'observability_evals': dict(priority='P2', milestone='M5', status='partial',
        reason='Local events, evidence exports and fake/live demos exist; repeated representative multi-task evaluations, baselines, resource provenance and false-positive/defect measurements remain incomplete.',
        implementation_refs=['src/storage.rs', 'scripts/demo.py', 'docs/m0/live-demo.json', 'tests/core.rs'],
        acceptance='Compare one CLI, executor/reviewer and the full workflow on identical representative tasks; publish versions, repeat counts, spread, false alarms, missed defects and user effort.'),
}


def route(parent, requirement):
    section, text = parent['section'], parent['original_text'].casefold()
    obligation = requirement['obligation'].casefold()
    if section == '2':
        if any(value in text for value in ('| główny interfejs |', '| graf |', '| sesje agentów |')) or 'ratatui' in obligation:
            return 'tui'
        if any(value in text for value in ('| konfiguracja |', '| routing cli |', '| autonomia |', '| pełny automat |', '| wybór orkiestratora |', '| personalizacja mvp |')):
            return 'configuration_autonomy'
        if '| dostawcy |' in text or '| integracje ai |' in text:
            return 'provider_adapters'
        if any(value in text for value in ('| restart komputera |', '| działanie w tle |', '| limity mvp |')):
            return 'runtime_recovery'
        if '| katalog pracy |' in text:
            return 'workspace_integration'
        if '| pełne wymagania |' in text:
            return 'requirements_review'
        if '| biblioteki |' in text:
            return 'dependencies_licenses'
        if '| dobór zespołu |' in text or '| pary ról |' in text:
            return 'scheduler_roles'
        if '| zakres zadań |' in text:
            return 'research_documents_analysis'
        return 'quality_contracts'
    if section.startswith('8'):
        return 'tui'
    if section.startswith('6.6'):
        return 'requirements_review'
    if section.startswith(('7.1', '7.2', '7.3')):
        return 'scheduler_roles'
    if section.startswith(('7.4', '9.6')):
        return 'configuration_autonomy'
    if section.startswith(('9.4', '9.5')):
        return 'provider_adapters'
    if section.startswith(('9.3', '10.4')):
        return 'ipc_transport'
    if section.startswith('11.1'):
        return 'workspace_integration'
    if section.startswith(('11.2', '12.3', '12.4')):
        return 'research_documents_analysis'
    if section.startswith('12.2.11'):
        return 'dependencies_licenses'
    if section.startswith('13'):
        return 'security_isolation'
    if section.startswith('14'):
        return 'runtime_recovery'
    if section.startswith('15'):
        return 'observability_evals'
    if section.startswith('19'):
        risk_id = text.split('/')[0].strip('| ').upper()
        number = int(risk_id[1:]) if risk_id.startswith('R') and risk_id[1:].isdigit() else 0
        if 191 <= number <= 195:
            return 'requirements_review'
        if 91 <= number <= 100:
            return 'runtime_recovery'
        if 51 <= number <= 70:
            return 'security_isolation'
        if 81 <= number <= 90:
            return 'research_documents_analysis'
        if 221 <= number <= 230:
            return 'dependencies_licenses'
        if 11 <= number <= 20:
            return 'scheduler_roles'
    return 'quality_contracts'


def inventory(atomic):
    parents = {parent['id']: parent for parent in atomic['parents']}
    records = []
    for requirement in atomic['requirements']:
        parent = parents[requirement['parent_id']]
        package = route(parent, requirement)
        assessment = PACKAGES[package]
        status, reason = assessment['status'], assessment['reason']
        if parent['section'] == '2' and requirement['obligation'] in (
                'Implement the product in Rust.', 'Do not implement a model-provider API client.',
                'Integrate through existing provider CLIs only.',
                'Deliver design.md as the current design-stage artifact; implementation follows as the next step.'):
            status = 'implemented'
            reason = 'The checked-in project is Rust, provider interaction is CLI-based, and the complete design.md artifact is present. This assesses that narrow obligation only, without establishing full product compliance.'
        records.append(dict(requirement_id=requirement['id'], parent_id=parent['id'],
                            source='design.md', source_line=parent['line'], section=parent['section'],
                            obligation=requirement['obligation'], authority=requirement['authority'],
                            atomicity=requirement['atomicity'], status=status, assessment_reason=reason,
                            work_package=package, priority=assessment['priority'], milestone=assessment['milestone'],
                            acceptance_criteria=requirement['mandatory_criteria'],
                            package_completion=assessment['acceptance'],
                            implementation_refs=assessment['implementation_refs'],
                            verification_status='not_independently_verified'))
    return dict(schema_version=1, source_sha256=atomic['source_sha256'],
                assessment='Provisional source-linked implementation gap inventory. Implemented is a narrow code/document observation, not verified acceptance. Sentence candidates remain provisional; package status must not grant requirement pass.',
                status_counts=dict(collections.Counter(record['status'] for record in records)),
                packages=PACKAGES, requirements=records)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    data = inventory(json.loads((ROOT/'docs/requirements-atomic.json').read_text()))
    path = ROOT/'docs/requirements-gaps.json'
    output = json.dumps(data, ensure_ascii=False, indent=2)+'\n'
    if args.check:
        if not path.exists() or path.read_text() != output:
            raise SystemExit('docs/requirements-gaps.json is stale')
    else:
        path.write_text(output)
    print(f'{len(data["requirements"])} source-linked work items: {data["status_counts"]}')


if __name__ == '__main__':
    main()
