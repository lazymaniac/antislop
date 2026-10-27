# antislop execution model

antislop decomposes an accepted request into small, independently verified tasks.
The orchestrator coordinates the request. A worker knows one local contract; its
verifier checks that contract and its result. Neither receives the conversation
history or the complete plan.

## Planning and authorization

The interview establishes the goal, constraints, exclusions and observable
acceptance criteria. Requirements remain versioned in the ledger. Independent
requirements review and plan review precede implementation.

The planner drafts the first plan from the agreed requirements; the orchestrator
revises it afterwards. Both propose `TaskContract` records: a goal, paths, requirement and
criterion links, named inputs and outputs, dependencies, work kind and verifier
focus. `verifier.risk_ids` selects relevant entries from the domain risk catalog.
The planner and the orchestrator read each needed catalog through `task_risks(kind)` before
selecting IDs; workers receive only the selected subset. Additional local risks may be stated in `verifier.risks`. Plan review checks both
coverage of the request and the selected risk subsets.

Dependencies express ordering. They do not forward reports. An input binding
`{name, task, output}` selects one declared output of a direct dependency. A
verified producer must actually have submitted that output before consumption.
An analytical task is judged on the correctness of its report, including defects
it correctly discovers, rather than on fixing the code it studies.

The runtime records plan authorization separately from task results, preserving
read scope and write scope independently. A revision cannot add write permission
or expand the authorized paths. Supervised
execution requires acceptance; autonomous execution uses the configured plan
acceptance policy. Replies from an agent cannot grant permissions. Review cannot
turn an unperformed task into completed work.

## Local options and quality gates

Every writer receives a dedicated read-only options task, including writers added
by a revision and generated gates that may change code. Shared architecture work
is input, not a replacement for local options.
The writer objective is supplied as JSON reference data, separately from options
instructions. Options inspects source and existing evidence; it neither edits files
nor executes validation commands. The dependent writer implements and validates
only after the selection is approved.

Options compares 2–4 realistic alternatives using common criteria, filters out
ineligible variants and selects a variant with rationale and a condition for
reconsideration. Its independent verifier judges the comparison, not the future
implementation. The writer receives only the selected alternative and rationale.
A changed premise is raised with the orchestrator; changing the selected approach
requires renewed options and review.

Required refactoring, static analysis, testing, security, performance and
integration gates remain enabled according to policy. Their contracts describe
the combined change they assess. Their verifiers assess those gate results; they
do not reconstruct the entire session. Global requirements coverage belongs to
the orchestrator and explicit integration/compliance work.

## Worktrees and evidence

Tasks execute in allocated Git worktrees. Writers never work in the primary
checkout. Read-only tasks receive a stable observation checkout when Git is
available. Each input generation has a distinct writer worktree and branch.
Previous partial work remains preserved when a contract is revised.

After a worker completes, the runtime captures its result commit. Its verifier
reads a separate checkout of that commit with read-only access. Runtime metadata
binds the turn to the task, role, input generation and observed source commit.
Base/result identify writer changes; a read-only task need not fabricate writer
commits. A new observation does not authenticate missing historical evidence.

Builds, tests, formatting and benchmarks belong to workers with write-capable
isolated environments. Verifiers inspect files, diffs and recorded evidence.
They do not execute those checks. Integration preserves user edits and refuses
unverified results, including when the user authorizes resolution of a conflict.

## Communication during work

Each agent is an interactive CLI in a daemon-owned PTY. MCP is the only structured
channel back to the runtime; the terminal is for interaction and raw diagnostics,
not an alternate result parser. Configuration is supplied per launch.

Task workers and task verifiers see:

- `ask_orchestrator(turn, request_key, question)`;
- `await_orchestrator(turn, request)`;
- `finish_turn(turn, result)`.

The orchestrator sees `task_risks`, `reply_to_agent`, `submit_plan_revision`,
`request_replan(turn, reason)` and `finish_turn`. `request_replan` records a
durable request; the next turn is the planner's, which redrafts the whole plan
with the current contracts and the reason, and the result is reviewed like any
new plan. Contract and dependency repairs stay with `submit_plan_revision`. The planner sees `task_risks` and `finish_turn`; it neither answers
agents nor revises a plan.
Other workflow roles finish with a report. Tools are authenticated, scoped to the
session and checked against the active turn. An agent cannot choose another role
or address another task. There are no peer messages, agent broadcasts or
receipt-only model turns.

