# Runtime reliability

Workers execute validation and make changes in their allocated worktrees.
Verifiers inspect files, diffs, assertions and recorded worker evidence. They
must not run builds, tests, formatters, linters or benchmarks, or edit files.
Validation commands in a task's goal or focus belong to its worker. Missing
worker evidence is actionable feedback; inability to independently execute a
command is not a reason to reject a result.

Codex writers start and resume with `sandbox_mode="danger-full-access"` and
`approval_policy="never"`. Read-only roles retain `sandbox_mode="read-only"`.
This is set per invocation, without changing the user's global Codex settings.

## Pending work

A failed prerequisite leaves downstream tasks queued. Runtime recovery cannot
discard a task, and the orchestrator can only retry it with guidance or extend
its rounds. Legacy skipped and cancelled rows remain readable and are restored
when their plan is resumed. Resuming also repairs legacy approvals of tasks
that never started. Restarting the daemon resumes previously accepted, unfinished
plans in both supervised and autonomous modes; explicit pauses remain effective.

A verification override requires a completed worker turn and, for a writer,
a real result commit. A writer must integrate successfully before it can be
approved. Keeping a branch preserves the work but leaves its task and dependents
pending. Applying only part of a result also leaves the task pending. Recovery
events record whether their actor was the user or the orchestrator.

## Worktree inputs and integration

A worker starts with a Git snapshot of tracked and untracked local changes,
excluding ignored files. All transitive dependency results are composed in
dependency order. Snapshotting is serialized with Antislop's integrations; it
does not change the primary branch, files or index.

If composition encounters a real conflict, the writer can still start in its
own worktree. Durable runtime guidance identifies the original Git error and
the exact upstream commit ranges to combine before completing the original goal.
The repair's inherited paths and goal are explicitly recorded in the task.

Integration preserves edits made in the primary tree. Pre-existing user edits
in a path the worker changes remain protected; the worker can read and build
on them, but their presence does not silently grant overwrite consent. Snapshot
integration restores only the worker's changed paths and respects commit policy.
With `policy.local_commits=true`, successfully integrated results are committed;
with it disabled, results remain applied in the working tree.

Successful integration removes the task worktree and branch. Composition
removes its temporary worktree and patch on both success and failure. Unfinished
or unintegrated work is retained. Existing CLI cancellation, timeout and process
group cleanup continue to apply.

## Task mail

Messages to a known task are persisted regardless of its progress. Unknown
recipients produce one rejection. Workers and verifiers of stopped tasks can
take read-only turns to acknowledge mail without changing task status. Pending
mail is scanned again after a daemon restart. Queued tasks receive mail in their
next task turn; completed and blocked tasks can answer in the background.

A receipt is recorded only after a successful recipient turn. Failed delivery
leaves the original message queued and reports the raw error. A retry can repeat
an interrupted delivery, so receipt semantics are at least once, not exactly once.

## Validation

Regression coverage includes two converging dependency chains with local edits,
real composition conflicts handed to a runnable worktree, temporary resource
cleanup, all stopped task mailbox states, verifier mail after restart, pending
dependencies, supervised restart, rejection of empty approvals and task
cancellation, integration safety, and Codex permissions on start and resume.

These changes do not migrate a daemon already running in memory. Let its active
work finish before replacing it. Preserve its result branches and use the normal
resume operation to continue pending obligations with the updated runtime.
