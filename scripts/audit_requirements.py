#!/usr/bin/env python3
"""Source-first structural audit plus a bounded semantic regression oracle.

This module intentionally does not import requirements.py or derive expected
obligations from the generated ledger. It rereads the original source in order,
checks preservation, and uses separately authored semantic expectations. It is
an automated audit, not a separate human reviewer or a proof of completeness.
"""
import argparse
import collections
import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[1]

# Separate source-first interpretation of safety-critical clauses. Expected
# substrings apply to child interpretations, so retaining a source quote alone
# cannot hide omission of an obligation. Conditions are additionally bound to
# the full original parent and context.
SEMANTIC_EXPECTATIONS = [
    ('| `supervised` |', ['before starting each stage', 'every correction round', 'each final stage result', 'may also present and approve the explicit scope of the next stage', 'optional combined approval, not implicit authorization', 'do not automatically open a correction round after a verifier rejects']),
    ('| `autonomous` |', ['opens successive stages after their applicable gates are satisfied', 'opens correction rounds after their applicable gates are satisfied', 'local work within the policy', 'local tests within the policy', 'local commits within the policy', 'separate approval for push', 'separate approval for publication', 'separate approval for external messages', 'separate approval for destructive operations', 'necessary product decision may require the user']),
    ('Zmiana trybu jest jawna, zapisana', ['mode change explicit', 'Persist every autonomy-mode change', 'future actions', 'Do not retrospectively approve prior changes', 'Do not remove verifier concerns', 'scope invalidates the affected approvals', 'scope requires reassessment']),
    ('Oddzielamy zgodę na **etap**', ['approval of a stage from approval of an action', 'does not authorize push', 'does not authorize publication', 'does not authorize external messages', 'does not authorize destructive operations', 'only the specified task changes', 'must not include unrelated user changes']),
    ('| Orchestrator | Orchestration verifier |', ['selected Orchestrator competency', 'substantive team changes', 'substantive scope changes', 'substantive strategy changes', 'decisions align with the objective', 'TeamPlan change proposal or a management decision', 'illustrative artifact forms']),
    ('| Context analyst | Context verifier |', ['actual behavior', 'sources', 'dependency coverage', 'applicable standards', 'closure of unknowns', 'ContextModel', 'verified knowledge register', 'illustrative artifact forms']),
    ('| Requirements analyst | Requirements verifier |', ['alignment with user intent', 'contradictions', 'acceptance-criteria completeness', 'specification and criteria map', 'illustrative artifact forms']),
    ('| Planner | Plan verifier |', ['abstract model', 'invariants', 'independent simulation', 'counterexamples', 'scope', 'evidence completeness', 'SolutionModel, SimulationReport and a task DAG', 'illustrative artifact forms']),
    ('| Options analyst | Options verifier |', ['available alternatives', 'comparison fairness', 'evidence', 'uncertainty', 'selecting the best solution', 'OptionsAnalysis and SolutionDecision', 'illustrative artifact forms']),
    ('| Architect | Architecture verifier |', ['fit with the project', 'simplicity', 'reuse of existing solutions', 'ADRs and module contracts', 'illustrative artifact forms']),
    ('| Dependency analyst | Dependency verifier |', ['alternatives to custom code', 'dependency provenance', 'licenses', 'dependency security', 'dependency maintenance cost', 'DependencyDecision and LicenseReport', 'illustrative artifact forms']),
    ('| Implementer | Code verifier |', ['model conformance', 'standards conformance', 'requirements conformance', 'simplicity', 'responsibilities', 'absence of unnecessary additions', 'patch, TraceabilityMap and a standards report', 'illustrative artifact forms']),
    ('| Refactorer | Refactor verifier |', ['closure of solution debt', 'preservation of contracts', 'structural improvement', 'absence of speculative abstractions', 'RefactorReport, a patch and behavior-preservation evidence', 'illustrative artifact forms']),
    ('| Static analysis operator | Static analysis verifier |', ['analyzer coverage', 'configurations', 'absence of unresolved concerns', 'absence of rule-weakening attempts', 'StaticAnalysisReport and raw results', 'illustrative artifact forms']),
    ('| Compliance auditor | Compliance verifier |', ['every requirement', 'every step of the full plan', 'realization quality', 'absence of extraction gaps', 'evidence is current', 'ComplianceAudit and RequirementLedger accounting', 'illustrative artifact forms']),
    ('| Test designer | Test design verifier |', ['scenarios derive from requirements', 'scenarios detect probable defects', 'test plan and requirements map', 'illustrative artifact forms']),
    ('| Test implementer | Test verifier |', ['assertions', 'actual execution paths', 'isolation', 'mocks', 'test resilience', 'tests and execution results', 'illustrative artifact forms']),
    ('| Security analyst | Security verifier |', ['threat model', 'coverage', 'reproductions', 'false alarms', 'fixes', 'threat model and findings', 'illustrative artifact forms']),
    ('| Performance engineer | Performance verifier |', ['benchmark representativeness', 'baseline', 'absence of semantic regression', 'measurements and recommendations or a patch', 'illustrative artifact forms']),
    ('| Integrator | Integration verifier |', ['entire change set', 'inter-module contracts', 'regressions', 'delivery candidate', 'illustrative artifact form']),
    ('| Researcher | Research verifier |', ['source reliability', 'source currency', 'source coverage', 'facts from inferences', 'linked report', 'illustrative artifact form']),
    ('| Data analyst | Analysis verifier |', ['data', 'assumptions', 'calculations', 'units', 'reproducibility', 'analysis and script or notebook', 'illustrative artifact forms']),
    ('| Writer | Editorial/factual verifier |', ['factual correctness', 'citations', 'coherence', 'alignment with the brief', 'document', 'illustrative artifact form']),
    ('| Delivery author | Delivery verifier |', ['report with the actual result', 'instructions with the actual result', 'final report', 'illustrative artifact form']),
    ('- Dla bugfixu: reprodukcja problemu', ['reproduce the problem before changing', 'check the result after the change', 'explicit gap requiring other evidence', 'Do not treat inability to reproduce as implicit success']),
    ('Orkiestrator proponuje `TeamPlan` na podstawie:', ['kind of result', 'task complexity', 'actual risks', 'dependencies', 'context size', 'project modules', 'available CLIs', 'required tools', 'quality profile', 'before starting', 'justified responsibility', 'agents without a task']),
    ('Bootstrap korzysta z wbudowanej bazy ról', ['built-in role base', 'explicitly selected orchestrator profile', 'Permit discovery before', 'Permit requirements analysis before', 'appropriate stage in supervised mode', 'context and RiskCoverage', 'gates apply to work on the solution', 'analyses needed to develop the plan', 'own criteria', 'Do not require a completed implementation plan']),
    ('Obowiązkowe warunki:', ['every listed team-selection condition as mandatory']),
    ('- Każde zadanie ma właściciela, rezultat', ['Every task has an owner', 'Every task has a result', 'verifiable completion criteria']),
    ('- Zadanie implementacyjne ma aktualne', ['current context_ready', 'current plan_ready', 'relevant model fragment', 'Do not waive', 'small change']),
    ('- Wybór wariantu ma `solution_selected`', ['alternative choice', 'solution_selected gate', 'architecture assessment before implementation', 'input assessment before implementation', 'dependency assessment before implementation', 'extent verifiable from the plan', 'those same gates', 'code and final-integration evidence before delivery']),
    ('- Każda uruchomiona rola wykonawcza ma przypisanego', ['independent verifier', 'every started executor role']),
    ('- Nie ma cykli w grafie zależności zadań', ['task dependency graph is acyclic', 'management delegation graph is acyclic']),
    ('- Każdy aktywny zapis ma określony zakres', ['Every active write', 'explicit scope', 'defined integration method']),
    ('- Są zdefiniowane warunki eskalacji', ['escalation conditions', 'stopping conditions', 'completion conditions']),
    ('- Każda nowa runda wskazuje nierozwiązany problem', ['unresolved problem', 'action plan', 'no numerical round limit', 'no numerical agent limit']),
    ('Graf komunikacji może zawierać cykle.', ['Permit cycles in the communication graph', 'must not create an infinite execution loop', 'specific task', 'artifact, a decision or escalation']),
    ('To katalog kompetencji. Każdy run obejmuje', ['competency catalog', 'Every run includes a context pair', 'requirements pair', 'options pair', 'planning pair', 'compliance pair', 'delivery pair', 'substantive orchestrator decisions', 'architecture pair', 'dependency pair', 'implementation pair', 'refactor pair', 'static-analysis pair', 'test-design pair', 'test-implementation pair', 'integration pair', 'brief but remain mandatory', 'actual threats', 'separate security or performance pair', 'Research without code', 'does not require fictitious compilation tests']),
    ('Każda uruchomiona rola ma własnego wykonawcę', ['own executor and dedicated verifier', 'shorter artifacts', 'fewer specialist roles', 'Do not share one verifier agent between several executor roles, including small tasks', 'execute sequentially', 'does not require a permanently running CLI process', 'Do not perform refactoring when no file transformations are needed', 'mandatory refactoring assessment', 'no transformations are needed']),
    ('Wszystkie szablony dziedziczą wspólną bazę', ['common role base', 'applicable role pairs from section 7.2', 'specialization', 'required competencies', 'required gates', 'extend the template', 'shorten queues', 'change work partitioning', 'cannot remove required verification', 'Every team composition change has a rationale', 'Every team composition change has a history event']),
    ('| Komunikacja agentów |', ['every agent', 'point-to-point', 'any current team agent', 'broadcast', 'other current team agents', 'authenticated message authorship', 'delivery state']),
    ('| Kontekst orkiestratora |', ['task reports', 'verifier decisions', 'addressed messages', 'Do not require the orchestrator to read child agent session transcripts']),
    ('| Integracje AI |', ['existing provider CLIs only', 'Do not implement a model-provider API client']),
    ('| Jakość |', ['quality over cost', 'each started executor role, including small tasks']),
    ('| Niezależność |', ['separate session and context', 'same provider is allowed in a separate session']),
    ('| Pełny automat |', ['local work', 'approval for push', 'approval for publication', 'approval for external messages', 'approval for destructive']),
    ('| Limity MVP |', ['agent-count limit', 'concurrency limit', 'time limit', 'cost limit', 'delegation-depth limit', 'correction-round limit']),
    ('Requirements analyst wyodrębnia', ['atomic requirements', 'hierarchy', 'condition and exception', 'limitation and quality', 'negative requirements', 'acceptance details']),
    ('Każdy rekord zawiera: `requirement_id`', ['source span and original', 'interpretation and parent', 'acceptance criteria', 'verifier ownership', 'dependencies, revision and evidence', 'original parent', 'all mandatory children and integration']),
    ('Requirements verifier niezależnie', ['entire original source independently', 'every obligation-bearing', 'interpretation accuracy', 'ambiguity explicitly', 'do not silently']),
    ('Końcowy `ComplianceAudit`', ['complete original source and ledger', 'omitted during extraction', 'dedicated verifier', 'all criteria of every mandatory', 'stale evidence', 'contradictory results', 'unapproved simplification']),
    ('Usunięcie, osłabienie', ['explicit user decision and new revision', 'removal, weakening, deferral or reinterpretation', 'Do not grant autonomous', 'completion percentage']),
    ('Definicja pozycji zawiera', ['stable identity', 'associated risks', 'rejection rule', 'not-applicable condition', 'actual method', 'missing data as pass', 'scope-specific', 'confirmed by']),
    ('`RiskCoverage` obejmuje', ['every risk ID', 'independently verify exclusions', 'dedicated verifier', 'missing role pair', 'Do not drop a risk', 'full responsibility map']),
    ('Runtime odrzuca raport', ['empty checklist', 'missing risks', 'stale evidence', 'disallowed author', 'do not establish honest reasoning', 'newly discovered risks']),
    ('Capabilities obejmują osobno:', ['structured-output capability separately', 'streaming capability separately', 'history capability separately', 'resume capability separately', 'mid-turn-steering capability separately', 'cancellation capability separately', 'approvals capability separately', 'tool-restriction capability separately', 'sandbox capability separately', 'usage capability separately', 'native-subagent capability separately', 'unavailable provider functions explicit', 'Do not interpret interrupt as rollback', 'interruption request from confirmed stopping in the receipt']),
    ('Chronimy: pliki użytkownika', ['Protect user files', 'Protect secrets', 'Protect CLI authentication', 'Protect session history', 'Protect artifacts', 'Protect local IPC', 'Protect repositories', 'Protect external resources', 'repository contents as potentially untrusted', 'dependencies as potentially untrusted', 'test scripts as potentially untrusted', 'webpages as potentially untrusted', 'tool results as potentially untrusted', 'plugins as potentially untrusted', 'MCP integrations as potentially untrusted', 'other agents responses as potentially untrusted', 'generated patches as potentially untrusted']),
    ('Polityka run definiuje dostęp', ['file access in the run policy', 'network access in the run policy', 'tool access in the run policy', 'secret access in the run policy', 'publication access in the run policy', 'deletion access in the run policy', 'external-effect access in the run policy', 'fully autonomous execution within the run policy', 'Do not substitute model judgment for process safeguards']),
    ('**Istotne ograniczenie integracji CLI:**', ['not automatically controlling every internal native tool invocation', 'documented provider approvals/restrictions mechanism or an external sandbox/environment', 'Block session start', 'required profile restriction cannot be enforced', 'unless the user explicitly selects a weaker profile', 'Do not silently select a weaker profile', 'Do not substitute a prompt for isolation', 'Do not substitute a worktree for isolation']),
    ('Weryfikator ma dostęp do przypiętego artefaktu', ['verifier access to the pinned artifact', 'Permit verifier checks in a separate environment', 'Deny verifier permission to modify accepted sources', 'Treat tests as executable code', 'same command restrictions to tests as to other commands']),
    ('Logi i eksport mają redakcję sekretów', ['Redact secrets in logs and exports', 'Restrict file permissions for logs and exports', 'retention policy for logs and exports', 'Do not claim redaction guarantees removal of every secret', 'Limit collection at the source', 'Do not copy CLI authentication stores into logs', 'Do not copy CLI authentication stores into artifacts', 'Sanitize dangerous terminal control sequences']),
    ('Każde przekazanie utrwala', ['negative requirements', 'provenance', 'dependent facts', 'dependent models and simulations', 'current snapshot', 'Do not plan using knowledge marked stale']),
    ('Trwałe źródła: wymagania', ['Persist the requirements', 'Persist ContextModel', 'Persist SolutionModel', 'Persist simulation results', 'Persist quality standards', 'Persist decisions', 'Persist the plan', 'Persist contracts', 'Persist accepted artifacts', 'Persist findings', 'Persist check results', 'Persist the source index', 'relevant to its own task', 'references to the complete source materials']),
    ('Pakiet zawiera obowiązkowo:', ['Include the objective', 'Include the scope', 'Include acceptance criteria', 'Include project instructions', 'Include constraints', 'Include the current revision', 'Include inputs', 'prohibition on self-authorized permission changes', 'Include stopping conditions', 'author of each summary', 'source references for each summary', 'Do not substitute a summary for the original source', 'After compaction, supply the key constraints', 'After compaction, verify', 'artifacts are current']),
    ('Instrukcje projektu (np.', ['project instructions with their directory scope', 'AGENTS.md, CLAUDE.md and GEMINI.md when present', 'examples, not an exhaustive list', 'conflicting project instructions', 'explicit decision', 'discovered document or webpage', 'increase runtime permissions']),
    ('Wiadomości przechodzą przez daemon;', ['messages through the daemon', 'before delivery', 'identifier', 'delivery receipts', 'queue inflow into memory through backpressure', 'question message type', 'answer message type', 'handoff message type', 'finding message type', 'decision message type', 'status message type', 'user_instruction message type', 'large materials using references to artifacts', 'queued messages durably when the TUI disconnects']),
    ('Nie udajemy gwarancji exactly-once', ['Do not claim a provider exactly-once', 'Deduplicate internal events', 'Deduplicate internal operations', 'uncertain delivery as delivery_unknown', 'uncertain delivery with the actual session', 'Do not automatically repeat external actions with unknown outcomes']),
    ('Zakresy komunikacji i subskrypcji', ['explicit communication scopes', 'explicit subscription scopes', 'broadcast for changes to shared assumptions', 'substantive conversations to specific roles', 'Limit status storms', 'Limit copying complete transcripts']),
    ('Porównywalność wynika', ['every checklist result', 'Do not lower mandatory', 'average or coverage percentage', 'checklist, requirement or material-input revision']),
]


