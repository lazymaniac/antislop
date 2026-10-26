# Session boundaries

A provider session belongs to one role and, for task agents, one task attempt.
Provider histories are never shared between workers, verifiers or the orchestrator.
A changed execution profile rotates the session. Each new attempt is reconstructed
from its local contract, named inputs, prior local report/checkpoint and open
findings. The whole conversation and complete dependency reports are excluded.

The runtime owns the turn identity, task input generation, permissions and source
identity. MCP tool calls must match that active lease. A task does not authenticate
historical sources or infer authorization from another agent's report.

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
