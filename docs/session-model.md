# Session boundaries

A provider session belongs to one role and, for task agents, one task attempt.
Provider histories are never shared between workers, verifiers or the orchestrator.
A changed execution profile rotates the session. Each new attempt is reconstructed
from its local contract, named inputs, prior local report/checkpoint and open
findings. The whole conversation and complete dependency reports are excluded.

The runtime owns the turn identity, task input generation, permissions and source
identity. MCP tool calls must match that active lease. A task does not authenticate
historical sources or infer authorization from another agent's report.

The planner drafts the first plan of every planning cycle (each interview that
ends in planning) in its own session and execution profile (`routing.planning`);
after that draft, comments, revisions and agent questions belong to the
orchestrator, which can hand the whole plan back with `request_replan`. The planner's words reach the user
as the orchestrator's.

Task agents see only question, answer-wait and completion tools. The orchestrator
sees domain risk lookup, response, revision and completion tools. Agent-to-agent and receipt-only
sessions do not exist. A worker's question does not occupy the orchestrator's
execution slot. Tool waits do not hold runtime locks.

Questions, answers and final submissions are durable. Final results are committed
before acknowledgement and support identical retries. A checkpoint can wait for
an answer without claiming success. Daemon restart interrupts in-flight turns;
resumption retains local work, questions and accepted data without granting review
or implementation approval.

Task code runs in an allocated worktree. Verification reads an immutable result
checkout. New input generations use distinct worktrees and branches; previous
partial work remains available through its preserved Git identities. The primary
checkout is modified only through controlled integration.

See [the execution model](lean-core.md) for contracts, result schemas, authorization,
quality gates and plan revisions, and [coordination](task-recovery.md) for recovery.

A `needs_input` checkpoint references the existing question ID returned by
`ask_orchestrator` in `request`; the question text belongs in `question` when
asking, and progress belongs in `checkpoint`. Pending and `awaiting_user`
questions park the task. Answered questions resume it, including when the reply
arrives before the checkpoint. A proposed plan revision is not an applied one.

A rejected `finish_turn` is not a completion receipt. The CLI must correct the
reported error and obtain `recorded=true`. The gateway allows 60 seconds from
the first rejected submission for correction, then fails the turn with the last
rejection reason. Stale-turn calls do not affect the current turn. A CLI exit or
the normal turn timeout also retains the last rejection reason.

During execution the orchestrator can request a user decision by finishing with
`type=report` whose text contains a fenced `question` JSON block with `id`,
`context`, `question`, and 2–6 `options` (`label`, optional `description`). The
question's context names the exact proposed action and why a decision is needed.
This records a durable `user_decision_requested` event and opens the question
form (`/question`, also `/interview`). Repeating an identical id and question is
idempotent; a different question needs a new id. Answers are user messages, not
plan approval or changes to task permissions. An answered worker question stays
answered; it cannot be reopened to represent a separate user decision.

Every task turn, including resumed sessions, receives its current turn, role,
working directory and session policy from runtime allocation metadata. Verifier
prompts use the verifier's allocated directory, including manual snapshot review.
Paths in previous reports describe historical allocations; this metadata does
not establish content equivalence or preserve uncommitted state across checkouts.