# Recovery expectations are authored against section 14, independently of the
# extractor. Preserve alternatives and all user-authorized/no-budget exceptions.
SEMANTIC_EXPECTATIONS += [
    ('- SQLite w trybie WAL;', ['SQLite WAL mode', 'event and its state update in one transaction', 'TUI as the source of truth']),
    ('- Checkpointy na granicach etapów,', ['stage boundaries', 'tool-result boundaries', 'substantive plan changes', 'snapshots to accelerate reads', 'events for audit']),
    ('- Po awarii:', ['reconstruct durable state', 'actual fate of processes', 'actual fate of sessions', 'inspect the workspace', 'resume or create a new session with a handoff package']),
    ('- Po restarcie komputera i uruchomieniu Architect:', ['interrupted sessions using durable IDs', 'check current session state', 'natively resume each interrupted session when its provider supports it', 'cannot resume a particular session, report an explicit error', 'cannot resume a particular session, offer a replacement session with a handoff package', 'Do not represent a replacement session as the same native session']),
    ('- Resume zachowuje pauzy', ['Preserve pauses during resume', 'Preserve pending approvals during resume', 'supervised mode, resume must not start an unapproved stage', 'supervised mode, resume must not start an unapproved correction round', 'autonomous mode, resume may continue previously authorized work', 'Do not expand permissions merely because of restart']),
    ('- Resume nie oznacza dokładnego', ['Do not claim resume exactly reproduces model generation', 'Reconcile state before continuing an operation whose effects are unknown']),
    ('- Przy utracie połączenia z dostawcą:', ['retry only safe operations', 'backoff to safe retries', 'authentication failure separately', 'rate-limit failure separately', 'network failure separately', 'Repair the cause of permanent failures', 'Do not blindly retry permanent failures']),
    ('- Brak aktywności:', ['distinguish waiting for the model', 'distinguish waiting for a tool', 'distinguish waiting for the user', 'manual interruption available while waiting', 'Do not impose an MVP turn-completion deadline', 'Do not impose an MVP run-completion deadline']),
    ('- Pause:', ['Pause stops scheduling new actions', 'availability of safely finishing an active turn explicit', 'availability of safely interrupting an active turn explicit', 'Do not promise pause immediately rolls back a tool']),
    ('- Cancel:', ['Cancel stops new actions', 'Cancel interrupts processes', 'Cancel preserves the checkpoint', 'Cancel reports observed effects', 'Cancel must not delete user work']),
    ('**Ustalone:** pierwsza wersja nie ma limitów', ['Do not impose an MVP agent-count limit', 'Do not impose an MVP concurrency limit', 'Do not impose an MVP time limit', 'Do not impose an MVP cost limit', 'Do not impose an MVP delegation-depth limit', 'Do not impose an MVP correction-round limit', 'Do not introduce hidden thresholds that stop MVP work', 'Schedule according to task dependencies', 'Schedule according to write safety', 'Schedule according to actual environment availability']),
    ('Użytkownik może ręcznie pauzować i anulować zadanie.', ['Allow the user to pause manually', 'Allow the user to cancel manually', 'Continue rejecting dependency cycles', 'Continue rejecting actions without authorization', 'Continue rejecting invalid protocols', 'Do not describe an authorization restriction as a budget limit', 'actual operating-system constraints as errors or waiting', 'actual throughput constraints as errors or waiting', 'actual provider constraints as errors or waiting', 'Do not present actual provider constraints as an absence of provider limits']),
    ('Buforowanie strumieni i walidacja rozmiaru komunikatów', ['stream buffering for transport resilience', 'message-size validation for transport resilience', 'Do not interpret transport buffering or message-size validation as a task budget', 'Do not interpret transport buffering or message-size validation as an agent-count budget', 'token and cost data as reported when actually reported', 'token and cost data as estimated when estimated', 'token and cost data as unavailable when absent', 'Do not represent missing token or cost data as zero', 'configurable task budgets outside the MVP', 'later-version feature']),
]


