# Lean core

The first runtime treated agents as tool-free reasoners: it copied project
sources into run state and prompts, then proved the agent had read every byte.
A 15 MB repository produced a 112 MB run document that was rewritten on every
progress step, prompts over provider limits, and runs that blocked on the first
message. The lean core (`src/lean/`, its own `core.db`) replaced that model;
the old runtime, its context reader, recovery/proof machinery and requirement
audits were deleted once the lean core handled "przeanalizuj architekturę
projektu" end to end.

## Principles

- **Agents work on the files.** Every agent is a native CLI session (Codex,
  Antigravity, Claude) with its own read/search/shell tools. Nothing copies
  project sources into prompts, state or evidence. The only copy is a git
  worktree a writer works in.
- **The orchestrator names scope, not content.** It tells an agent which
  directories matter: this project, and related things such as services it talks
  to. Agents explore themselves.
- **Workers solve, verifiers judge.** A worker gets its goal and scope. Its
  paired verifier additionally gets only the risks for that worker's domain.
  There is no global risk catalog in any prompt.
- **State does not scale with the repository.** Small rows, updated one at a
  time, plus an append-only event log. No document hashing, no whole-state
  rewrites, no copies of state per request.
- **Nothing blocks without a reason the user can act on.** A failed turn fails
  that turn with its message; the user retries or rephrases. A daemon restart
  marks running turns interrupted; nothing else is held hostage.

## Agents

`lean::agent::run(spec, prompt, cancel, on_event)` starts one CLI turn and
streams normalized events: `Session(id)`, `Text(delta)`, `Tool{name, detail}`,
`Usage`, and finally `Done{text}` or a failure with the CLI's own message.

| | read-only (orchestrator, verifier, analysis) | write (worker) |
|---|---|---|
| cwd | project (or the worktree being reviewed) | task worktree |
| codex | `exec --json -s read-only -C <dir>` | `-s workspace-write` |
| agy | `-p --output-format stream-json --mode plan` | `--mode accept-edits --sandbox` |
| claude | `-p --output-format stream-json --permission-mode plan` | `acceptEdits` |

Extra scope directories go through `--add-dir`. Follow-up turns resume the
CLI's own session (`codex exec resume <id>`, `agy --conversation <id>`,
`claude --resume <id>`), so earlier context is never re-sent.

Read-only modes are not a confinement: agy's plan mode was observed running
`ls ..`. Writes are confined by the worktree and the CLI sandbox; reads are not.

## Flow

1. **Conversation.** The user's message goes to the orchestrator session
   (cwd = project). It answers, asks, or returns a plan as JSON:
   tasks with goal, scope directories, `writes` flag, dependencies, worker
   profile, and verifier focus + domain risks.
2. **Plan approval.** Supervised policy: the user accepts the plan (one key).
3. **Task.** The worker runs (worktree if `writes`, otherwise the project
   read-only). A writer's changes are committed in its worktree by the runtime.
4. **Verification.** The verifier gets the goal, the worker's report, the diff
   range (writers) and its risks; it reads the files itself and returns
   `{approved, findings}`. Findings resume the worker's session; the loop is
   bounded (default 3 rounds), then the task waits for the user.
5. **Integration.** An approved writer's work is squashed into one change
   and landed on the user's current branch, one integration at a time. With
   `local_commits` it becomes one commit (cherry-pick); otherwise it is left as
   ordinary unstaged changes. Without `local_write` it stays on its branch
   `antislop/<task>`. Integration never touches files the user is editing and
   never leaves conflict markers: such a task ends `needs_user` with its work
   kept on its branch.
6. **Summary.** The orchestrator receives task outcomes and answers the user.

## State

`core.db`, WAL, normal synchronous:

- `conversations(id, project, created_ms)`
- `messages(seq, conversation_id, author, text, created_ms)`
- `turns(id, conversation_id, role, provider, session_id, status, error, upto_message, started_ms, finished_ms)`
- `tasks(id, conversation_id, plan_id, position, spec, status, round, report, verdict, base, result, created_ms)`
- `events(seq, conversation_id, turn_id, kind, payload)` — append-only; tool
  calls are stored as a name and a one-line detail, never their output.

