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
| gemini | `-p --output-format stream-json --approval-mode plan` | `auto_edit` (no shell, no sandbox) |

Extra scope directories go through `--add-dir` (gemini: `--include-directories`). Follow-up turns resume the
CLI's own session (`codex exec resume <id>`, `agy --conversation <id>`,
`claude --resume <id>`, `gemini --resume <id>`), so earlier context is never re-sent.

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
   approve, or one that asks for the same work as the last plan that ran
   (after the user's last message), waits for the user. There is no count of
   plans (design §14: no limits in the MVP). Deviation from design §6.2:
   in supervised mode the first three correction rounds of a task run
   without asking, then the task waits for the user; asking at every round
   is what made the first runtime unusable.
4. **Task.** The worker runs (worktree if `writes`, otherwise the project
   read-only). A writer's changes are committed in its worktree by the runtime.
   With `workspace.mode = "in-place"` writers work in the project itself
   (see In-place mode).
5. **Verification.** The verifier gets the goal, the worker's report, the diff
   range (writers), its risks and a short checklist for the task's kind
   (`tasks::checklist`: e.g. assertions that fail on the bug for testing,
   suppressed warnings for static analysis, recomputation for analysis,
   sources that support claims for research). Risks (`R1`…) and checklist
   items (`C1`…) are its numbered obligations. It reads the files itself and
   returns `{approved, findings, coverage}`, where `coverage` accounts for
   every obligation as `ok`, `issue` or `n/a`, each with a note. Findings
   resume the worker's session. Supervised: after three rounds the task waits
   for the user. Autonomous: rounds go on until approval; a verifier that
   repeats its findings word for word means the worker is not getting closer,
   and the task waits for the user.
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

## Comments

A comment targets one element by key: an agent (`worker:t2`, `verifier:t2`,
`orchestrator`, `plan-verifier`, `auditor`) or a diff (`diff:t2:FILE[:LINE]`).
It is stored with the plan it was made on and reaches the orchestrator at once
as a user message `[user comment #N on <what it is>] text` (one transaction
with the comment, so a crash cannot keep one without the other). Until the user
resolves or deletes it, every later orchestrator prompt lists it under
"Comments the user has not resolved yet"; delivery never resolves it, and a
comment grants nothing: work it asks for goes through a plan as usual. Every
change is a `comment` event carrying the whole comment.

## Diff and code review

A writer's change is read where it is: in its worktree while it works (the
working tree against its base, so uncommitted edits show; untracked files only
once the runtime commits them), else in the project's repository between its
base and result commits. A code review the user asks for runs with the task's
verifier profile in a session of its own (`code-review:<task>`), read-only, in
that directory, with the user's open comments on the change in its prompt. It
records a `code_review` event (running, then the verdict) and sends its
findings to the orchestrator as a message; it never changes the task's status.
One code review runs per conversation at a time; Ctrl+X cancels it.

## In-place mode

`workspace.mode = "in-place"` (design §11.1) runs writers in the project
directory instead of worktrees, for users who want to watch the files change.

- Writers run one at a time (a process-wide lock; a waiting writer still
  honours cancel). Read-only tasks keep running beside them.
- Before the worker starts, the runtime records a snapshot commit of the
  working tree (tracked and untracked files, through a scratch index; the
  user's index, branch and files are untouched). Its parent is `HEAD`; the
  files it differs in are the user's own edits, and the worker is told not to
  modify them.
- After each worker turn another snapshot is the task's `result`; the
  verifier and the code review read `git diff base result`, the diff view
  shows the same range.
- A worker that touched a file the user was editing ends `needs_user`; the
  report names the files and how to get the user's version back
  (`git checkout <base> -- <file>`).
- An approved change is already in place: with `local_commits` the runtime
  commits exactly the files the task changed; otherwise it stays as
  uncommitted changes.

This protects the user's edits by detection, not confinement: a worker that
ignores the instruction overwrites the file, and the snapshot is what makes
the user's version recoverable.

## State

`core.db`, WAL, normal synchronous:

- `conversations(id, project, created_ms)`
- `messages(seq, conversation_id, author, text, created_ms)`
- `turns(id, conversation_id, role, provider, session_id, status, error, upto_message, started_ms, finished_ms)`
- `tasks(id, conversation_id, plan_id, position, spec, status, round, report, verdict, base, result, created_ms)`
- `mail(id, conversation_id, sender, recipient, kind, text, created_ms, delivered_turn, delivered_ms)`
- `comments(id, conversation_id, plan_id, target, label, text, status, message, created_ms, updated_ms)`
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
its report, verdict and commits), `comment{conversation, target, text}`,
`comment_status{conversation, id, status}`, `diff{conversation, plan, id}` (a
writer's change as files with counts and numbered lines, at most 2 MB of
patch), `review{conversation, plan, id}` (a code review now), and `events{conversation, after, wait_ms}`,
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
  and tool calls), Wiadomości (mail to and from it), Wynik (the worker's
  report and commits, or the verifier's verdict), Kontrole (each obligation
  of the task's verifier, risks and checklist, with how the latest verdict
  accounted for it) and Komentarze. `/` in the graph finds an agent by role
  or task title; Ctrl+P opens a palette of every command; `?` (F1 anywhere)
  shows the keys of the active view. `c` comments
  on the selected agent (in the graph or its details); a box shows how many
  comments on it are open. On Komentarze, `r` resolves, `o` reopens and `d`
  deletes the selected one; a comment made on an earlier plan is marked so.
  `d` on a task's agent opens its change full screen: changed files with their
  status and +/− counts beside the unified diff with old and new line numbers;
  `c` comments on the selected line (or file), comments show under their lines,
  `R` asks for a code review and `r` reloads.
  Keys: Enter send, Alt+Enter
  new line, Ctrl+A run the proposed plan, Ctrl+R resume what the plan left
  unfinished, Ctrl+X cancel, PgUp/PgDn scroll, Ctrl+N new conversation,
  Ctrl+P commands, F1 help, Esc
  back (from the chat: detach), `q` in the graph detaches.
