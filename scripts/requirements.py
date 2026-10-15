#!/usr/bin/env python3
"""Pin the full source and derive reviewable requirements without inventing approval.

The source ledger is deliberately lossless. The atomic ledger is a proposed
interpretation: approved user requirements, design contracts and proposals retain
different authority. Neither generation nor coverage grants verified status.
"""
import argparse
import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[1]


def sha(text):
    return hashlib.sha256(text.encode()).hexdigest()


def extract(text):
    lines = text.splitlines(keepends=True)
    records, risks, section, occurrence = [], [], '0', {}
    in_code = False
    for number, raw in enumerate(lines, 1):
        line = raw.rstrip('\r\n')
        heading = re.match(r'^#{2,6} (\d+(?:\.\d+)*)\.', line)
        if heading and not in_code:
            section = heading[1]
        if not line.strip():
            continue
        digest = sha(line)
        key = (section, digest)
        occurrence[key] = occurrence.get(key, 0) + 1
        kind = ('research' if section.startswith(('4', '21')) else
                'risk_contract' if section.startswith('19') else
                'established' if section == '2' else 'design_contract')
        if in_code:
            kind = 'code'
        if (line.startswith('#') and not in_code) or re.match(r'^\|[ :|-]+\|$', line) or line.startswith('```'):
            kind = 'structure'
        if line.startswith('```'):
            in_code = not in_code
        stage = ('M3' if section.startswith('8') else
                 'M0' if section in ('9.4', '9.5', '13', '20') else
                 'M2Q' if section.startswith(('6.', '7.', '12.', '19')) else
                 'M4' if section.startswith(('11', '9.6')) else 'M1')
        records.append(dict(id=f'D-{section}-{digest[:12]}-{occurrence[key]}',
                            source='design.md', line=number, section=section,
                            original_text=line, source_hash=digest, kind=kind,
                            milestone=stage, status='pending_review',
                            acceptance='Source preservation only; see atomic ledger and audit for interpretation and unresolved review.'))
        risk = re.match(r'^\| (R\d{3}) /', line)
        if risk:
            cells = [v.strip() for v in line.strip('|').split('|')]
            risks.append(dict(id=risk[1], problem=cells[1], method=cells[2], mitigation=cells[3], source_line=number))
    return dict(schema_version=2, source_sha256=sha(text),
                semantic_review='Atomic interpretation and source audit are separate artifacts; no implementation compliance is inferred.',
                atomic_ledger='docs/requirements-atomic.json',
                audit='docs/requirements-audit.json', records=records), dict(version=sha(text), risks=risks)


