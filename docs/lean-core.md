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
revises it afterwards. Both submit tasks with `id`, `title`, `goal` and `kind`,
and optionally `depends_on`, `scope`, `writes`, `covers`, `section` and
`summary`. The runtime derives everything else: `writes` from the kind unless
given, requirement and criterion links from `covers` (criterion ids are globally
unique), options tasks and quality gates, and the verifier's checks. A task's
checks are its goal, the acceptance criteria it covers and the whole risk
catalog of its kind; the planner neither selects risks nor adds its own. Plan
review checks coverage of the request.

Dependencies express ordering. When a dependency is approved, each direct
dependent learns that it is unblocked and receives the dependency's title and
short `summary`, never its full report or round history. An options task's
selection reaches its writer. An analytical task is judged on the correctness of
its report, including defects it correctly discovers, rather than on fixing the
code it studies.

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

Every result reaches the runtime through a tool that ends the turn, checked
while the turn is still open: a refused call names what to fix and the agent
calls again. There are no fenced result blocks and no repair turns. Each role
sees only its own tools:

- task workers: `complete(summary, report)`, or `select(alternatives, selected,
  rationale)` for an options task; `ask(question)`; `wait_for_answer(note?)`;
- task verifiers: `review(report, checks, findings)`, `ask`, `wait_for_answer`;
- the planner: `submit_plan(document, tasks)`;
- the orchestrator: `finish_turn(text, addressed?)`, `ask_user(context,
  question, options)`, `submit_requirements(markdown)` during the interview,
  `submit_plan`, `answer_agent(request, answer)`, `defer_to_user(request,
  explanation)`, `revise_plan(reason, tasks, retire)` and
  `request_replan(reason)`;
- plan, audit and code reviewers: `review(report, findings)`;
- the requirements analyst: `submit_ledger(entries)`; the requirements auditor:
  `submit_audit(findings, evidence)`; its verifier:
  `submit_audit_review(note, assessments)`;
- any other role: `finish_turn(text)`.

`request_replan` records a durable request; the next turn is the planner's,
which redrafts the whole plan with the current contracts and the reason, and the
result is reviewed like any new plan. Contract and dependency repairs stay with
`revise_plan`. Tools are authenticated, scoped to the session and checked
against the active turn. An agent cannot choose another role or address another
task. There are no peer messages, agent broadcasts or receipt-only model turns.

A CLI that dies or never connects before it took its prompt lost nothing, so the
runtime starts it again up to twice (a login or limit error is not retried, it
waits for the user). A failure after the prompt was typed is never retried
automatically.

Answers to decisions the runtime asked during execution steer the running work;
they are not new requirements sources and do not re-extract the ledger.

Questions and replies are durable. Asking the same question again in a turn
returns its original request and state. `ask` waits at most 25 seconds for the
answer and does not consume the agent's active-work timeout.
The orchestrator can answer while all task slots are occupied. No model wait owns
a database, integration or scheduler lock.

For a longer interruption, a worker or verifier calls `wait_for_answer`, which
parks the task on the turn's open question. The orchestrator answers with
`answer_agent` or hands the decision to the user with `defer_to_user`; an actual
answer later resumes only that owner. This is waiting, not completion. Cancellation,
CLI death, daemon interruption and input invalidation remain explicit failures or
interruptions. Raw tool errors are recorded without copying tool arguments or
credentials into logs.

## Typed results

Agents give content and judgement; the runtime adds identities, versions,
links and verdicts:

- `ask_user` questions are named by the runtime from their content;
- a `review` is approved when every check is ok or n/a and no finding is
  critical, high or medium; each finding gets an id stable across rounds from
  its check and location;
- `addressed` lists the comments an orchestrator's answer settles;
- `submit_ledger` takes a quote of the user's words instead of a byte range:
  the runtime finds it in the named source or the first that has it (it must
  occur there once, on grapheme boundaries) and fills in the run, revision,
  position and disposition;
- `submit_audit` evidence names a criterion and a task; the runtime binds the
  entry and its version, the task's input generation and round, the producing
  turn and the integrated fingerprint. The audit approves unless a finding
  blocks. Its verifier assesses each proposal by the id the runtime gave it
  (`E1`, `E2`, …) and must assess all of them.

Options compares 2–4 alternatives, each with `id`, `approach` and `tradeoffs`,
and names the `selected` one with a `rationale`.

Task review covers every listed check with a status and a note. Findings name
their `check` plus `severity`, `issue` and `where`. An unrelated suggestion
belongs in the report, not a blocking finding.

The runtime persists a result before acknowledgement. Identical repeated
submissions return the receipt. Conflicting submissions and obsolete turn IDs
are rejected. Text in a report does not select a variant, create a blocker,
change task state or override a review.

## Revisions and progress

The orchestrator uses `revise_plan(reason, tasks, retire)` on the plan it was
shown; the runtime supplies the plan, its base revision and an idempotency key
from the request's content. Omitted contracts remain. Retiring a task never satisfies its obligations.
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
user request. A requirements analysis that ended without `submit_ledger` is
resumed with a separate bounded retry guard.

A revision review that has rejected twice in a row no longer blocks on findings
that are only medium or low; they travel with the plan as a notice. A high or
critical finding always blocks, and a review turn that ended without `review`
fails with that reason.
`policy.max_tasks` (default 24)
bounds the tasks a plan is drafted with, and `policy.local_options = false`
leaves out the per-writer options tasks for mechanical work.

Agents work on their allocated checkout in its current state. Revision identifiers,
adopted snapshots and changed-file lists from earlier reports are context, never
execution or acceptance gates, even when scoped files differ. Workers perform the
assigned work and measurements on the files they receive; verifiers assess concrete
defects and evidence rather than repository identity. Do not reset or switch a
checkout to satisfy a historical snapshot requirement.

Every task of a plan starts from one baseline commit, fixed when the plan's
first task starts. Contracts never name commits or checkout paths; edits the
user makes afterwards stay protected when results are integrated.

The first rejection establishes a correction baseline. Later rejections must
close an earlier finding or check. Rounds, reworded reports and arbitrary commits
are not progress. As a safety valve, one run of a task takes at most 16 rounds;
then it returns to the orchestrator as `coordinating` (never as failed), and
running it again starts a new count. Stagnation returns to the orchestrator for new input, a reviewed
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
