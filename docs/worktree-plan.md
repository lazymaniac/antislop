# Per-task worktrees and integration

Workspace mode `task-worktrees` (now the intake bootstrap default) replaces the run-wide inlined input snapshot for team execution with a private Git object store per run and one checkout per writer task. It addresses two observed failures: a 28 MB verifier prompt (sources inlined twice) and global invalidation of every verdict whenever any task, or the user, changed a source file.

## Model

- **Base.** On the first wave the runtime captures the project's current files (tracked, untracked, not ignored; `.antislop`/`.architect` excluded) as commit `B` in `<data_dir>/task-views/<run>/repo.git`. The user's repository, index and branch are never touched. `head` starts at `B` and advances as verified results are integrated.
- **Views.** A task works on `combine(head, results of its dependencies)`: ancestors collapse, otherwise `git merge-tree --write-tree` builds a merge commit. Read-only tasks (no write scopes) share a detached checkout per commit under `views/<sha>`. Writer tasks get a branch `task/<slug>` in `tasks/<slug>`, starting from the view, or from the previous attempt's result on repair.
- **Execution.** Edits remain runtime-mediated whole-file proposals (decision 2). The runtime validates them against the task's editable scope, writes them into the writer's checkout, commits, pins `refs/antislop/...` and records an execution binding whose receipts carry the pre-edit hashes from the start commit. No `input_snapshot` comparison is used in this mode; staleness flows only through dependencies.
- **Prompts.** Sources are a manifest of the view (path, size, hash) with editable files and then the smallest files inlined up to 1.2 MB; verifiers additionally get the diff from view to result (at most 400 KB). Effects and receipts are summarized by hash instead of repeating the snapshot.
- **Checks.** Task checks run in the task's checkout. A check that changes files fails the task, and the checkout is reset.

## Integration

After a wave, verified writer results whose dependencies are already integrated land serially under one lock:

1. Per file, 3-way merge: base = the task's view, ours = the user's current project file, theirs = the result (`git merge-file --diff3`; delete-vs-change is a conflict).
2. **Conflict:** nothing in the project changes. The runtime records the project state as a commit, prepares a new start commit containing the conflict markers, and returns the task to its executor as an `INTEGRATION:<path>` repair. The repaired result is verified again before integration retries.
3. **Clean:** each file lands with compare-and-swap (the bytes read for the merge must still be present; temp file + rename). If the project is a Git repository, paths that were clean before landing are committed as one commit on the current branch (`antislop: <summary>`, with task, run and verifier session in the body; hooks disabled). Paths the user had uncommitted edits in receive the merged content but stay uncommitted and are reported in `Integration.uncommitted`.

## Status

Implemented: `src/workspace/views.rs` (Git store, captures, merges, CAS landing), `src/runtime/team/views.rs` (view preparation, mediated output, prompt sources, integration), the authentication path for view bindings, check routing, restart restoration of task checkouts, and the integration-conflict repair contract in `src/team.rs`. Tests: `tests/task_views.rs` (store primitives) and `tests/task_worktrees.rs` (landing as a commit, conflict repair, uncommitted user edits).

Known limitations:

- Orchestrator rounds after the team is installed still see the run's base snapshot rather than integrated results.
- The full base `runtime:input-snapshot` evidence is still stored once per wave.
- An `Intent` binding left by a crash stays `Intent`; reconciliation resets the checkout to its start commit.
- Integration happens immediately after verification, also in supervised mode, before stage acceptance.
- Checks that legitimately rewrite tracked files (formatters, generated lockfiles) fail in a private checkout.
- Agents still receive inlined sources; letting providers read the checkout themselves (read-only tools with the checkout as cwd) is a follow-up per provider.
- Verifiers do not separately review a conflict resolution; the repair is verified as a full new attempt.
- Checks are not rerun on the merged project content after a clean landing.