# Hand-authored interpretations for established compound decisions. Values are
# independent acceptance obligations; full original row remains their parent.
ESTABLISHED = {
    'Komunikacja agentów': ['Allow every agent to send a point-to-point message to any current team agent.', 'Allow every agent to broadcast to the other current team agents.', 'Persist authenticated message authorship, recipients and delivery state.'],
    'Kontekst orkiestratora': ['Manage through task reports, artifacts, verifier decisions and addressed messages.', 'Do not require the orchestrator to read child agent session transcripts.'],
    'Użytkownik': ['Serve programmers working locally.'],
    'Standard rezultatu': ['Use idiomatic code for the task and project.', 'Deliver readable and coherent code.', 'Complete required refactoring.', 'Do not add unnecessary features or mechanisms.', 'Resolve every applicable static-analysis concern.'],
    'Checklisty weryfikacji': ['Version mandatory verification checklists.', 'Define a method, evidence and rejection condition for every checklist item.', 'Account for the entire risk catalog.'],
    'Kryteria refaktoru': ['Evaluate refactoring using language and project standards.', 'Apply the Refactoring.Guru problem and technique catalog where appropriate to the context.'],
    'Architektura': ['Evaluate component boundaries and contracts.', 'Evaluate patterns and introduced-code risks.', 'Use a pattern when it addresses an identified problem.'],
    'Wejścia': ['Model the full domain of every input parameter.', 'Model invalid values and important input combinations.', 'Justify the verification method for the input domain.'],
    'Biblioteki': ['Check available solutions before custom implementation.', 'Assess dependency quality.', 'Assess dependency security.', 'Assess dependency licenses and their obligations.'],
    'Wybór rozwiązania': ['Investigate real alternatives.', 'Choose the best investigated alternative for requirements, context and documented criteria.', 'Independently verify the solution choice.'],
    'Routing CLI': ['Define profiles mapping effort and type of work to CLIs; provider assignments in the source are examples.'],
    'Pary ról': ['Assign a dedicated verifier to each started executor role.'],
    'Konfiguracja': ['Use TOML for configuration.'],
    'Bieżący rezultat': ['Deliver design.md as the current design-stage artifact; implementation follows as the next step.'],
    'Platformy': ['Support macOS.', 'Support Linux.'],
    'Technologia': ['Implement the product in Rust.', 'Use Ratatui for the TUI.'],
    'Integracje AI': ['Integrate through existing provider CLIs only.', 'Do not implement a model-provider API client.'],
    'Dostawcy': ['Support Codex CLI.', 'Support Claude Code.', 'Support Gemini CLI.', 'Allow adding further CLI adapters.'],
    'Zakres zadań': ['Support software tasks from the beginning.', 'Support research tasks from the beginning.', 'Support documents from the beginning.', 'Support analyses from the beginning.'],
    'Jakość': ['Prioritize quality over cost minimization.', 'Independently verify each started executor role, including small tasks.'],
    'Planowanie': ['Create an abstract solution model before implementation.', 'Simulate solution scenarios before implementation.', 'Independently search for counterexamples before implementation.'],
    'Kompletność': ['Do not leave unexplained gaps affecting correctness.', 'Do not leave hidden assumptions affecting correctness.', 'Do not leave unexplained dependencies affecting correctness.'],
    'Boy Scout Rule': ['Assess opportunities to improve used code in every task.', 'Perform justified structural improvements within the task scope.'],
    'Pełne wymagania': ['Persist all requirements from the original request and plan.', 'Independently verify extraction completeness.', 'Independently verify actual fulfillment of every requirement.'],
    'Niezależność': ['Give the verifier a separate session and context.', 'Prefer another model or CLI; the same provider is allowed in a separate session.'],
    'Autonomia': ['Provide supervised approval of every stage and every correction round.', 'Provide a fully autonomous mode within its policy.'],
    'Pełny automat': ['Allow local work, tests and local commits in autonomous mode.', 'Require separate approval for push.', 'Require separate approval for publication.', 'Require separate approval for external messages.', 'Require separate approval for destructive operations.'],
    'Limity MVP': ['Do not impose an agent-count limit in MVP.', 'Do not impose a concurrency limit in MVP.', 'Do not impose a time limit in MVP.', 'Do not impose a cost limit in MVP.', 'Do not impose a delegation-depth limit in MVP.', 'Do not impose a correction-round limit in MVP.'],
    'Wybór orkiestratora': ['Make orchestrator CLI and model configurable.', 'Do not select a default orchestrator provider.'],
    'Katalog pracy': ['Use a separate worktree by default.', 'Allow direct work only when selected for the particular task.'],
    'Personalizacja MVP': ['Use built-in roles and templates in MVP.', 'Allow editing CLI and effort assignments in MVP.', 'Defer custom roles and templates beyond MVP.'],
    'Restart komputera': ['Resume interrupted sessions when Architect starts after reboot.', 'Check the actual session state before resuming.'],
    'Działanie w tle': ['Keep the local executor running when the TUI closes.', 'Support reattachment.', 'Support recovery after failure.'],
    'Główny interfejs': ['Provide an orchestrator chat panel.', 'Provide an agent graph or tree panel.'],
    'Graf': ['Support arrow-key graph navigation.', 'Display actual work status.', 'Display communication.', 'Provide agent details.'],
    'Sesje agentów': ['Allow inspecting agent sessions.', 'Allow writing directly to a selected agent.'],
    'Widok diff i recenzja': ['Provide workspace diff inspection with file listing and change statistics.', 'Support adding and deleting diff comments on files and specific diff lines.', 'Orchestrator listens for unresolved diff comments and delegates targeted work.', 'Allow triggering on-demand code review directly from the diff view.'],
    'Język': ['Use English for TUI, code, identifiers and product documentation.', 'Keep this design document in Polish.'],
}

