# Coordination and plan revision

An isolated worker or verifier asks the orchestrator about missing input or an
ambiguous decision through MCP. It does not inspect other sessions, contact peers
or invent another agent's obligations. Ordinary replies are local information;
they cannot change requirements, permissions or the selected implementation.

Use `needs_input` to save a checkpoint against a durable question. A reply can
resume the waiting owner. Login failures, CLI failures, missing inputs, review
findings and integration conflicts retain their actual reasons.

The orchestrator repairs contracts and dependencies using one operation:

```json
{
  "request_key": "revise-storage-inputs",
  "revision": {
    "plan": "plan-id",
    "base_revision": 0,
    "reason": "Provide the missing interface before implementation",
    "tasks": [],
    "retire": []
  }
}
```

`tasks` contains complete changed/new contracts; unchanged tasks are omitted.
Retiring a task requires preserving its obligations in the candidate graph.
The runtime validates the DAG, named input/output bindings, requirement coverage,
permissions, local options and quality gates. Independent plan review is required.
The accepted revision is applied atomically after affected agents stop.

Changed tasks and dependent results receive new input generations. Prior worktrees
remain preserved; fresh generations do not silently reuse stale files. Historical
source bytes that were never recorded remain unavailable. Request a fresh source
assessment instead of repeatedly trying to reconstruct them.

Corrections continue while a verifier confirms closure of a previous finding or
check. Stagnation is an orchestration problem, not automatic user approval.
Repeated submissions are identified by request keys. A daemon interruption keeps
the proposal and permits resubmission for review.

Force approval, extending a round quota and specialized add/split/clarify recovery
operations are not supported. Operational retry, cancellation and controlled
integration retain their separate safety checks. User consent to resolve an
integration conflict never replaces independent review.
