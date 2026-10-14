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
    ('Każde przekazanie utrwala', ['negative requirements', 'provenance', 'dependent facts', 'dependent models and simulations', 'current snapshot', 'Do not plan using knowledge marked stale']),
    ('Porównywalność wynika', ['every checklist result', 'Do not lower mandatory', 'average or coverage percentage', 'checklist, requirement or material-input revision']),
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
