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
   profile, and verifier focus + domain risks. Its instructions sketch team
   shapes (change, refactoring, security fix, new project, research,
   document, analysis); small requests get few, short tasks.
2. **Plan review.** Before the user sees a plan, an independent plan
   verifier (the orchestration route's verifier profile, its own session,
   cwd = project, read-only) checks it against the user's messages: coverage,
   missing or misordered dependencies, assumed ordering or unknowns,
   checkable goals, scope, `writes`, verifier focus. A plan it sends back goes
   to the orchestrator with the findings (shown in the transcript as the plan
   verifier's message) and is marked `revised`; after three returns since the
   user's last message the plan is shown with the open findings. A review
   that fails to run shows the plan with the error.
3. **Plan approval.** `supervised`: the user accepts the plan (one key).
   `autonomous`: a plan the verifier approved starts at once; one it did not
   approve, or a fourth plan in a row without a user message, waits for the
   user. Deviation from design §6.2: correction rounds inside a task run
   without asking in both modes; asking at every round is what made the first
   runtime unusable.
4. **Task.** The worker runs (worktree if `writes`, otherwise the project
   read-only). A writer's changes are committed in its worktree by the runtime.
5. **Verification.** The verifier gets the goal, the worker's report, the diff
   range (writers), its risks and a short checklist for the task's kind
   (`tasks::checklist`: e.g. assertions that fail on the bug for testing,
   suppressed warnings for static analysis, recomputation for analysis,
   sources that support claims for research). It reads the files itself and
   returns `{approved, findings}`. Findings resume the worker's session; the
   loop is bounded (3 rounds), then the task waits for the user.
6. **Integration.** An approved writer's work is squashed into one change
   and landed on the user's current branch, one integration at a time. With
   `local_commits` it becomes one commit (cherry-pick); otherwise it is left as
   ordinary unstaged changes. Without `local_write` it stays on its branch
   `antislop/<task>`. Integration never touches files the user is editing and
   never leaves conflict markers: such a task ends `needs_user` with its work
   kept on its branch.
7. **Audit.** When a plan ends (not cancelled), an independent auditor (own
   session, read-only) lists every obligation of the user's messages and
   checks the results and files against them. Its verdict goes into the
   results the orchestrator receives; gaps ask it to propose a plan for what
   is missing.
8. **Summary.** The orchestrator receives task outcomes and the audit and
   answers the user (or proposes the next plan).

## Messages between agents

An agent addresses another with a ```` ```message ```` block in its answer:
`{"to": "t2" | "orchestrator" | "all", "kind": "finding|handoff|question|answer|decision|status", "text": "..."}`.
The runtime takes the blocks out of the answer, records each message (sender,
recipient, kind, text) in `mail` before delivery, and delivers it at the top
of the recipient's next prompt; it is marked delivered when that turn
completes (a failed turn delivers it again). `all` means the workers of every
unfinished task of the plan except the sender's. A worker that receives
messages while it works gets up to two extra turns for them before its
verifier runs. An unknown recipient is reported as `mail_rejected`.

The user writes `@t2 text` to reach a task's worker the same way; the
orchestrator gets a copy in its next turn, without a turn of its own. The
TUI shows each message as waiting for the recipient's next turn and then as
received: the CLIs offer no delivery into a running turn, so none is
pretended.

## State

`core.db`, WAL, normal synchronous:

- `conversations(id, project, created_ms)`
- `messages(seq, conversation_id, author, text, created_ms)`
- `turns(id, conversation_id, role, provider, session_id, status, error, upto_message, started_ms, finished_ms)`
- `tasks(id, conversation_id, plan_id, position, spec, status, round, report, verdict, base, result, created_ms)`
- `mail(id, conversation_id, sender, recipient, kind, text, created_ms, delivered_turn, delivered_ms)`
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
`start`, `send`, `accept`, `resume`, `cancel`, `task{plan, id}` (one stored task with
its report, verdict and commits), and `events{conversation, after, wait_ms}`,
a long poll returning at most 500 events / 1 MB. A daemon from another build is
replaced when idle. The socket is `<state>/core.sock`, or
`/tmp/antislop-<uid>/<hash>.sock` when that path is too long for Unix sockets.

- `antislop chat` in a terminal opens the TUI on the project's latest
  conversation (`--new` for a fresh one, `--conversation ID` for a given one).
  A poller thread long-polls events and every request runs on its own thread,
  so the draw loop never waits. The screen has two panels: the chat (messages,
  the orchestrator's tool calls, reviews, mail) and the team graph. The graph
  draws every agent as a box: the orchestrator between the plan verifier and
  the auditor on top, below them each task's worker with its verifier right
  under it, tasks in columns by dependency depth. Edges are routed through the
  gaps between boxes: dependencies, verification, messages (with a count,
  highlighted for 10 s after one is sent) and supervision; `f` cycles which
  relations are shown (supervision only on its own, since it reaches every
  worker). The layout depends only on the plan's shape, so nodes never jump as
  events arrive. A "teraz" overlay in a corner lists the steps actually running,
  or says that the plan waits for the user, or that nothing runs; it moves to
  the other corner rather than cover the selected node. Tab moves focus between
  the panels (on terminals narrower than 100 columns it switches which one is
  shown); in the graph, arrows select the nearest node in that direction and
  Enter opens its details full screen: Przegląd (task, scope, goal, the
  verifier's focus and, for a verifier, its risks), Sesja (the agent's turns
  and tool calls), Wiadomości (mail to and from it) and Wynik (the worker's
  report and commits, or the verifier's verdict). Keys: Enter send, Alt+Enter
  new line, Ctrl+A run the proposed plan, Ctrl+R resume what the plan left
  unfinished, Ctrl+X cancel, PgUp/PgDn scroll, Ctrl+N new conversation, Esc
  back (from the chat: detach), `q` in the graph detaches.
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
  The orchestrator then answers what it was answering, and an autonomous
  conversation resumes its cut-off plan.
- A turn that fails because the CLI needs a login or hit a usage/rate limit
  leaves its task `waiting` (not `failed`), with the CLI's message.
- `resume` (Ctrl+R, `/resume`) runs again every task of a plan that ended
  without approval. Agents resume their own sessions and are told what
  happened; a writer continues in its worktree (or from its last result); a
  change that was approved but could not be integrated is integrated again
  without another worker turn.
- At most four tasks of a plan run at once.
- No plan is shown to the user, or run, without passing through the plan
  verifier; an autonomous conversation never runs a plan the verifier did not
  approve, and stops for the user after three plans in a row.
- The audit counts as part of the running plan: it can be cancelled and keeps
  the conversation busy, and the orchestrator starts on the results before the
  conversation can look idle.
- A session is resumed only by the CLI that created it.

## Known limits

- A writer that depends on exactly one writer starts from that writer's
  result; one that depends on several starts from `HEAD`, so without
  `local_commits` it does not see their uncommitted results.
- A plan runs in waves: a task waits for every task of the previous wave.
- Resume works per plan, not per task, and `waiting` tasks are not retried
  on a timer.
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
6. ✅ Plan verifier, autonomous mode, final audit, per-kind verifier
   checklists, team shapes in the orchestrator's instructions
   (`tests/lean_review.rs`).
7. ✅ Recovery: resume plans through the agents' sessions, `waiting` for
   logins and limits, integration retried on resume, work picked up after a
   restart (`tests/lean_recovery.rs`, `tests/lean_writer.rs`,
   `tests/lean_core.rs`).
8. ✅ Messages between agents and from the user to one agent
   (`tests/lean_messages.rs`).
9. ✅ TUI team graph, progress overlay and agent details (design §8.1–8.4;
   unit tests in `src/lean/tui`).
