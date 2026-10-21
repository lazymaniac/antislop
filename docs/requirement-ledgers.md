# Requirement and plan-item ledgers

Schema 7 adds a durable run and two versioned ledgers to the lean runtime.
A run groups one order and its successive task plans. Plans name their `run_id`
explicitly; corrections keep that identity. A new manually created run starts
at the latest user message, while the first run includes the interview.

## Sources and records

User sources reference existing message IDs. Imported documents and user
corrections live in immutable, content-addressed `sources/*.txt` artifacts
under the private state directory (normally `.antislop/`). SQLite stores their
hashes and locators. Entry versions retain byte spans on extended grapheme
boundaries, interpretation, qualifiers, hierarchy, dependencies, owners and
assignments. Source excerpts are resolved when read; document bodies are not
copied into version rows or agent handoffs. A source hash mismatch fails with
the underlying reason. Source lines are derived from the immutable byte span.

Criteria have stable IDs, mandatory flags and an integration flag. Composite
requirements need their own mandatory integration criterion. Technical plan
items need task assignments and, without a product requirement, a technical
goal and an execution check. They also need criterion evidence to become
verified. The analyst and its independent verifier check that a prescribed
implementation method has a corresponding requirement.

Requirement states are `identified`, `clarification_needed`, `planned`,
`in_progress`, `implemented`, `verified`, `blocked`, `stale`, and
`superseded_by_user`. Plan-item states are `pending`, `planned`, `in_progress`,
`executed`, `verified`, `blocked`, `stale`, `replaced_by_user`, and
`withdrawn_by_user`. Execution states are derived, never editable approvals.
No operation deletes an obligation or its history.

## Runtime contract

A candidate task plan triggers a requirements analyst using the existing
requirements executor profile. A separate requirements verifier compares both
ledgers with every full source. Document sources are supplied by reference;
user messages are supplied in full. Only then can the orchestrator produce an
executable plan with explicit `run_id`, `requirement_ids`, `criterion_ids`, and
`plan_item_ids`. Unresolved mandatory criteria, unknown references, missing
coverage, and ignored ledger task assignments block execution. Automatic
quality tasks inherit the obligations they inspect. Continuations can reuse
current approved work from earlier plans in the same run.

Every worker/verifier turn, including resumed and mailbox turns, receives
current ledger IDs, versions, criteria, task generation and source references.
An agent can add extracted obligations through its runtime-bound analyst turn.
Semantic changes to existing obligations become proposals for the user.
Sessions cannot assert their own verifier identity in JSON.

A semantic edit uses `expected_spec_revision` and an operation ID. The change,
new entry versions, user decision, affected task generations and ledger event
commit in one SQLite transaction. Comments and execution progress do not
advance the specification revision. Repeated proposals and applications reuse
the original operation result; a different payload cannot reuse its ID.

Invalidation follows ledger dependencies, parents/children, requirement links,
task inputs and transitive task dependencies. Affected tasks become `stale`
and receive individual cancellation signals. Unrelated processes continue.
Late reports cannot overwrite the current task generation. Scope publication
and Git integration share a mutex, and integration rechecks the inputs under
that mutex. In-place changes are retained for reassessment. A running plan
must finish stopping before a replacement starts; restart reaps the recorded
processes before recovery. Stale tasks are replanned rather than automatically
requeued. Supervised replacements wait for acceptance; autonomous replacements
still require source coverage and plan verification.

The compliance analyst and its independent verifier inspect the integrated
project and every source, including obligations missed during extraction.
Evidence identifies an entry version, criterion, task UUID, generation,
attempt, producing turn, verifying turn, concrete result, optional artifact
and integrated Git tree. Worktree approval, a completion assertion, old inputs,
same-session verification, and an old integrated tree cannot establish
completion. A record needs at least one mandatory criterion; optional criteria
alone cannot establish verification. All mandatory children and prerequisites
must also be verified.
A failed later audit closes the gate even if an earlier audit was positive.
The completion gate is closed if the integrated Git snapshot cannot be obtained;
exploratory work can still run, but cannot be announced as verified delivery.