# These splits are semantic decisions, not punctuation heuristics. Each phrase
# names one artifact or obligation while retaining all original qualifiers.
CRITICAL_SPLITS = {
    '| `supervised` |': [
        'In supervised mode, require user approval before starting each stage.',
        'In supervised mode, require user approval for every correction round.',
        'In supervised mode, require user acceptance of each final stage result.',
        'A result-acceptance card may also present and approve the explicit scope of the next stage; this is an optional combined approval, not implicit authorization.',
        'In supervised mode, do not automatically open a correction round after a verifier rejects the result.'],
    '| `autonomous` |': [
        'In autonomous mode, runtime opens successive stages after their applicable gates are satisfied.',
        'In autonomous mode, runtime opens correction rounds after their applicable gates are satisfied.',
        'In autonomous mode, permit local work within the policy.',
        'In autonomous mode, permit local tests within the policy.',
        'In autonomous mode, permit local commits within the policy.',
        'In autonomous mode, require separate approval for push.',
        'In autonomous mode, require separate approval for publication.',
        'In autonomous mode, require separate approval for external messages.',
        'In autonomous mode, require separate approval for destructive operations.',
        'A missing necessary product decision may require the user even in autonomous mode.'],
    'Zmiana trybu jest jawna, zapisana': [
        'Make an autonomy-mode change explicit.',
        'Persist every autonomy-mode change.',
        'Apply an autonomy-mode change to future actions.',
        'Do not retrospectively approve prior changes when changing autonomy mode.',
        'Do not remove verifier concerns when changing autonomy mode.',
        'Changing the accepted task scope invalidates the affected approvals.',
        'Changing the accepted task scope requires reassessment.'],
    'Oddzielamy zgodę na **etap**': [
        'Distinguish approval of a stage from approval of an action.',
        'Approval of local implementation does not authorize push.',
        'Approval of local implementation does not authorize publication.',
        'Approval of local implementation does not authorize external messages.',
        'Approval of local implementation does not authorize destructive operations.',
        'A local commit includes only the specified task changes.',
        'A local commit must not include unrelated user changes.'],
    '| Orchestrator | Orchestration verifier |': [
        'For the selected Orchestrator competency, independently review substantive team changes.',
        'For the selected Orchestrator competency, independently review substantive scope changes.',
        'For the selected Orchestrator competency, independently review substantive strategy changes.',
        'For the selected Orchestrator competency, check that decisions align with the objective.',
        'Typical Orchestrator artifacts are a TeamPlan change proposal or a management decision; these are illustrative artifact forms.'],
    '| Context analyst | Context verifier |': [
        'For the selected Context analyst competency, independently inspect actual behavior.',
        'For the selected Context analyst competency, independently inspect sources.',
        'For the selected Context analyst competency, check dependency coverage.',
        'For the selected Context analyst competency, inspect applicable standards.',
        'For the selected Context analyst competency, check closure of unknowns.',
        'Typical Context analyst artifacts are ContextModel and a verified knowledge register; these are illustrative artifact forms.'],
    '| Requirements analyst | Requirements verifier |': [
        'For the selected Requirements analyst competency, check alignment with user intent.',
        'For the selected Requirements analyst competency, independently inspect contradictions.',
        'For the selected Requirements analyst competency, check acceptance-criteria completeness.',
        'Typical Requirements analyst artifacts are a specification and criteria map; these are illustrative artifact forms.'],
    '| Planner | Plan verifier |': [
        'For the selected Planner competency, independently inspect the abstract model.',
        'For the selected Planner competency, check invariants.',
        'For the selected Planner competency, perform independent simulation.',
        'For the selected Planner competency, seek counterexamples.',
        'For the selected Planner competency, inspect scope.',
        'For the selected Planner competency, check evidence completeness.',
        'Typical Planner artifacts are SolutionModel, SimulationReport and a task DAG; these are illustrative artifact forms.'],
    '| Options analyst | Options verifier |': [
        'For the selected Options analyst competency, check coverage of available alternatives.',
        'For the selected Options analyst competency, independently assess comparison fairness.',
        'For the selected Options analyst competency, inspect evidence.',
        'For the selected Options analyst competency, account for uncertainty.',
        'For the selected Options analyst competency, assess justification for selecting the best solution.',
        'Typical Options analyst artifacts are OptionsAnalysis and SolutionDecision; these are illustrative artifact forms.'],
    '| Architect | Architecture verifier |': [
        'For the selected Architect competency, check fit with the project.',
        'For the selected Architect competency, assess simplicity.',
        'For the selected Architect competency, check reuse of existing solutions.',
        'Typical Architect artifacts are ADRs and module contracts; these are illustrative artifact forms.'],
    '| Dependency analyst | Dependency verifier |': [
        'For the selected Dependency analyst competency, independently inspect alternatives to custom code.',
        'For the selected Dependency analyst competency, inspect dependency provenance.',
        'For the selected Dependency analyst competency, inspect licenses.',
        'For the selected Dependency analyst competency, inspect dependency security.',
        'For the selected Dependency analyst competency, assess dependency maintenance cost.',
        'Typical Dependency analyst artifacts are DependencyDecision and LicenseReport; these are illustrative artifact forms.'],
    '| Implementer | Code verifier |': [
        'For the selected Implementer competency, independently check model conformance.',
        'For the selected Implementer competency, check standards conformance.',
        'For the selected Implementer competency, check requirements conformance.',
        'For the selected Implementer competency, assess simplicity.',
        'For the selected Implementer competency, inspect responsibilities.',
        'For the selected Implementer competency, check absence of unnecessary additions.',
        'Typical Implementer artifacts are a patch, TraceabilityMap and a standards report; these are illustrative artifact forms.'],
    '| Refactorer | Refactor verifier |': [
        'For the selected Refactorer competency, check closure of solution debt.',
        'For the selected Refactorer competency, independently check preservation of contracts.',
        'For the selected Refactorer competency, assess structural improvement.',
        'For the selected Refactorer competency, check absence of speculative abstractions.',
        'Typical Refactorer artifacts are RefactorReport, a patch and behavior-preservation evidence; these are illustrative artifact forms.'],
    '| Static analysis operator | Static analysis verifier |': [
        'For the selected Static analysis operator competency, inspect analyzer coverage.',
        'For the selected Static analysis operator competency, independently inspect configurations.',
        'For the selected Static analysis operator competency, check absence of unresolved concerns.',
        'For the selected Static analysis operator competency, check absence of rule-weakening attempts.',
        'Typical Static analysis operator artifacts are StaticAnalysisReport and raw results; these are illustrative artifact forms.'],
    '| Compliance auditor | Compliance verifier |': [
        'For the selected Compliance auditor competency, independently account for every requirement.',
        'For the selected Compliance auditor competency, account for every step of the full plan.',
        'For the selected Compliance auditor competency, assess realization quality.',
        'For the selected Compliance auditor competency, check absence of extraction gaps.',
        'For the selected Compliance auditor competency, check that evidence is current.',
        'Typical Compliance auditor artifacts are ComplianceAudit and RequirementLedger accounting; these are illustrative artifact forms.'],
    '| Test designer | Test design verifier |': [
        'For the selected Test designer competency, independently check that scenarios derive from requirements.',
        'For the selected Test designer competency, check that scenarios detect probable defects.',
        'Typical Test designer artifacts are a test plan and requirements map; these are illustrative artifact forms.'],
    '| Test implementer | Test verifier |': [
        'For the selected Test implementer competency, independently inspect assertions.',
        'For the selected Test implementer competency, inspect actual execution paths.',
        'For the selected Test implementer competency, inspect isolation.',
        'For the selected Test implementer competency, inspect mocks.',
        'For the selected Test implementer competency, assess test resilience.',
        'Typical Test implementer artifacts are tests and execution results; these are illustrative artifact forms.'],
    '| Security analyst | Security verifier |': [
        'For the selected Security analyst competency, independently inspect the threat model.',
        'For the selected Security analyst competency, inspect coverage.',
        'For the selected Security analyst competency, inspect reproductions.',
        'For the selected Security analyst competency, inspect false alarms.',
        'For the selected Security analyst competency, inspect fixes.',
        'Typical Security analyst artifacts are a threat model and findings; these are illustrative artifact forms.'],
    '| Performance engineer | Performance verifier |': [
        'For the selected Performance engineer competency, independently inspect benchmark representativeness.',
        'For the selected Performance engineer competency, inspect the baseline.',
        'For the selected Performance engineer competency, check absence of semantic regression.',
        'Typical Performance engineer artifacts are measurements and recommendations or a patch; these are illustrative artifact forms.'],
    '| Integrator | Integration verifier |': [
        'For the selected Integrator competency, independently inspect the entire change set.',
        'For the selected Integrator competency, check inter-module contracts.',
        'For the selected Integrator competency, check regressions.',
        'A typical Integrator artifact is a delivery candidate; this is an illustrative artifact form.'],
    '| Researcher | Research verifier |': [
        'For the selected Researcher competency, independently assess source reliability.',
        'For the selected Researcher competency, check source currency.',
        'For the selected Researcher competency, check source coverage.',
        'For the selected Researcher competency, distinguish facts from inferences.',
        'A typical Researcher artifact is a linked report; this is an illustrative artifact form.'],
    '| Data analyst | Analysis verifier |': [
        'For the selected Data analyst competency, independently inspect data.',
        'For the selected Data analyst competency, inspect assumptions.',
        'For the selected Data analyst competency, inspect calculations.',
        'For the selected Data analyst competency, inspect units.',
        'For the selected Data analyst competency, check reproducibility.',
        'Typical Data analyst artifacts are an analysis and script or notebook; these are illustrative artifact forms.'],
    '| Writer | Editorial/factual verifier |': [
        'For the selected Writer competency, independently check factual correctness.',
        'For the selected Writer competency, inspect citations.',
        'For the selected Writer competency, check coherence.',
        'For the selected Writer competency, check alignment with the brief.',
        'A typical Writer artifact is a document; this is an illustrative artifact form.'],
    '| Delivery author | Delivery verifier |': [
        'For the selected Delivery author competency, independently compare the report with the actual result.',
        'For the selected Delivery author competency, compare instructions with the actual result.',
        'A typical Delivery author artifact is the final report; this is an illustrative artifact form.'],
    '- Dla bugfixu: reprodukcja problemu': [
        'For a bugfix, reproduce the problem before changing it.',
        'For a bugfix, check the result after the change.',
        'An inability to reproduce is an explicit gap requiring other evidence.',
        'Do not treat inability to reproduce as implicit success.'],
    'Orkiestrator proponuje `TeamPlan` na podstawie:': [
        'Base the proposed TeamPlan on the kind of result.',
        'Base the proposed TeamPlan on task complexity.',
        'Base the proposed TeamPlan on actual risks.',
        'Base the proposed TeamPlan on dependencies.',
        'Base the proposed TeamPlan on context size.',
        'Base the proposed TeamPlan on project modules.',
        'Base the proposed TeamPlan on available CLIs.',
        'Base the proposed TeamPlan on required tools.',
        'Base the proposed TeamPlan on the quality profile.',
        'Runtime validates the proposed plan before starting it.',
        'Give each added agent a justified responsibility.',
        'Absence of numerical agent limits does not authorize agents without a task.'],
    'Bootstrap korzysta z wbudowanej bazy ról': [
        'Bootstrap uses the built-in role base.',
        'Bootstrap uses an explicitly selected orchestrator profile.',
        'Permit discovery before a complete implementation plan exists.',
        'Permit requirements analysis before a complete implementation plan exists.',
        'Require approval of the appropriate stage in supervised mode.',
        'The orchestrator extends the team after obtaining context and RiskCoverage.',
        'Implementation gates apply to work on the solution.',
        'Analyses needed to develop the plan have their own criteria.',
        'Do not require a completed implementation plan for analyses needed to develop that plan.'],
    'Obowiązkowe warunki:': [
        'Treat every listed team-selection condition as mandatory.'],
    '- Każde zadanie ma właściciela, rezultat': [
        'Every task has an owner.',
        'Every task has a result.',
        'Every task has verifiable completion criteria.'],
    '- Zadanie implementacyjne ma aktualne': [
        'An implementation task requires a current context_ready gate.',
        'An implementation task requires a current plan_ready gate.',
        'An implementation task links the relevant model fragment.',
        'Do not waive implementation gates for a small change.'],
    '- Wybór wariantu ma `solution_selected`': [
        'Bind the alternative choice to the solution_selected gate.',
        'Require architecture assessment before implementation to the extent verifiable from the plan.',
        'Require input assessment before implementation to the extent verifiable from the plan.',
        'Require dependency assessment before implementation to the extent verifiable from the plan.',
        'Close those same gates with code and final-integration evidence before delivery.'],
    '- Każda uruchomiona rola wykonawcza ma przypisanego': [
        'Assign an independent verifier to every started executor role.'],
    '- Nie ma cykli w grafie zależności zadań': [
        'The task dependency graph is acyclic.',
        'The management delegation graph is acyclic.'],
    '- Każdy aktywny zapis ma określony zakres': [
        'Every active write has an explicit scope.',
        'Every active write has a defined integration method.'],
    '- Są zdefiniowane warunki eskalacji': [
        'Define escalation conditions.', 'Define stopping conditions.',
        'Define completion conditions.'],
    '- Każda nowa runda wskazuje nierozwiązany problem': [
        'Every new round identifies an unresolved problem.',
        'Every new round identifies its action plan.',
        'MVP imposes no numerical round limit.',
        'MVP imposes no numerical agent limit.'],
    'Graf komunikacji może zawierać cykle.': [
        'Permit cycles in the communication graph.',
        'Conversation cycles must not create an infinite execution loop.',
        'Substantive conversations concern a specific task.',
        'Conversations terminate with an artifact, a decision or escalation.'],
    'To katalog kompetencji. Każdy run obejmuje': [
        'Treat the role table as a competency catalog.',
        'Every run includes a context pair.',
        'Every run includes a requirements pair.',
        'Every run includes an options pair.',
        'Every run includes a planning pair.',
        'Every run includes a compliance pair.',
        'Every run includes a delivery pair.',
        'Every run includes review of substantive orchestrator decisions.',
        'Software tasks additionally include an architecture pair.',
        'Software tasks additionally include a dependency pair.',
        'Software tasks additionally include an implementation pair.',
        'Software tasks additionally include a refactor pair.',
        'Software tasks additionally include a static-analysis pair.',
        'Software tasks additionally include a test-design pair.',
        'Software tasks additionally include a test-implementation pair.',
        'Software tasks additionally include an integration pair.',
        'Software assessments may be brief but remain mandatory.',
        'Select remaining specialist pairs for actual threats.',
        'RiskCoverage may require a separate security or performance pair.',
        'Research without code does not require fictitious compilation tests.'],
    'Każda uruchomiona rola ma własnego wykonawcę': [
        'Every started role has its own executor and dedicated verifier.',
        'A small task may have shorter artifacts.',
        'A small task may have fewer specialist roles.',
        'Do not share one verifier agent between several executor roles, including small tasks.',
        'Agents may execute sequentially.',
        'Team membership does not require a permanently running CLI process.',
        'Do not perform refactoring when no file transformations are needed.',
        'The mandatory refactoring assessment may establish that no transformations are needed.'],
    'Wszystkie szablony dziedziczą wspólną bazę': [
        'Every template inherits the common role base.',
        'Every template inherits the applicable role pairs from section 7.2.',
        'The template table describes specialization of those shared obligations.',
        'A template defines required competencies.',
        'A template defines required gates.',
        'The orchestrator may extend the template.',
        'The orchestrator may shorten queues.',
        'The orchestrator may change work partitioning.',
        'Template adaptation cannot remove required verification.',
        'Every team composition change has a rationale.',
        'Every team composition change has a history event.'],
    'Requirements analyst wyodrębnia': [
        'Extract atomic requirements while preserving hierarchy.',
        'Preserve every condition and exception from the source.',
        'Preserve every limitation and quality expectation from the source.',
        'Preserve negative requirements and acceptance details from the source.'],
    'Każdy rekord zawiera: `requirement_id`': [
        'Retain requirement identity, source span and original wording.',
        'Record a precise interpretation and parent relation.',
        'Record mandatory acceptance criteria.',
        'Record executor and verifier ownership.',
        'Record dependencies, revision and evidence.',
        'Keep the original parent when splitting a requirement.',
        'Verify the parent only after all mandatory children and integration criteria pass.'],
    'Requirements verifier niezależnie': [
        'Requirements verifier reads the entire original source independently.',
        'Compare every obligation-bearing source fragment with the ledger.',
        'Check interpretation accuracy independently of executor extraction.',
        'Resolve ambiguity explicitly; do not silently make requirements optional.'],
    'Końcowy `ComplianceAudit`': [
        'Compare the integrated result with the complete original source and ledger.',
        'Look for requirements omitted during extraction.',
        'Use a dedicated verifier to check audit coverage and evidence.',
        'Require all criteria of every mandatory requirement.',
        'Block requirements_complete for absence, superficial implementation, stale evidence, contradictory results or unapproved simplification.'],
    'Usunięcie, osłabienie': [
        'Require an explicit user decision and new revision for removal, weakening, deferral or reinterpretation.',
        'Do not grant autonomous mode authority to change requirements.',
        'Expose original requirements, coverage, evidence, omissions and revision differences in the TUI.',
        'Do not substitute a completion percentage for every mandatory acceptance criterion.'],
    'Definicja pozycji zawiera': [
        'Version each checklist item with stable identity and associated risks.',
        'Specify a control question, method, evidence type, rejection rule and permitted not-applicable condition.',
        'Record requirement and artifact revisions, requirement IDs, actual method, evidence and findings in each result.',
        'Use pass, fail, blocked or not_applicable as the result state.',
        'Do not treat missing data as pass; unresolved issues are blocked.',
        'Require a scope-specific not-applicable rationale confirmed by the appropriate verifier.'],
    '`RiskCoverage` obejmuje': [
        'Account for every risk ID in the pinned catalog version.',
        'Assign applicability and an owner; independently verify exclusions.',
        'Give each applicable risk an executor and dedicated verifier.',
        'Add the missing role pair when competencies are absent.',
        'Do not drop a risk between planning and completion.',
        'Give each reviewer the full responsibility map as well as assigned checks.'],
    'Runtime odrzuca raport': [
        'Reject empty checklist positions.', 'Reject missing risks.',
        'Reject stale evidence.', 'Reject a disallowed author.',
        'Independently evaluate evidence quality; runtime format checks do not establish honest reasoning.',
        'Record newly discovered risks with an owner and check, then extend the catalog.'],
    'Każde przekazanie utrwala': [
        'Preserve invariants, contracts, negative requirements, open findings and provenance at each handoff.',
        'Invalidate dependent facts when source code or configuration changes.',
        'Invalidate dependent models and simulations when their source changes.',
        'Compare the resumed context package with the current snapshot.',
        'Do not plan using knowledge marked stale.'],
    'Trwałe źródła: wymagania': [
        'Persist the requirements as durable context sources.',
        'Persist ContextModel as a durable context source.',
        'Persist SolutionModel as a durable context source.',
        'Persist simulation results as durable context sources.',
        'Persist quality standards as durable context sources.',
        'Persist decisions as durable context sources.',
        'Persist the plan as a durable context source.',
        'Persist contracts as durable context sources.',
        'Persist accepted artifacts as durable context sources.',
        'Persist findings as durable context sources.',
        'Persist check results as durable context sources.',
        'Persist the source index as a durable context source.',
        'Give each agent a context package relevant to its own task.',
        'Include references to the complete source materials in that task package.'],
    'Pakiet zawiera obowiązkowo:': [
        'Include the objective in every mandatory context package.',
        'Include the scope in every mandatory context package.',
        'Include acceptance criteria in every mandatory context package.',
        'Include project instructions in every mandatory context package.',
        'Include constraints in every mandatory context package.',
        'Include the current revision in every mandatory context package.',
        'Include inputs in every mandatory context package.',
        'Include the prohibition on self-authorized permission changes in every mandatory context package.',
        'Include stopping conditions in every mandatory context package.',
        'Identify the author of each summary.',
        'Retain source references for each summary.',
        'Do not substitute a summary for the original source.',
        'After compaction, supply the key constraints to the agent again.',
        'After compaction, verify that the relied-on artifacts are current.'],
    'Instrukcje projektu (np.': [
        'Record project instructions with their directory scope, including AGENTS.md, CLAUDE.md and GEMINI.md when present; these filenames are examples, not an exhaustive list.',
        'Resolve conflicting project instructions through an explicit decision.',
        'Do not allow content from a discovered document or webpage to increase runtime permissions.'],
    'Wiadomości przechodzą przez daemon;': [
        'Route agent messages through the daemon.',
        'Persist a message before delivery.',
        'Give each message an identifier.',
        'Retain delivery receipts for messages.',
        'Control queue inflow into memory through backpressure.',
        'Support the question message type.',
        'Support the answer message type.',
        'Support the handoff message type.',
        'Support the finding message type.',
        'Support the decision message type.',
        'Support the status message type.',
        'Support the user_instruction message type.',
        'Represent large materials using references to artifacts.',
        'Retain queued messages durably when the TUI disconnects.'],
    'Nie udajemy gwarancji exactly-once': [
        'Do not claim a provider exactly-once delivery guarantee.',
        'Deduplicate internal events.',
        'Deduplicate internal operations.',
        'Represent uncertain delivery as delivery_unknown.',
        'Reconcile uncertain delivery with the actual session.',
        'Do not automatically repeat external actions with unknown outcomes.'],
    'Zakresy komunikacji i subskrypcji': [
        'Declare explicit communication scopes.',
        'Declare explicit subscription scopes.',
        'Use broadcast for changes to shared assumptions.',
        'Direct substantive conversations to specific roles.',
        'Limit status storms.',
        'Limit copying complete transcripts.'],
    'Porównywalność wynika': [
        'Retain every checklist result using common definitions, methods and snapshots.',
        'Do not lower mandatory criteria based on severity.',
        'Do not use an average or coverage percentage to excuse an omitted requirement.',
        'Invalidate dependent results after a checklist, requirement or material-input revision.'],
}