- `antislop chat "message"` (or piped input) prints events line by line and
  exits once the conversation is idle; `/accept` and `/cancel` lines work too.
- `antislop export` writes the conversation's history (`--conversation ID`,
  else the project's latest) to stdout as JSON lines: a header, then every
  event, including which CLI, model and effort each turn ran with. Private
  keys, bearer tokens, well-known token formats and `password=`-like values
  are redacted; that is best effort, so the export stays where the user puts
  it (design §15, §13).

## Guarantees

- `accept` names the plan the user saw (`plan_id`); the scheduler and the
  report to the orchestrator stay pinned to it even if another plan is
  proposed meanwhile. Queuing a plan is one transaction.
- A failed or skipped task skips everything that depends on it, transitively.
  A failed task's report keeps the agent's error.
- A verdict that approves while listing high or medium findings is a
  rejection. A task verifier's verdict approves only when it accounts for
  every obligation with a note; one marked `issue` rejects it and its note
  reaches the worker. A verdict that leaves obligations out is asked for once
  more in the verifier's session (earlier entries kept), then the task waits
  for the user: the worker cannot fix a verifier that skipped its checks
  (design §18 scenario 30). A plan where a task depends on itself or a later task is
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
- Every task whose dependencies are approved runs at once (no concurrency
  limit, design §14); writers land one at a time, and in place they work one
  at a time.
- No plan is shown to the user, or run, without passing through the plan
  verifier; an autonomous conversation never runs a plan the verifier did not
  approve, and stops for the user when a plan repeats the work of the last
  one that ran.
- The audit counts as part of the running plan: it can be cancelled and keeps
  the conversation busy, and the orchestrator starts on the results before the
  conversation can look idle.
- A session is resumed only by the CLI that created it.
- `/pause` (or the palette) starts no new plan work: worker, verifier and
  audit turns wait, turns already running finish, the orchestrator still
  answers. `/continue` goes on; Ctrl+X cancels what waits too. A pause is
  stored, so it holds across a restart (design §14).
- A plan never widens where an agent may write: directories of a task's
  scope outside the project are passed to the CLI (`--add-dir`,
  `--include-directories`) only for read-only agents, since a writer's CLI
  would make them writable (design §18 scenario 15).
- The daemon runs with the configuration it read at start. A frontend that
  finds `architect.toml` changed restarts an idle daemon; a busy one keeps
  its configuration for the running work and the frontend says the new one
  applies once that work is done (scenario 17).
- With `NO_COLOR` set the TUI draws no color. Nothing depends on color
  alone: task states have marks (✓ ✗ ! ● ○ –), the selected node a double
  border, the active tab brackets, obligations ✓ ✗ – ?, diff lines + and −;
  relations are told apart by layout (a verifier sits under its worker,
  dependencies run between columns, messages carry counts) and `f` shows one
  kind at a time (design §17).
- Agents' text is stripped of control characters before it reaches the
  terminal, in the TUI and in line mode (design §13).
- Token use is what the CLI reported, summed per agent in its details
  (Przegląd). A CLI that reported nothing shows `niedostępne`, a side it
  left out shows `nieznane`, and turns it did not report for are counted as
  such: missing data is never a zero (design §14). Tokens are not turned
  into money.

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
- The history records each turn's CLI, model and effort, not the CLI's
  version.
- In-place: read-only tasks running beside a writer may see its partial
  edits, and the user's edits made while a writer runs are attributed to the
  writer.

## Stages

1. ✅ Store, agent adapters (codex, agy, claude; gemini added later from its
   documented stream-json, both field spellings accepted, not yet run against
   a real gemini binary), conversation with the
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
10. ✅ Persistent comments on agents and diffs (design §8.5;
   `tests/lean_messages.rs`).
11. ✅ Diff view with line comments and a code review on request (design §8.6;
   `tests/lean_writer.rs`, `src/lean/diff.rs`).
12. ✅ In-place mode with snapshot commits (design §11.1;
   `tests/lean_writer.rs`, `src/lean/git.rs`).
13. ✅ Acceptance scenarios of design §18 mapped to mechanisms and tests
   ([acceptance.md](acceptance.md)); runtime gaps closed: verdict coverage of
   every obligation, idempotent integration, writers never given directories
   outside the project, configuration kept by a busy daemon.

Open (design M5): measuring real CLIs on the scenarios that rest on
instructions, and a schema version for `core.db`.
