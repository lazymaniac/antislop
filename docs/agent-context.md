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

The planner invokes `consult_planning` before drafting: research and analysis,
then dependencies from research, architecture from analysis/dependencies, and test
design from analysis/architecture. Each call returns a retained reference immediately;
`read_planning_result` polls its state and reads the report in pages. Consultations
use configured executor profiles and read-only project inspection. Repeated calls
reuse findings for unchanged requirements; `refresh=true` explicitly replaces stale
or failed findings. The plan synthesizes decisions, task ownership and observable
verification, cites the reports, and gives relevant references to workers. Completed
preparation is not repeated as execution tasks. New specialist tasks need a distinct
unresolved question or an explicitly requested deliverable. Implementation review
and final checks still verify actual changes.

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

At the beginning of a turn, `read_turn_input` with no input lists the indexed
inputs, their read status and continuation cursors. Repeated calls with
`input=next` deliver the remaining required pages until
`all_required_complete` is true. Task roles can read their
own current worker report or last completed review with
`read_dependency_result(source="current_worker_report")` or
`read_dependency_result(source="previous_review")`.

Planners declare required consultation references in `planning_inputs`.
Antislop checks that those reports exist before approving a draft and again
before launching a task role. Declare unresolved choices for a code writer
in `open_decisions`. Where the selected rigor supports Options, only writers
with such choices receive a separate selection task. Saved legacy contracts
retain their existing Options policy.

Use `preview_plan_revision` with the accepted `base_revision` and
`patches` keyed by task. Update existing acceptance criteria by ID.
Set `open_decisions: []` explicitly when the decision has been made. The
preview shows which generated Options tasks will be removed. Incremental
reviews receive the changed criteria, complete graph index, and prior findings;
all original material remains available by reference. A scope amendment
requires reading the original scope.

`inspect_plan_witness` counts search results at an exact commit or checks a
libtest selector against a retained passing `--list` receipt for that tree.
Attach the expected counts under `witnesses` to check them before review.
The search result counts matching lines and matching files separately.

If `task_states` shows `answer_recovery`, use its `supersede_receipt` in
`answer_agent`. The original answer is retained while the new answer can
continue the task or name a replacement revision.

## Reusing builds, tests and static analysis

All execution and review roles use `prepare_validation` before a validation
command. Its specification includes exact argv, a checkout-relative directory,
covered targets and a nonsecret environment identity (toolchain, settings,
fixtures, seeds and relevant ignored/external inputs). The runtime compares Git
trees using the existing scratch-index snapshot, including uncommitted and
nonignored untracked source. Commit IDs and worktree paths alone do not establish
equivalence. Identical files across different commits/worktrees can share results.
No repository contents are stored in validation rows or prompts.

The prompt lists recent validation IDs, command previews and statuses. Call
`read_validations(validation=ID)` for the full specification and evidence ID,
or omit `validation` to page the history. Submit the exact specification to
`prepare_validation` to check reuse on the current tree.

For delivery, map each required check to a validation receipt on the final
integrated tree and to the target and totals in its retained output. Reuse a
full-suite receipt for covered targets under matching conditions. Run distinct
scopes, including required ignored tests, separately. Reserve `--nocapture`
for a targeted diagnostic rerun and keep the original failure in the report.

`action=run` claims the check. The agent executes it through its existing CLI
permissions, then calls `complete_validation` with its real exit code, decisive
raw output and any reported errors. Output uses the existing immutable MCP
evidence store. `action=reuse` means read and cite that evidence without rerunning;
it does not mean the consuming agent executed the command. `action=wait` means
another live turn owns this exact check; continue independent work and poll.
There is no execution timeout or automatic termination. An interrupted/failed
owner cannot leave a reusable success or an unrecoverable claim.

Use `problems: []` for a successful observation and put coverage caveats in
`limitations`. Run each check as a direct command. Inline shell commands can
mask an exit status and are rejected. Use `kind: test` for test execution,
`inspection` for code or build checks, or `discovery` for listing tests.
Cargo test infers its kind from its arguments. Its execution receipts require
raw libtest totals; other runners supply `tests_executed`. Inspections can
declare an `expected_exit_code`, such as grep exit 1 to assert absence.

Task reviewers can attach a stable scenario name and its failing validation
receipt to a finding as `reproduction`. Later reviews keep that finding open
until `resolved` includes its ID with `resolution_evidence` for a newer
passing run of the same command, directory, environment and expected exit on
the reviewed tree. Use `carry_findings` to retain an earlier ID and optionally
correct its location. An independently reviewed plan revision can supersede
an invalid reproduction or transfer it with `finding_actions`. For a transfer,
the receiving review includes the passing validation in the named criterion.

Only observations with the expected exit, no reported errors and unchanged source are
reusable. A changed command, target, environment, source tree, missing/corrupt raw
evidence, explicit invalidation or a newer failed observation prevents reuse.
Non-Git directories and dirty submodule contents retain observations without
claiming safe equivalence. `read_validations` pages the retained specifications
and evidence references; prompt indexes remain compact. References survive daemon
restart and are scoped to the conversation.

`invalidate_validation` records a concrete defect or missing required output;
being a different agent/reviewer is not a reason to repeat a successful command.
A build observation does not provide its binary: retrieve retained artifacts when
needed, rebuilding only when the actual output is unavailable or incompatible.
Independent review still inspects source, diffs and acceptance coverage, and runs
new checks for uncovered behavior. Changed external/ignored inputs must be
represented in the declared environment identity; the runtime cannot infer all
inputs read by arbitrary CLI commands. Legacy unstructured reports remain
historical evidence and are not silently promoted into reusable observations.

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
New Options results record the selected ID, all alternative IDs in ranked
order, and the criterion that decided the choice. Choosing an alternative below
first place requires an explicit `ranking_override` for the reviewer to inspect.
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


The planning/validation API additions are intentional: planner consultations,
retained validation specifications/results and their scoped MCP interfaces.
Hologram's name-similarity findings concern different operations:
`Store.invalidate_validation` invalidates reusable evidence, whereas
`terminate_validator` terminates an OS process; `Store.validation` loads one
stored observation, whereas configuration `validate` checks routing/profile
rules. These functions must remain separate. The validation regressions reuse
the existing Core fixture and evidence turn helper. Their calls to `as_str`,
`open` and `finish_turn` are setup/lifecycle operations supporting new scenarios:
one physical execution across commits/worktrees/restart, invalidation after
source/input/error changes and recovery of interrupted ownership. They do not
repeat tests of JSON conversion, opening a store or ending a turn in isolation.