def classify(record, table_header, context):
    line, section = record['original_text'], record['section']
    if record['kind'] in ('structure', 'code'):
        return record['kind'], 'non_normative'
    if line.startswith('|') and (line == table_header or '---' in line or 'ID /' in line or 'Obszar |' in line):
        return 'structure', 'non_normative'
    if (section == '2' or (section == '6.2' and line.startswith('|')) or
            line.startswith('**Ustalone:**')):
        return 'mandatory', 'established_user_requirement'
    if section.startswith(('4', '21')) or line.startswith(('Podstawa:', 'Źródła przeczytane', 'Research wykonano')):
        return 'research', 'non_normative'
    if section == '20':
        return 'open_decision', 'unresolved'
    if (line.startswith(('Proponowan', 'Propozycja', 'Poza pierwszym MVP', 'Przykłady pytań')) or
            (section == '8.3' and table_header and 'Klawisz' in table_header) or
            (section == '7.4' and line.startswith('|')) or
            (section == '9.1' and 'biblioteki' in line) or
            (section == '13' and line.startswith('Proponowane profile'))):
        return 'proposal', 'proposed_implementation'
    return 'design_obligation', 'design_contract_pending_approval'


def sentence_units(line):
    # Preserve alternatives, conjunctions, conditions and exceptions within a
    # sentence. No comma/"and" heuristic can establish semantic atomicity.
    return [part.strip() for part in re.split(r'(?<=[.!?])\s+(?=[A-ZĄĆĘŁŃÓŚŹŻ`„])', line) if part.strip()]


