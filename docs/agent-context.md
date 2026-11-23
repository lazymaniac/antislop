# Agent context and retained results

Agent prompts use Markdown. MCP arguments, tool schemas, persisted contracts and
structured results continue to use JSON. The common prompt boundary renders the
current turn, role policy, inbox, assignment, allocation, inputs/evidence and
continuation sections, omitting empty sections. Reference data uses delimiters
chosen to avoid collisions with the data itself. Embedded headings, fences or
apparent instructions do not change the role or CLI permission policy.

Session state distinguishes an initial launch, a resumed provider session and a
role reintroduction after daemon restart. Continuation identifies role work,
independent review or a task round. The central prompt boundary supplies round
handoffs for resumed sessions too. Handoffs contain changed files and commit
ranges, unresolved finding IDs, interruptions and relevant comments. Previous
reports and verdicts remain available through MCP instead of being repeated in
the handoff. Presentation changes preserve input generations and checkpoints;
semantic changes to a writer invalidate its generated options selection too.

The planner submits complete plans with `submit_plan`. The orchestrator uses
`revise_plan` for incremental changes and `request_replan` to delegate a complete
replacement. Plan comments name the appropriate operation. A successful revision
or replan request is recorded against its turn, so an audit response can leave a
finding open with that remediation route without submitting a duplicate full
plan; the request itself confers no approval. Activation guidance
is included only for contracts explicitly requiring `antislop activate`; the
new prompt format itself never activates a build or interrupts existing work.

## Read interfaces

- `read_plan_context` reads the bound plan's scope, source references, plan
  document and current task index. `read_task_contract` reads a full contract by
  local name or runtime UUID. Planning and control roles can inspect their bound
  plan; task workers/reviewers can read their own contract, and generated options
  workers/reviewers can also read their owner's contract.
- Both reads return `text`, `plan`, `revision`, `cursor` and `complete`. Continue
  with the returned cursor until `complete=true`. Cursors bind the reader turn,
  plan revision and selected content. A changed revision/context produces an
  explicit error requiring a new read without the cursor.
- `read_dependency_result` retains its dependency, `self`, current-round and
  immutable `result_ref` reads. `view=review` reads a retained reviewer verdict
  for the selected round; it also pages until complete. Legacy round reports
  with no worker-result reference can be read by their owner with an explicit
  `round`. Historical content never proves acceptance against current inputs.
- `read_audit` takes `audit_ref=audit:<event sequence>` and pages one auditor's
  report. Verdict/findings and the reference reach the orchestrator separately
  from aggregate plan results. Every new final audit retains its report, also
  outside code-review rigor. Older events without a report explicitly say it
  is unavailable; no aggregate report is substituted. Large audit text uses the
  existing disk-backed content store rather than enlarging SQLite event rows.

All pages contain at most 16 KiB of UTF-8 text. Access checks retain conversation,
turn, task generation and role boundaries. A cursor is not a permission grant.
The full history interfaces continue to retain original reports and events.

## Historical report inputs

Authored and stored task contracts accept optional `result_inputs`:

```json
{
  "result_inputs": [
    {"alias": "original-measurement", "result_ref": "exact-worker-turn-uuid"}
  ]
}
```

A worker or its reviewer reads it with
`read_dependency_result(dependency="original-measurement")`. The reference must
identify a finished worker report in this conversation. It remains exact even
when a newer report exists. Blocker reports remain marked as blocked. Aliases
must be unique and cannot collide with task/dependency names, runtime task IDs,
`self` or `selected_approach`. Explicit inputs create no execution dependency
and confer no approval. They are copied into generated options contracts;
changing an input goes through ordinary contract revision and reassessment.

Omitted inputs default to an empty list. Historical contracts, options goals,
selection reports and audit events remain readable without migration. Known
legacy generated options goals are normalized only for contract comparison;
new goals refer to the owner contract through MCP. Retaining an options result
across plans also checks its owner's contract, so a changed owner cannot reuse
an obsolete selection.

Reports, selections and evidence references are assembled separately. A
selection's rationale appears once, including for legacy reports that appended
evidence references to that same rationale. MCP evidence is retained as metadata
and read using `read_evidence`. The completion limit of 64 references counts
both automatically recorded and explicitly retained references together.
Reviewer schemas require the actual obligation IDs and enumerate permitted
`findings.check` values without copying full obligation descriptions.

## Measurements and known limits

`prompt_sections` events record UTF-8 bytes by rendered section and for the body.
`prompt_transport` records body bytes and the exact appended footer bytes. These
are measurements of generated prompt text, not the provider's entire context,
token usage or billed tokens. The footer is derived from the same role/contract
scope as `tools/list`.

Regression fixtures exercise the audited duplication patterns: repeated graphs,
owner contracts in options goals, rationale plus evidence, aggregate reports
misidentified as audits, and historical inputs outside normal dependencies.
The original audit corpus comparison was waived for this implementation.
No percentage reduction or agent execution budget is imposed.

The Hologram block in `AGENTS.md` remains generator-owned. A provider may truncate
repository instructions before the runtime sees them; these prompt byte counts
neither detect nor quantify that provider-side truncation.

`hologram review` reports the intentional contract/session API changes. Its
name-similarity findings for `same_contract` versus filesystem `same_content`,
and `historical_result` versus terminal-tool `tool_result`, concern distinct
operations: normalized contract equality, file metadata equality,
conversation-scoped stored-report lookup and terminal-result decoding. They do
not share an implementation. Existing paging, result lookup, content storage,
round history and test fixtures are reused.