def digest(text):
    return hashlib.sha256(text.encode()).hexdigest()


def audit(source, ledger, atomic):
    errors, checked_semantics = [], []
    lines = source.splitlines()
    expected_lines = {number: line for number, line in enumerate(lines, 1) if line.strip()}
    source_records = ledger.get('records', [])
    parents = atomic.get('parents', [])
    children = atomic.get('requirements', [])
    for name, data in [('source ledger', ledger), ('atomic ledger', atomic)]:
        if data.get('source_sha256') != digest(source):
            errors.append(f'{name}: stale source SHA-256')
    for name, records in [('source ledger', source_records), ('atomic parents', parents)]:
        nums = [item.get('line') for item in records]
        if len(nums) != len(set(nums)):
            errors.append(f'{name}: duplicate source line')
        actual = {item.get('line'): item.get('original_text') for item in records}
        if actual != expected_lines:
            missing = sorted(set(expected_lines) - set(actual))
            changed = sorted(n for n in expected_lines if n in actual and actual[n] != expected_lines[n])
            errors.append(f'{name}: source omission or alteration; missing={missing}, changed={changed}')
        ids = [item.get('id') for item in records]
        if len(ids) != len(set(ids)):
            errors.append(f'{name}: duplicate record ID')
    parent_by_id = {item.get('id'): item for item in parents}
    by_line = {item.get('line'): item for item in parents}
    child_by_parent = collections.defaultdict(list)
    for child in children:
        if child.get('parent_id') not in parent_by_id:
            errors.append(f'orphan requirement: {child.get("id")}')
        if child.get('id') != child.get('semantic_id'):
            errors.append(f'inconsistent semantic identity: {child.get("id")}')
        child_by_parent[child.get('parent_id')].append(child)
    for field in ('id', 'semantic_id'):
        values = [item.get(field) for item in children]
        if any(not value for value in values) or len(values) != len(set(values)):
            errors.append(f'atomic requirements: missing or duplicate {field}')
    # Independent Markdown walk: code, headers, source-only chapters and all
    # normative table/list/prose fragments are inspected in original order.
    code, section, titles, table_header, preceding = False, '0', [], '', ''
    scanned = collections.Counter()
    for number, line in expected_lines.items():
        heading = re.match(r'^(#{1,6})\s+(?:(\d+(?:\.\d+)*)\.)?', line)
        if heading and not code:
            if heading[2]:
                section = heading[2]
            titles = [(level, n, text) for level, n, text in titles if level < len(heading[1])]
            titles.append((len(heading[1]), number, line))
            preceding, table_header = '', ''
        if line.startswith('|'):
            if number == 1 or not lines[number-2].startswith('|'):
                table_header = line
        else:
            table_header = ''
        fence = line.startswith('```')
        non_obligation = code or fence or bool(heading) or line == table_header or bool(re.fullmatch(r'\|[ :|\-]+\|', line))
        source_only = section.split('.')[0] in ('4', '20', '21')
        parent = by_line.get(number)
        if parent:
            expected_context = [n for _, n, _ in titles]
            if [item.get('line') for item in parent.get('context', [])] != expected_context:
                errors.append(f'line {number}: lost heading context')
            if parent.get('table_header') != table_header:
                errors.append(f'line {number}: lost table contract')
            if parent.get('preceding_context') != preceding:
                errors.append(f'line {number}: lost preceding conditions or exceptions')
            if parent.get('source_hash') != digest(line):
                errors.append(f'line {number}: changed source hash')
            linked = child_by_parent[parent['id']]
            if sorted(parent.get('child_ids', [])) != sorted(c['id'] for c in linked):
                errors.append(f'line {number}: inconsistent child index')
            if not non_obligation and not source_only:
                scanned['obligation_or_proposal_fragments'] += 1
                if parent.get('kind') not in ('proposal', 'research', 'open_decision') and not linked:
                    errors.append(f'line {number}: obligation-bearing source has no interpretation')
            established = section == '2' or (section == '6.2' and line.startswith('|')) or line.startswith('**Ustalone:**')
            if established and not non_obligation and parent.get('kind') != 'mandatory':
                errors.append(f'line {number}: established requirement weakened')
            explicitly_proposed = line.startswith(('Proponowan', 'Propozycja', 'Poza pierwszym MVP', 'Przykłady pytań'))
            if explicitly_proposed and parent.get('kind') != 'proposal':
                errors.append(f'line {number}: explicit proposal promoted to obligation')
            if not established and parent.get('authority') == 'established_user_requirement':
                errors.append(f'line {number}: design interpretation promoted to established user requirement')
            if (non_obligation or source_only) and linked:
                errors.append(f'line {number}: code, structure or research promoted to obligation')
            for child in linked:
                if child.get('original_text') != line:
                    errors.append(f'line {number}: child lost original qualifier text')
                if not child.get('mandatory_criteria') or any(not value.strip() for value in child.get('mandatory_criteria', [])):
                    errors.append(f'line {number}: missing mandatory acceptance criteria')
                if not child.get('obligation', '').strip():
                    errors.append(f'line {number}: empty interpretation')
                refs = child.get('source_refs')
                if refs != [dict(source='design.md', line=number, source_hash=digest(line))]:
                    errors.append(f'line {number}: child has stale or wrong source span')
                qualifiers = child.get('qualifiers', {})
                if (qualifiers.get('inherit_full_parent') is not True or
                        qualifiers.get('inherited_context_lines') != expected_context or
                        qualifiers.get('inherited_preceding_context') != preceding or
                        qualifiers.get('inherited_table_header') != table_header):
                    errors.append(f'line {number}: dropped inherited qualifier/context')
                if child.get('kind') != parent.get('kind') or child.get('authority') != parent.get('authority'):
                    errors.append(f'line {number}: changed child obligation authority')
                if child.get('status') != 'identified' or child.get('evidence_refs'):
                    errors.append(f'line {number}: extraction must not claim implementation or verification')
            risk = re.match(r'^\| (R\d{3}) /', line)
            if risk:
                cells = [value.strip() for value in line.strip('|').split('|')]
                criteria = '\n'.join(value for child in linked for value in child.get('mandatory_criteria', []))
                if len(linked) != 1 or not all(value in criteria for value in cells[2:4]):
                    errors.append(f'line {number}: risk detection or mitigation obligation missing')
            # The original text states this particular table must be atomized.
            if section == '7.6' and line.startswith('|') and ';' in line:
                cells = [value.strip() for value in line.strip('|').split('|')]
                if len(cells) == 3 and cells[0].endswith('verifier'):
                    if len(linked) != len(cells[1].split(';')):
                        errors.append(f'line {number}: aggregate role checkbox hides components')
            if section == '7.2' and line.startswith('|'):
                cells=[value.strip() for value in line.strip('|').split('|')]
                if len(cells)==4 and linked and all(c.get('atomicity')=='reviewed' for c in linked):
                    checks=[c['obligation'] for c in linked if 'illustrative artifact form' not in c['obligation']]
                    if not checks or not all(c.startswith(f'For the selected {cells[0]} competency,') for c in checks):
                        errors.append(f'line {number}: role catalog loses selected-competency applicability')
        if fence:
            code = not code
        if not line.startswith(('|', '#', '```')) and not code:
            preceding = line
        scanned['source_fragments'] += 1
    for prefix, expected in SEMANTIC_EXPECTATIONS:
        matches = [(n, line) for n, line in expected_lines.items() if line.startswith(prefix)]
        if len(matches) != 1:
            errors.append(f'semantic oracle source anchor absent or ambiguous: {prefix}')
            continue
        number, _ = matches[0]
        parent = by_line.get(number, {})
        interpreted = '\n'.join(item.get('obligation', '') for item in child_by_parent[parent.get('id')])
        missing = [phrase for phrase in expected if phrase.casefold() not in interpreted.casefold()]
        if missing:
            errors.append(f'line {number}: semantic obligation/qualifier missing: {missing}')
        checked_semantics.append(dict(source_line=number, anchor=prefix,
                                      checks=expected, result='fail' if missing else 'pass'))
    pending = []
    for parent in parents:
        candidates = [child for child in child_by_parent[parent['id']] if child.get('atomicity') != 'reviewed']
        if not candidates:
            continue
        text = parent['original_text']
        if text.startswith('|'):
            reason = 'This table row combines a domain label with content/method/outcome cells; independent acceptance obligations inside the cells have not been individually reviewed.'
        elif any(word in text.casefold() for word in ('jeśli', 'gdy ', 'albo ', 'lub ', 'nie ', 'bez ', 'wyjąt', '— gdy')):
            reason = 'This clause includes a condition, exception, alternative or prohibition whose scope across candidate sentences still needs independent interpretation.'
        elif len(candidates) > 1:
            reason = 'Multiple candidate sentences share one parent; antecedents and integration obligations have not been independently reviewed.'
        elif any(word in text.casefold() for word in (',', ';', ' oraz ', ' i ')):
            reason = 'This candidate contains a list or conjunction that may conceal separately testable obligations; automatic punctuation splitting would risk losing scope.'
        else:
            reason = 'This single candidate has not yet received a source-to-interpretation review and a concrete independently testable acceptance oracle.'
        pending.append(dict(parent_id=parent['id'], source_line=parent['line'], section=parent['section'],
                            original_text=text, candidate_ids=[child['id'] for child in candidates], reason=reason,
                            required_action='Review the original parent with heading/table/preceding conditions; split independent obligations, resolve antecedents and specify acceptance evidence.'))
    reviewed = sum(c.get('atomicity') == 'reviewed' for c in children)
    return dict(schema_version=1, source_sha256=digest(source),
                audit_method='Independent source-first Markdown walk and separately authored semantic expectations; extractor code and predicates are not imported.',
                result='fail' if errors else 'structural_checks_pass_semantic_review_partial',
                completeness_verified=False,
                independence='Separate automated pass, not an independent person or model review.',
                scope=dict(**scanned, interpretation_units=len(children), reviewed_atomic_units=reviewed,
                           semantic_anchor_count=len(checked_semantics), remaining_parent_reviews=len(pending)),
                errors=errors, semantic_checks=checked_semantics, unresolved=pending,
                limitations=['Source coverage proves preservation, not semantic correctness.',
                             'The semantic oracle checks critical contract regressions only; it is not exhaustive.',
                             'Unreviewed candidates must not be treated as a complete acceptance checklist.',
                             'No extraction status establishes compliance of the runtime with design.md.'])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    source = (ROOT/'design.md').read_text()
    result = audit(source, json.loads((ROOT/'docs/requirements.json').read_text()),
                   json.loads((ROOT/'docs/requirements-atomic.json').read_text()))
    output = json.dumps(result, ensure_ascii=False, indent=2)+'\n'
    if args.check:
        if not (ROOT/'docs/requirements-audit.json').exists() or (ROOT/'docs/requirements-audit.json').read_text() != output:
            raise SystemExit('docs/requirements-audit.json is stale')
    else:
        (ROOT/'docs/requirements-audit.json').write_text(output)
    if result['errors']:
        raise SystemExit('\n'.join(result['errors']))
    print(f'{result["scope"]}; structural audit passed; semantic completeness not established')


if __name__ == '__main__':
    main()