def atomic_extract(text, ledger):
    source_lines = text.splitlines()
    records, parents, titles = [], [], []
    table_header, paragraph_context = '', ''
    for source in ledger['records']:
        line = source['original_text']
        if line.startswith('#') and source['kind'] != 'code':
            level = len(line) - len(line.lstrip('#'))
            titles = [item for item in titles if item['level'] < level]
            titles.append(dict(level=level, line=source['line'], text=line))
            table_header = ''
            paragraph_context = ''
        if line.startswith('|') and (source['line'] == 1 or not source_lines[source['line']-2].startswith('|')):
            table_header = line
        if not line.startswith('|'):
            table_header = ''
        kind, authority = classify(source, table_header, paragraph_context)
        parent = dict(id=source['id'], source='design.md', line=source['line'],
                      original_text=line, source_hash=source['source_hash'], section=source['section'],
                      kind=kind, authority=authority, context=list(titles),
                      table_header=table_header,
                      preceding_context=paragraph_context,
                      child_ids=[], integration_criterion='All mandatory children and their joint behavior must be verified on the same current result.',
                      status='identified')
        parents.append(parent)
        if kind in ('structure', 'code', 'research', 'open_decision', 'proposal'):
            parent['integration_criterion'] = None
            if not line.startswith(('|', '#', '```')) and kind != 'code':
                paragraph_context = line
            continue
        interpretation_method = 'sentence_candidate'
        acceptance_criteria = []
        units = sentence_units(line)
        if source['section'] == '2' and line.startswith('|'):
            cells = [cell.strip() for cell in line.strip('|').split('|')]
            units = ESTABLISHED.get(cells[0], [cells[1]])
            interpretation_method = 'reviewed_established_decision' if cells[0] in ESTABLISHED else 'single_decision_candidate'
        risk_match = re.match(r'^\| (R\d{3}) /', line)
        if risk_match:
            cells = [cell.strip() for cell in line.strip('|').split('|')]
            units = [f'Check {risk_match[1]} against the pinned task scope: {cells[1]}']
            acceptance_criteria = [f'Apply the source detection method: {cells[2]}',
                                   f'Check effectiveness of the source mitigation: {cells[3]}',
                                   'Provide current scoped evidence, or a justified independently reviewed not_applicable decision.']
            interpretation_method = 'reviewed_catalog_risk_check'
        for prefix, children in CRITICAL_SPLITS.items():
            if line.startswith(prefix):
                units, interpretation_method = children, 'reviewed_m2q_contract'
                break
        # Explicit role-checklist contract makes each semicolon-separated check
        # mandatory; method/rejection and the whole role row remain inherited.
        if source['section'] == '7.6' and line.startswith('|'):
            cells = [cell.strip() for cell in line.strip('|').split('|')]
            if len(cells) == 3 and cells[0].endswith('verifier') and ';' in cells[1]:
                units = [f'{cells[0]}: {part.strip().rstrip(".")}' for part in cells[1].split(';')]
                interpretation_method = 'explicit_role_checklist_components'
                acceptance_criteria = [cells[2]]
        for index, unit in enumerate(units, 1):
            semantic_id = f'R-{sha(source["id"] + ":" + unit)[:20]}'
            child = dict(id=semantic_id, semantic_id=semantic_id, parent_id=parent['id'],
                         source_refs=[dict(source='design.md', line=source['line'], source_hash=source['source_hash'])],
                         original_text=line, obligation=unit, kind=kind, authority=authority,
                         qualifiers=dict(inherit_full_parent=True, inherited_context_lines=[item['line'] for item in titles],
                                         inherited_preceding_context=paragraph_context,
                                         inherited_table_header=table_header),
                         atomicity='reviewed' if interpretation_method.startswith(('reviewed_', 'explicit_')) else 'needs_semantic_review',
                         interpretation_method=interpretation_method,
                         mandatory_criteria=acceptance_criteria or [unit, 'Preserve and satisfy the full inherited source conditions, exceptions and prohibitions.'],
                         owner=None, verifier=None,
                         dependencies=[], version=1, evidence_refs=[], status='identified')
            records.append(child)
            parent['child_ids'].append(semantic_id)
        if not line.startswith(('|', '#', '```')) and kind != 'code':
            paragraph_context = line
    return dict(schema_version=1, source='design.md', source_sha256=sha(text),
                source_policy='Section 2 and explicitly established autonomy/no-limit clauses have user authority; other design obligations retain pending-approval authority. Examples, proposals, code and research are not promoted to mandatory requirements.',
                interpretation_language='English for reviewed obligations; verbatim Polish source for candidates.',
                semantic_review=dict(status='partial',
                    scope='Reviewed all established decisions, M2Q source preservation/compliance/invalidation contracts, explicit role-checklist components and individual catalog-risk acceptance checks.',
                    limitation='Sentence candidates may contain multiple obligations. Full original wording and context are authoritative; pending candidates require semantic review before use as complete acceptance criteria. Generation never marks requirements verified.'),
                parents=parents, requirements=records)


def serialize(data):
    return json.dumps(data, ensure_ascii=False, indent=2)+'\n'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    text = (ROOT/'design.md').read_text()
    ledger, risks = extract(text)
    atomic = atomic_extract(text, ledger)
    for path, data in [('docs/requirements.json', ledger), ('fixtures/risks.json', risks),
                       ('docs/requirements-atomic.json', atomic)]:
        output, target = serialize(data), ROOT/path
        if args.check:
            if not target.exists() or target.read_text() != output:
                raise SystemExit(f'{path} is stale')
        else:
            target.write_text(output)
    # Coverage has a dedicated independent checker, deliberately not imported.
    print(f'{len(ledger["records"])} source records; {len(atomic["requirements"])} interpretation units; semantic review partial')


if __name__ == '__main__':
    main()
