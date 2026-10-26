# Recovering blocked tasks

The runtime's blocked-plan message lists available recovery actions and the
reason for each unavailable action. Proposed task specs, dependency cycles,
permissions and live state are still validated when an action is applied.

`add_task` inserts a read-only prerequisite. Recovery allows two levels of
prerequisites and at most two additions per blocked task, including successful
additions. `split_task` cannot split an unblocker, a split part or a writer with
partial changes. When approved dependency evidence already supplies the missing
input, use `retry_with_guidance` rather than creating another discovery task.
Ordinary retries in supervised mode require the user's decision.

`clarify_task` corrects a blocked read-only task's evidence contract. Put it in a
`recover` action with `task`, a nonempty evidence-based `message`, and a full
replacement `spec`. Copy the current spec exactly, including runtime-owned
`resolves` and `part_of` fields when present; change only `goal` and
`verifier.focus`. Requirement and criterion identities, dependencies, scope,
access and verifier risks cannot change through this action.

An independent plan verifier must approve that all substantive accepted
obligations remain. A rejected, cancelled or stale review leaves the task
unchanged. An accepted clarification queues fresh worker and verifier turns;
it never approves the task's result. The runtime preserves old rounds and
reports, records both contracts and the review in a `task_clarified` event,
and advances the input generation to reject late results from old turns.
The two-clarification limit persists across daemon restarts.

Historical source identities that were never captured cannot be reconstructed
by certifying the current tree. Record that limitation. Where the accepted
requirements allow it, regenerate claims against identified current sources
and obtain a new independent assessment. Distinguish dirty working files from
committed blobs. A read-only task has no writer base/result commits, and an
input-producing task must not wait for its downstream consumer's assessment.
Clarification cannot lift a permission blocker or waive a requirement that
actually requires historical evidence.