Questions and replies are durable. Repeating a request key with the same question
returns its original identity; different content conflicts. Answer waits last at
most 25 seconds per tool call and do not consume the agent's active-work timeout.
The orchestrator can answer while all task slots are occupied. No model wait owns
a database, integration or scheduler lock.

For a longer interruption, a worker or verifier saves `needs_input` with the
question identity and a checkpoint. `reply_to_agent` may return
`state: awaiting_user` with the unresolved decision; an actual answer later
resumes only that owner. This is waiting, not completion. Cancellation,
CLI death, daemon interruption and input invalidation remain explicit failures or
interruptions. Raw tool errors are recorded without copying tool arguments or
credentials into logs.

## Typed results

`finish_turn` accepts a role-specific result:

- `completed`: `summary`, `report`, declared `outputs`, optional `selection`;
- `review`: `report` and `verdict` with local check coverage and findings;
- `needs_input`: `request` and `checkpoint`;
- `report`: `text`, for orchestration and other workflow roles.

Options requires a structured selection. Its alternatives have `id`, `approach`,
per-criterion `evaluations`, `eligible` and `tradeoffs`; the selection records
`criteria`, `selected`, `rationale` and `reconsider_when`.

Task review covers every assigned criterion, common/domain check and selected
risk with evidence or justified non-applicability. Findings use stable `id` and
`check` references plus `severity`, `issue` and `where`. An unrelated suggestion
belongs in the report, not a blocking finding. Invalid results are rejected at
the tool boundary for correction in the current turn.

The runtime persists a result before acknowledgement. Identical repeated
submissions return the receipt. Conflicting submissions and obsolete turn IDs
are rejected. Text in a report does not select a variant, create a blocker,
change task state or override a review.

## Revisions and progress

The orchestrator uses `submit_plan_revision(turn, request_key, revision)`.
A revision contains `plan`, `base_revision`, `reason`, changed/new `tasks` and
`retire`. Omitted contracts remain. Retiring a task never satisfies its obligations.
The proposed full graph must preserve all mandatory requirements.

Normalization restores dedicated options and required gates. Independent plan
review checks the candidate. A scheduler barrier stops affected tasks before
atomic publication. The plan version is compared again on application. Affected
input generations are incremented; old replies/results cannot revive them.
Unchanged independent tasks continue. Previous contracts and partial work are
retained. Input invalidation cancels obsolete executions and starts fresh checkouts
without reusing partial commits from the previous inputs. Retired records remain
in history and cannot be scheduled or reactivated by a direct message. Explicit
reviewed reintroduction creates a new execution identity. Revisions within authorized scope require review, not another user
approval; expanded scope or permissions require a user decision.

Revision submissions have durable receipts. Preparation or thread-launch failures
close the receipt with the raw cause and release the revision slot immediately.
A failure to resume execution after an applied revision is reported separately;
it does not turn an applied revision into a rejected one. Interrupted review can be resubmitted
with the same request key. There are no separate add/split/clarify recovery paths
and no force approval or extension of a correction-round quota.

Plan reviews also use stable finding IDs and verified closure rather than a fixed
number of attempts. Their baseline survives daemon restart and resets for a new
user request. Format and schema repair retain a separate bounded retry guard.

A revision review that has rejected twice in a row no longer blocks on findings
that are only medium or low; they travel with the plan as a notice. A high or
critical finding, or a rejection without a structured verdict, always blocks.
`policy.max_tasks` (default 24)
bounds the tasks a plan is drafted with, and `policy.local_options = false`
leaves out the per-writer options tasks for mechanical work.

Every task of a plan starts from one baseline commit, fixed when the plan's
first task starts. Contracts never name commits or checkout paths; edits the
user makes afterwards stay protected when results are integrated.

The first rejection establishes a correction baseline. Later rejections must
close an earlier finding or check. Rounds, reworded reports and arbitrary commits
are not progress. Stagnation returns to the orchestrator for new input, a reviewed
contract revision or a specific user decision.

## Storage, UI and alpha format changes

SQLite stores compact task, requirement, question, result and revision records
with append-only events. Repository trees and file contents are not serialized
into task state. Reports are not CLI transcript mirrors.

The TUI shows execution, checks, waiting reasons and integration results while
preserving drafts and modal isolation. The primary checkout changes only through
controlled runtime integration. Trust is checked before project state is created.

This format uses database schema 14, configuration schema 2 and daemon protocol 3.
There are no compatibility adapters or migrations. Stop the old daemon and client,
manually archive the project's `.antislop/` (including worktrees and Git references
needed to preserve unfinished work), then start the new version with updated
configuration. Do not delete working data to resolve a schema error.
