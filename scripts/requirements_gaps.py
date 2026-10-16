#!/usr/bin/env python3
"""Source-linked work inventory. Assessment is provisional, never compliance proof."""
import argparse
import collections
import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[1]

PACKAGES = {
    'quality_contracts': dict(priority='P1', milestone='M2Q', status='partial',
        reason='Typed requirements, context/model output and review validators exist. Turn-bound immutable context paging has genuine Antigravity and Codex runtime evidence. New interview rounds retain complete pre-contract observations; durable file/directory kinds bind original captured bodies across review, tasks and recovery. Selected directory scopes expose original member bodies without expanding selectors. Pinned task replacements use lossless flat turn archives with exact original context/receipt/status reconstruction, while old inline handoff hashes stay unchanged. Versioned task instructions include the exact response wire schema; invalid native candidates remain rejected. Model citations resolve exact captured directory members and authenticated task-check output without expanding canonical source/approval hashes; provenance rechecks those bindings. A borrowed-run resolver reuses canonical/member hashes within each immutable validation and keys check memoization by reference plus expected hash; conflicting hashes remain rejected. Reader/transport clones share immutable source ownership; copy-on-write, cache eviction and deserialized replacements cannot reuse an old attestation or change original wire hashes. Full repository/task acceptance, complete source-kind storage, compaction, semantic source compliance and complete role/family delivery remain incomplete.',
        implementation_refs=['src/domain.rs', 'src/verification.rs', 'src/context.rs', 'src/context/transport.rs', 'src/context/bridge.rs', 'src/context/history.rs', 'src/context/citations.rs', 'src/runtime/team/replacement/context.rs', 'tests/context_citations.rs', 'tests/task_history.rs', 'src/team/models.rs', 'src/workspace/observation.rs', 'tests/input_observations.rs', 'tests/context_pages.rs', 'tests/context_bridge.rs', 'tests/core.rs'],
        acceptance='A seeded omission or stale/contradictory criterion prevents delivery; every applicable design and delivery gate has current independently reviewed evidence.'),
    'requirements_review': dict(priority='P1', milestone='M2Q', status='partial',
        reason='The full original source is pinned; established decisions, critical M2Q contracts, role-checklist components, provider-capability dimensions, security contracts, risk checks and all section 14 recovery/no-budget contracts are atomized. Other sentence candidates still need semantic review; runtime does not ingest the design ledger as task requirements.',
        implementation_refs=['scripts/requirements.py', 'scripts/audit_requirements.py', 'tests/test_requirements.py'],
        acceptance='Review all remaining parents, preserve their qualifiers, and independently compare actual final behavior against the original source and all acceptance criteria.'),
    'runtime_recovery': dict(priority='P0', milestone='M1/M4', status='partial',
        reason='Durable transactions, scoped approvals, idempotency, pause/cancel admission, workspace leases and partial-edit reconciliation exist. Concurrent native callbacks preserve the canonical session ledger. Scoped exact-ID continuation and explicit replacement paths have protocol-fixture coverage. Pre-delivery task-input checkpoints and scoped read-only admission recover exact completed native output before any effect checkpoint; the owned genuine result was admitted without repeating inference/checks and still awaits independent verification. Durable check-container custody checkpoints block uncertain replay and source-only reconciliation; exact-name observation can recover a verified owned ID without creating, starting or assuming absence. Production controller and genuine cross-provider/restart recovery acceptance remain incomplete.',
        implementation_refs=['src/runtime.rs', 'src/storage.rs', 'src/process.rs', 'src/recovery.rs', 'src/runtime/team/restore.rs', 'src/runtime/team/restore/readonly.rs', 'src/sessions.rs', 'tests/team_execution.rs', 'tests/task_worktrees.rs', 'tests/daemon.rs'],
        acceptance='Crash at each persisted effect boundary, reconcile child processes and workspace effects, preserve approvals, and resume the exact provider session or report the documented replacement path.'),
    'ipc_transport': dict(priority='P1', milestone='M1/M4', status='partial',
        reason='Versioned bounded local IPC, mutation deduplication, hash-consistent document reads and event polling exist. Team acceptance authenticates asynchronously outside the store, binds the complete canonical run and live sources, and refuses cold IPC mutation without a sealed prepared proof. Authenticated next-turn direct/broadcast delivery receipts are recorded. Immutable task-recovery documents prepare asynchronously, bind canonical state and current source observations, and reuse hash-consistent pages. Push subscriptions, controlling-client arbitration and active-turn hook transport remain open.',
        implementation_refs=['src/ipc.rs', 'src/ipc/projection.rs', 'src/ipc/task_projection.rs', 'src/transport.rs', 'src/storage.rs', 'tests/daemon.rs'],
        acceptance='Bounded snapshot/event/artifact reads work at required scale; reconnect replays exact sequence and concurrent control cannot silently overwrite decisions.'),
    'provider_adapters': dict(priority='P1', milestone='M0/M4', status='partial',
        reason='Codex, Claude, Gemini and agy have process-backed structured adapters, capability checks, cancellation-aware probes and native receipts. Explicit agy 1.3.5/1.4.0 and Codex 0.162.1 pinned_context commit daemon read receipts before delivery and authenticate full observed input coverage; actual runtime/native probes and reattachment passed. New agy v2 inputs specify the exact immutable-reader call while retaining v1 authentication. Exact-binary 1.4.0 native probes passed boundary inline input and complete durable-reader coverage; future/prerelease versions remain refused. Antigravity 1.4.0 can retain the same supervised process after a narrowly recognized indexed model-format rejection or a reasonless allowlisted error-step envelope; its typed observation must commit before reading onward, and actual finish/schema/coverage/clean-exit gates remain mandatory. Executable fixtures cover correction, failed commits and refusal cases; genuine CLI correction acceptance is still under evaluation. Exact native-session resume for Antigravity and Codex pinned_context remains unsupported and is excluded from task recovery proposals/admission before any approval or native worker; explicit replacement preserves the captured handoff. Complete native task acceptance, all required versions/platforms, hook/live transport and native recovery coverage remain incomplete.',
        implementation_refs=['src/providers/mod.rs', 'src/providers/codex.rs', 'src/providers/claude.rs', 'src/providers/gemini.rs', 'src/providers/agy.rs', 'src/providers/agy/context.rs', 'src/providers/codex/context.rs', 'tests/codex_context.rs', 'src/context/bridge.rs', 'src/runtime/turns.rs', 'tests/agy.rs', 'tests/verifier_agy.rs', 'tests/agy_version_live.rs', 'tests/providers.rs'],
        acceptance='Contract tests pass on explicitly supported versions of all three CLIs, including exact-session resume, partial payloads, cancellation, authentication errors and unsupported capabilities.'),
    'scheduler_roles': dict(priority='P0', milestone='M2Q/M3', status='partial',
        reason='The daemon reviews and installs source-bound proposals, runs accepted DAG tasks with dedicated reviewers, checkpoints effects/checks/models and supports scoped refresh, recovery and stage acceptance. Executable fixtures cover these paths. All twelve owned native plan reports authenticated, the reviewed team installed through prepared IPC and its first three-action wave started. Adaptive team adequacy and genuine complete chat-origin task/quality/delivery acceptance remain unproven; full design competencies and families still need audit.',
        implementation_refs=['src/team.rs', 'src/runtime/orchestration.rs', 'src/runtime/team_acceptance.rs', 'src/ipc/management_projection.rs', 'src/orchestration/planning.rs', 'tests/team.rs', 'tests/orchestration_runtime.rs', 'src/runtime/team.rs', 'src/runtime/team/tasks.rs', 'tests/team_execution.rs'],
        acceptance='Execute independent DAG nodes concurrently, serialize collisions, preserve dedicated verifier pairs and all required competencies, and record every team change.'),
    'tui': dict(priority='P2', milestone='M3', status='partial',
        reason='A Ratatui/Crossterm TUI provides stable keyboard navigation, team/relationship views, session/artifact/check/message inspection, explicit approvals, orchestration conversation and a broadcast composer. A spatial graph, progress overlay, version-bound element annotations, reviewed orchestration reactions and explicit mode-change controls are implemented. Guided interrupted-task reconciliation, captured result/check restoration, scoped native continuation/replacement and read-only source refresh now reuse explicit frozen confirmation cards. The session inspector selects the latest reserved session for its displayed task even before an agent binding checkpoint. Active native rejected steps show committed counts and pending result status without inferring a cause or completed review. Complete native DAG/source recovery, intent translation and future-only autonomous acceptance remain incomplete.',
        implementation_refs=['src/tui/app.rs', 'src/tui/model.rs', 'src/tui/backend.rs', 'tests/tui.rs', 'tests/tui_sessions.rs', 'src/comments.rs', 'src/tui/graph.rs', 'tests/comments.rs'],
        acceptance='Two-panel TUI provides stable keyboard selection, chat, session/requirements/checks views, honest receipts and accessibility without color/Unicode.'),
    'configuration_autonomy': dict(priority='P1', milestone='M4', status='partial',
        reason='Pinned TOML profiles, explicit orchestrator selection, per-role provider/model/effort routing and policy types are implemented. Explicit run mode changes retain validated future-action policy history. Full autonomous progression/repair/refresh, restricted local runners and enforcement across all task/workspace capabilities remain incomplete.',
        implementation_refs=['src/config.rs', 'src/providers/mod.rs', 'src/runtime.rs', 'tests/config.rs'],
        acceptance='Persist per-run TOML-derived profiles and mode revisions; autonomous local work respects policy and separately approved external/destructive actions.'),
    'workspace_integration': dict(priority='P1', milestone='M4', status='partial',
        reason='Managed workspace inspection, explicit dirty-input inclusion, private preparation/activation, runtime-mediated edits/create/delete, ownership receipts and separately approved source delivery have implementation and executable fixture coverage. General integrated candidates, concurrent user-edit ownership, binary/link/mode lifecycle and complete native task acceptance remain incomplete.',
        implementation_refs=['src/workspace.rs', 'src/workspace/managed.rs', 'src/workspace/delivery.rs', 'src/runtime/workspace.rs', 'src/runtime/delivery.rs', 'tests/managed_workspace.rs', 'tests/source_delivery.rs', 'tests/file_operations.rs', 'tests/core.rs'],
        acceptance='Exercise dirty/untracked files, concurrent user edits, overlapping tasks, partial edits and conflict resolutions; identify and reverify the integrated candidate without losing user changes.'),
    'security_isolation': dict(priority='P1', milestone='M0/M4', status='partial',
        reason='Audited CLI profiles, scoped immutable readers and runtime-mediated edits exist; these do not establish an OS sandbox. A Rust guardian now retains workspace leases until exact labelled-container removal, including cancellation and parent death; owned native/mocked lifecycle probes passed. A canonical SQLite custody journal now fences create/start, rejects uncertain replay, and requires actual removal proof; owned exact-name observation does not infer absence. Production restricted check admission/controller, complete pre-start recovery/evidence, secret/retention policies and cross-platform enforcement remain incomplete.',
        implementation_refs=['src/providers/codex.rs', 'src/workspace.rs', 'src/ipc.rs', 'src/process.rs', 'src/process/container.rs', 'src/runtime/check_resources.rs', 'tests/check_resources.rs', 'tests/container_supervision.rs', 'scripts/probe_restricted_runner.py', 'scripts/probe_container_runner.py', 'docs/restricted-runner.md'],
        acceptance='Enforce each configured restriction outside model text on macOS/Linux; reject unavailable enforcement, sanitize terminal output and preserve authenticated IPC and secret provenance.'),
    'dependencies_licenses': dict(priority='P1', milestone='M2Q/M4', status='partial',
        reason='Dependency inventory and source/license checks exist; the user selected private code with no public distribution. Complete source/license obligations, vulnerability checks and per-task alternative/reuse decisions remain incomplete.',
        implementation_refs=['scripts/dependencies.py', 'docs/dependencies.json', 'Cargo.toml'],
        acceptance='Document real alternatives and full final dependency graph, resolve license/distribution conditions, fulfill all notices/obligations and repeat relevant scans after graph changes.'),
    'research_documents_analysis': dict(priority='P1', milestone='M4', status='pending',
        reason='Research/document/analysis templates and family-specific typed quality contracts exist. Complete task execution, authenticated source access, claim maps, independent calculations and representative final acceptance remain unproven.',
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
        if any(value in text for value in ('| główny interfejs |', '| graf |', '| sesje agentów |', '| komunikacja agentów |', '| widok diff i recenzja |')) or 'ratatui' in obligation:
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
        if any(value in text for value in ('| dobór zespołu |', '| pary ról |', '| kontekst orkiestratora |')):
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