Writer worktrees live in `<state>/worktrees/<task>`, removed after a clean
integration. A real nine-task analysis of this repository (40 agent turns,
about 1,400 tool calls) records under 1 MB: messages, reports, verdicts and
one-line tool calls. Streamed answer fragments are not stored; the answer is
recorded whole when its turn ends.

## Daemon and frontends

`antislop` (or `antislop chat`) starts (or reuses) the project's daemon, which owns the `Core`
(`core.lock` makes it the only owner), so work goes on after a frontend exits.
One JSON request per connection: `identity`, `shutdown`, `conversations`,
`start`, `send`, `accept`, `cancel`, and `events{conversation, after, wait_ms}`,
a long poll returning at most 500 events / 1 MB. A daemon from another build is
replaced when idle. The socket is `<state>/core.sock`, or
`/tmp/antislop-<uid>/<hash>.sock` when that path is too long for Unix sockets.

- `antislop chat` in a terminal opens the TUI on the project's latest
  conversation (`--new` for a fresh one, `--conversation ID` for a given one).
  A poller thread long-polls events and every request runs on its own thread,
  so the draw loop never waits. The transcript shows messages and the
  orchestrator's tool calls; a side panel shows each task's status, round and
  its agent's current tool call. Keys: Enter send, Alt+Enter new line, Ctrl+A
  run the proposed plan, Ctrl+X cancel, PgUp/PgDn scroll, Ctrl+N new
  conversation, Esc detach.
- `antislop chat "message"` (or piped input) prints events line by line and
  exits once the conversation is idle; `/accept` and `/cancel` lines work too.

## Guarantees

- `accept` names the plan the user saw (`plan_id`); the scheduler and the
  report to the orchestrator stay pinned to it even if another plan is
  proposed meanwhile. Queuing a plan is one transaction.
- A failed or skipped task skips everything that depends on it, transitively.
  A failed task's report keeps the agent's error.
- A verdict that approves while listing high or medium findings is a
  rejection. A plan where a task depends on itself or a later task is
  rejected.
- A turn succeeds only with a complete result *and* a clean exit. When a turn
  ends or is cancelled its whole process group is killed, so no descendant
  outlives it or holds the turn open. Only the tail of stderr is kept.
- On start, turns left running become `interrupted` and unfinished tasks
  become `cancelled` with the reason; nothing waits for a reconciliation.
- At most four tasks of a plan run at once.
- A session is resumed only by the CLI that created it.

## Known limits

- A writer that depends on exactly one writer starts from that writer's
  result; one that depends on several starts from `HEAD`, so without
  `local_commits` it does not see their uncommitted results.
- A plan runs in waves: a task waits for every task of the previous wave.
- `needs_user` and cancelled tasks keep their branch and worktree, but there is
  no command yet to resume one task or retry its integration.
- Read-only modes are prompts and CLI modes, not OS confinement; `scope`
  guides agents but does not limit what they read.
- Prompts are passed as arguments (the orchestrator's plan report is bounded
  by 40k characters per task).
- `core.db` has no schema versioning yet.

## Stages

1. ✅ Store, agent adapters (codex, agy, claude), conversation with the
   orchestrator, `antislop chat` CLI. Accepted on a real run: the orchestrator
   answered "przeanalizuj architekturę projektu" by reading the project itself;
   a second message sent while the first ran was accepted at once and answered
   in the resumed session.
2. ✅ Plans and read-only worker/verifier pairs with domain risks.
3. ✅ Writers in worktrees, commit, verifier on the commit range, integration
   (`tests/lean_writer.rs`).
4. ✅ Daemon protocol (`events` long poll) and the TUI on top of it
   (`tests/lean_core.rs`: two CLI processes share one daemon and session).
   A real nine-task analysis of this repository ran end to end (about 34
   minutes, 40 agent turns); its findings about the lean core itself are the
   regressions in `tests/lean_recovery.rs` and the guarantees above.
5. ✅ The old runtime, TUI, context reader, recovery/proof machinery and the
   requirement audits were deleted; `antislop` with no verb opens the chat.