`requirements_complete` opens only with current source coverage, all mandatory
criteria, accounted plan items, and an independently checked compliance audit.
Conversation summaries report `requirements_pending` until this holds. The
final runtime report names gaps and asks for executor work; verifiers stay
read-only. Historical plans remain `legacy`, without fabricated verification.
Resuming one requests source extraction and a replacement mapping; historical
reports are inputs, never proof of current completion.

## Daemon API

All operations are project/conversation/run scoped and reject unknown fields.

| Operation | Purpose |
| --- | --- |
| `CreateRun`, `ListRuns` | Create an explicit order or inspect run identities |
| `AddLedgerSource`, `GetLedgerSource` | Import an immutable correction/document or read a full source |
| `ListRequirements`, `ListPlanItems` | Query by optional revision, status, task and missing coverage; bounded pages |
| `GetLedgerEntry` | Entry, version history and evidence |
| `GetLedgerDiff` | Semantic differences between revisions and affected tasks |
| `GetEvidence` | Criterion evidence with provenance |
| `ProposeLedgerChange`, `GetLedgerProposal` | Prepare/retrieve a concrete change for review |
| `ApplyLedgerChange` | Apply the exact approved proposal and diff hash |

List operations take a `LedgerQuery` in `query`. An empty `run_id` selects the
latest run. Other operations require explicit run identity. Applying requires
the original proposal ID, diff hash and change payload. A changed revision or
changed impact requires another preview; the draft remains intact. Approval
hashes use immutable version identities and semantic changes, not document
contents. Updates publish IDs and versions rather than whole-ledger snapshots.

## TUI

Use `/requirements`, `/plan-items`, or their command-palette entries. Task
details have a Requirements tab linking the task's obligation IDs.

- `Tab`: switch ledger; arrows select records and detail tabs.
- `Enter`: load details; `s`: full source; `/`: search; `f`: status filter;
  `t`: task ID/UUID filter; `m`: gaps; `PgUp`/`PgDn`: page; `r`: refresh.
- `n`: create a record (or initialize a manual run); `N`: new manual run;
  `g`: cycle runs; `e`: edit; `p`: review the latest pending proposal.
- In the form, `Tab`/`Shift+Tab` selects fields, or click a field label.
  `Enter` inserts a line. Criteria use
  `ID | required/optional | integration/check | text`, one per line.
- `Ctrl+S` prepares a preview showing the exact change and affected tasks.
  `Enter` applies that preview; `Esc` returns without losing the draft.
  Supply a reason for edits, withdrawal or weakening. New records need a
  source/correction. Existing source text and record identity are immutable.
- `Ctrl+R` explicitly reloads the revision before preparing a new preview.
  Conflicts and transport failures retain text and cursor. Drafts, filters and
  cursors survive record/run changes and conversation switching with `Ctrl+O`.

The editor uses the shared grapheme-aware buffer, mouse cursor placement,
viewport wrapping, small-terminal guard, ASCII borders and color-free styles.
Execution and `verified` statuses cannot be set through the form.

## Verification

`tests/lean_ledger.rs` exercises immutable/long sources, middle-document negative
obligations, parents/integration, technical steps, author/session checks,
versioned evidence, concurrent edits, exact proposals, idempotency/restart,
transitive invalidation, real process cancellation and integrated audit sessions.
`tests/lean_ledger_tui.rs` covers Unicode/ZWJ editing, mouse selection, small
viewports, conflict drafts, exact previews and conversation preservation.
`tests/lean_protocol.rs` exercises the full daemon proposal/application/read
flow and scope rejection. Existing worker, plan, recovery and mailbox fixtures
implement the new analyst protocol without bypassing production gates.

The other design models, full risk catalog and general quality-gate system
remain outside this implementation.
