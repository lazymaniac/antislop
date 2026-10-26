# Recovering blocked tasks

The runtime's blocked-plan message lists available recovery actions and the
reason for each unavailable action. Proposed task specs, dependency cycles,
permissions and live state are still validated when an action is applied.

`add_task` inserts a read-only prerequisite. There is no lifetime quota on
additions or fixed recovery-chain depth; the chain must remain acyclic, and
only one active prerequisite may clear a given blocker at a time.
`split_task` cannot split an unblocker, a split part or a writer with partial
changes. When approved dependency evidence already supplies the missing input,
use `retry_with_guidance` rather than creating another discovery task.
Ordinary retries in supervised mode require the user's decision.

Recovery uses progress rather than two-attempt or three-retry budgets:

- Proposals are recorded before execution or review in append-only events.
  Repeating the same proposal with unchanged evidence and outcome stops, even
  after a daemon restart or a cancelled/failed review.
- Whitespace, renamed prerequisite IDs/titles, new CLI session UUIDs and round
  counters do not count as progress. `extend_rounds` and `retry_with_guidance`
  share history, so alternating their names does not bypass this check.
- Changed approaches receive independent progress review against previous
  attempts and findings. Task-contract clarification performs that assessment
  in its existing independent contract review. Actionable corrections may
  continue without a numerical quota.
- Changes to accepted inputs, dependency results, scoped Git changes (including
  edits to already dirty files), or a new explicit user message establish a
  new observation. A different observation is evidence to assess, not automatic
  approval of a repeated approach. For unversioned directories or external
  directories, the runtime does not claim to capture all their source bytes;
  the reviewer must inspect the specific new evidence.
- Rewording the same rejected approach or a requirement that needs a user
  decision stops automatic continuation. A clarification reviewer identifies
  those findings with `code: recovery_no_progress` or `recovery_user_decision`.
  Ordinary actionable rejection findings are returned to the orchestrator for
  a corrected proposal.

This policy concerns orchestrator recovery. Per-turn timeouts, cancellation,
worker-round policy, scope checks, permission checks and independent verification
remain in force. There is no new configurable time/cost limit in this change.

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
A correction's review records its proposal and findings in durable history.
Rejected reviews with actionable findings return them to the orchestrator as a
runtime message. A reviewer finding no progress or a necessary user decision
ends automatic continuation and leaves the task contract unchanged.

Historical notes and architecture recommendations do not add requirements.
Make instrumentation and other implementation choices conditional on their
necessity for an accepted criterion; propose additional scope separately.

Historical source identities that were never captured cannot be reconstructed
by certifying the current tree. Record that limitation. Where the accepted
requirements allow it, regenerate claims against identified current sources
and obtain a new independent assessment. Distinguish dirty working files from
committed blobs. A read-only task has no writer base/result commits, and an
input-producing task must not wait for its downstream consumer's assessment.
Clarification cannot lift a permission blocker or waive a requirement that
actually requires historical evidence.
