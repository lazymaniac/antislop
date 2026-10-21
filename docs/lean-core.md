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

Profiles in `antislop.toml` select a provider, model and effort; routing assigns
these profiles to work kinds and roles. Profiles contain only CLI/model/effort
settings; they do not repeat `work_kinds` or `roles`. `[routing.KIND].executor`
and `.verifier` select the worker and reviewer for that work kind, while
`[orchestrator].profile` selects the conversation orchestrator. A profile may
be reused across work kinds and roles; each verifier still has its own session.
All assignments must name existing profiles.

Effort values are `low`, `medium`, `high` and `xhigh`, passed directly to the
CLI. Support depends on the selected model; an unsupported level produces the
CLI's error rather than silently lowering the effort.
Gemini has no native effort flag, so no effort argument is sent to it.

Configuration only exposes settings used by the runtime. Obsolete fields
(`isolation`, `managed_home`, `role_providers`, `policy.revision`,
`policy.local_checks`, `orchestrator.review_checks_per_part`, `effort_map` and
`native_effort`, profile `work_kinds` and `roles`) are rejected: remove them from
older configuration files.
Replace the old effort names `light`, `standard` and `deep` with `low`, `medium`
and `high`, and workspace mode `worktree` with `task-worktrees`.

The runtime determines CLI access for each turn from its role and whether the
task writes files, as shown below; `workspace.mode` determines where writers
work.

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
| opencode | `run --format json --agent antislop` (read/search/web tools, no edits or shell) | same transport, editing and shell enabled |

Extra scope directories go through `--add-dir` (gemini: `--include-directories`). Follow-up turns resume the
CLI's own session (`codex exec resume <id>`, `agy --conversation <id>`,
`claude --resume <id>`, `gemini --resume <id>`, `opencode run --session <id>`), so earlier context is never re-sent.

OpenCode uses a dedicated primary agent in `OPENCODE_CONFIG_CONTENT`; other
inline configuration is preserved. Native `task` subagents and interactive
questions are denied. Additional scope paths are allowed through its
`external_directory` permission; other external paths are denied. This is a
CLI permission policy, not an OS sandbox. Effort maps to `--variant`
(`low`/`medium`/`high`/`xhigh`), whose effect depends on the selected model/provider.
The JSON parser records session IDs, completed tool calls and per-step token
usage, and returns only the final model step's text after a `stop` finish.
Tool-call steps alone, truncated responses and error events do not count as
successful turns. Provider commands may prefix the transport, for example
`["omlx", "launch", "opencode", "--model", "MODEL", "--"]`; the profile model
should then be `omlx/MODEL`. Non-JSON launcher diagnostics are retained in a
bounded tail so startup failures on stdout remain actionable.

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
   A plan that changes code always gets the static analysis, testing and
   integration gates it lacks (`policy.required_gates`, which needs routes for
   them, and `policy.max_parallel` bounds how many tasks run at once). The
   context scout is required before the writers: the plan verifier is told
   when it is missing and does not approve the plan.
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
with the comment, so a crash cannot keep one without the other). Comments on
the plan itself (a line, the whole plan) are the exception: they are kept as
`draft`s while the user reads and reach the comment agent together, in one
message and one turn, when the user sends the review (`R` in the plan view,
the `review_comments` request). The agent ends its answer with a
```` ```comments ```` block listing the comments its revised plan took care of;
once the plan verifier approves the plan only those are resolved, and a
comment it only answered in words stays open (`comment_replied`). Until the user
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
  files it differs in are protected, except for paths still identical to the
  latest approved writer's output for that path in this project. Approval
  order comes from persisted task events, across plans, conversations and
  daemon restarts. Only paths the approved writer changed qualify; unrelated
  user files merely present in its snapshot do not. Later user edits (including
  reverts to an older approved version) and unapproved results stay protected,
  and the worker is told not to modify them. This lets sequential writers
  extend approved uncommitted changes with `local_commits = false`.
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
- `events(seq, conversation_id, turn_id, kind, payload, created_ms)` — append-only; tool
  calls are stored as a name and a one-line detail, never their output.
- `pauses(conversation_id)` and `modes(conversation_id, autonomous)` — what
  the user set for a conversation.

`PRAGMA user_version` holds database schema version 6 (independent of the TOML
configuration's `schema_version = 1`). Opening an older database migrates it:
1 → 2 adds event timestamps; 2 → 3 adds process-group IDs; 3 → 4 adds plan
associations; 4 → 5 adds compact task feedback; 5 → 6 rebuilds the events table
transactionally to permit daemon-wide events with a NULL conversation id.
Existing event sequence IDs, plan associations, payloads and timestamps are
preserved, and both event indexes are recreated. Per-conversation reads include
global config events without exposing another conversation's private events.
A binary that only supports an older database schema refuses version 6 rather
than rewriting it; downgrade requires a pre-migration backup. An agent's details
show its time in turns where both ends are known.

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
replaced when idle. While busy, frontends for the same project and configuration
path reconnect with a notice; work keeps its original build and configuration
until it finishes and the frontend is reopened. A busy daemon for another
project or configuration path cannot be reused; use a separate `--state`
directory to run both. The socket is `<state>/core.sock`, or
`/tmp/antislop-<uid>/<hash>.sock` when that path is too long for Unix sockets.

- `antislop chat` in a terminal opens the TUI on the project's latest
  conversation (`--new` for a fresh one, `--conversation ID` for a given one).
  A poller thread long-polls events and every request runs on its own thread,
  so the draw loop never waits. The screen has two panels: the chat (messages,
  the orchestrator's tool calls, reviews, mail) and the team graph. The graph
  draws every agent as a box: the orchestrator between the plan verifier and
  the auditor on top, below them one node per task combining worker and verifier,
  with tasks in columns by dependency depth. Edges are routed through the
  gaps between boxes: dependencies, verification, messages (with a count,
  highlighted for 10 s after one is sent) and supervision; `f` cycles which
  relations are shown (supervision only on its own, since it reaches every
  worker). The layout depends only on the plan's shape, so nodes never jump as
  events arrive. A "now" overlay in a corner lists the steps actually running,
  or says that the plan waits for the user, or that nothing runs; it moves to
  the other corner rather than cover the selected node.  Tab moves focus between
  the panels (`[Chat]`, `[Team]`; on terminals narrower than 85 columns it switches
  the single visible panel in compact mode); in the graph, arrows select the nearest node in that
  direction and Enter opens its details full screen. The tabs depend on the agent
  (`tabs_for` in `details.rs`): Overview (task, scope, goal, the verifier's focus and,
  for a verifier, its risks, followed by both sessions' transcripts drawn like the CLI:
  prompt, reasoning, commands with their output, answers; it follows the newest entry
  until the user scrolls up, and `Enter` folds prompts), Messages (mail to and from it),
  Result (a worker's report and commits), Checks (each task obligation,
  risk and checklist item, with how the latest verdict accounted for it), Review (the
  plan verifier's and the auditor's latest verdict) and Comments. A dedicated conversation
  inspector opens via `Ctrl+I` or palette (`Ctrl+P`). Within inspectors, `v` toggles between Readable
  and Raw JSON view, `/` searches whole records (`DetailRecord`) with query highlighting, and per-tab
  scroll and search memory is preserved across tab and conversation switches. Explicit notices state
  historical data gaps (`unavailable (CLI did not report them)`, `unknown`).
  `/` in the graph finds an agent by role or task title; `@` on an empty draft or `Ctrl+P > Address a message`
  opens a recipient picker (`@orchestrator`, `@all`, `@worker:t1`, `@verifier:t1`), validated by the daemon.
  Ctrl+F searches the conversation (newest first; Enter brings the entry to the top, highlighted);
  Ctrl+P opens a palette of every command; `?` (F1 anywhere) shows the keys of the active view. `c` comments
  on the selected agent or sub-element with precise targets (`plan:`, `task:`, `check:`, `finding:`,
  `report:`, `artifact:`, `diff:`, `worker:`, `verifier:`, `conversation`); a box shows how many comments
  on it are open. On Komentarze, `r` resolves, `o` reopens and `d` deletes the selected one; a comment
  made on an earlier plan is marked so (`[dotyczy poprzedniego planu]`).
  `d` on a task's agent opens its change full screen: changed files with their status and +/− counts
  beside the unified diff with old and new line numbers and reviewer profile in the header; `c` comments
  on the selected line (or file), comments show under their lines, `R` asks for a code review and `r` reloads.
  Critical actions (accepting plans, resuming plans, reviewing diffs, cancelling) present a pinned
  confirmation card (`ConfirmationCard`) with frozen `scope_hash`; `Enter` or `y`/`Y` confirms, `Esc` or
  `n`/`N` rejects. If an asynchronous background event alters the plan or scope before execution, the
  card transitions to `Stale` (`[nieaktualne: stale_scope]`), and execution is safely blocked.
  Keys: Enter or Ctrl+S send message (submit draft), Alt+Enter or Ctrl+J new line in draft, ↑/↓ navigate
  wrapped lines in draft (preserving `preferred_col`); when on row 0 (or draft is empty), ↑ stashes the
  draft into `history_draft` and recalls sent messages history (successive ↑ moves backward); when browsing
  history, ↓ on the last row moves forward and restores `history_draft` with its exact cursor and column position.
  The latest 1,000 submitted messages persist in `input-history.db` in the project’s state directory
  (normally `.antislop/`) and are restored on startup. Each project keeps its own history;
  concurrent frontends append separate rows without overwriting each other.
  PgUp/PgDn scroll transcript, Ctrl+A run proposed plan (on empty draft) or Home, Ctrl+R resume plan
  (on empty draft) or search history, Ctrl+U/Ctrl+K delete to start/end of line, Ctrl+W or Alt+Backspace
  delete word backward, Ctrl+E move to end of line, Alt/Ctrl+Left and Alt/Ctrl+Right word navigation,
  Ctrl+X cancel, Ctrl+N new conversation, Ctrl+O switch conversation, Ctrl+I conversation inspector,
  Ctrl+P commands, F1 help in every view (including comments and search lists; ? also opens help in the
  graph, details and diff). Help scrolls with ↑/↓, k/j, PgUp/PgDn, Home, End and closes with Esc, F1 or ?;
  other typing, paste and action shortcuts stay blocked, preserving the drafts and search query beneath
  it, with no cursor showing through. Ctrl+C detaches from any view.
  Outside help: Esc cancels history when browsing and restores the in-progress draft; when drafting,
  clears a non-empty draft (a draft cleared by Esc is not retrievable via ↑/↓, as ↑ on an empty buffer
  overwrites the history draft slot); detaches when draft is empty; in modals, pickers, details and diff,
  Esc dismisses the overlay without losing underlying drafts. `q` in the graph detaches.
  Text editing, visual wrapping, and deletions operate strictly on Unicode extended grapheme cluster
  boundaries (`unicode_segmentation`), safely handling combining accents (`a\u{301}`) and multi-byte
  ZWJ emoji sequences (`👩‍💻`) without breaking glyphs or splitting UTF-8 bytes on Backspace or Delete.
  Draft retention and recovery: network or daemon errors never lose text — drafts are automatically
  restored into `input` if empty, or preserved in `failed_send` if the user was typing anew; conversation
  switching (`Ctrl+O`) isolates and preserves drafts per conversation (`A → B → A`); per-target comment
  drafts are remembered on modal close and restored when commenting on that target again.
  Drafts reserve a display column for the insertion cursor; cursor rows and viewport offsets support
  more than 65,535 lines. Search queries and activity labels are bounded to the viewport before conversion
  to terminal coordinates. Bracketed paste normalizes CRLF/CR to LF, sanitizes control characters, caps
  length at 64 KB at character boundaries, and routes paste according to modal ownership. The layout
  enforces a 12x5 minimum terminal guard and an 85-column breakpoint switching between side-by-side
  split and single-panel compact views.
- `antislop chat "message"` (or piped input) prints events line by line and
  exits once the conversation is idle; `/accept` and `/cancel` lines work too.
- There is no export command: the history is in the state directory (`.antislop/`).
  `antislop export` says so instead of sending a message.

## Guarantees

- `accept` names the plan the user saw (`plan_id`); the scheduler and the
  report to the orchestrator stay pinned to it even if another plan is
  proposed meanwhile. Queuing a plan is one transaction.
- A failed or skipped task skips everything that depends on it, transitively.
  A failed task's report keeps the agent failure cause with credentials redacted.
- A verdict that approves while listing high or medium findings is a
  rejection. A task verifier's obligations are the task's risks (`R1`…),
  the checks of every verification (`V1`…: each requirement with its
  qualifiers, key claims reproduced, a counterexample sought, nothing beyond
  scope, earlier findings resolved; design §7.6) and its kind's checklist
  (`C1`…). Its verdict approves only when it accounts for every obligation
  with a note; one marked `issue`, or `blocked` (could not be settled:
  missing data is not `ok`), rejects it and its note reaches the worker. A verdict that leaves obligations out is asked for once
  more in the verifier's session (earlier entries kept), then the task waits
  for the user: the worker cannot fix a verifier that skipped its checks
  (design §18 scenario 30). A plan where a task depends on itself or a later task is
  rejected.
- A turn succeeds only with a complete result *and* a clean exit. When a turn
  ends or is cancelled its whole process group is killed, so no descendant
  outlives it or holds the turn open. Only the tail of stderr is kept.
- On start, the CLIs of turns left running by a daemon that died are ended
  first: each runs in its own process group and would otherwise go on
  beside its resumed turn. A group is ended only while its number still
  belongs to that CLI (a group leader running the provider, started when
  the turn did). Then turns left running become `interrupted` and
  unfinished tasks become `cancelled` with the reason; nothing waits for a reconciliation.
  The orchestrator then answers what it was answering, and an autonomous
  conversation resumes its cut-off plan.
- A task that stops short of approval (`needs_user`, `waiting`, `failed`,
  `cancelled`) says why in its event; the chat, the line mode and the task's
  Overview show the reason (design §8.4).
- Before writers start in worktrees, the user is told which files have
  uncommitted changes: writers start from the last commit and do not see
  them, and a change landing on one of them waits for the user (design
  §11.1).
- Every worker and verifier is told to keep to the project's own
  instructions (AGENTS.md, CLAUDE.md, GEMINI.md…) whichever CLI it is, to
  name contradictions, and never to change its own permissions; a worker
  stops and says so when its goal needs something outside its task (design
  §10.3).
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
- A CLI's own subagents are turned off, so no team runs that no verifier
  checks (design §9.5): codex runs with `features.multi_agent=false`, claude
  without its `Task`/`Agent` tool.
- `/unblock` (also `u` on a stopped step and the header's `[Unblock]`) offers
  the recovery actions that fit why the task stopped (`recover` in the
  protocol; the TUI reads a fresh `freshness_token` itself). When a plan ends
  with blocked tasks the runtime messages the orchestrator once with the
  stopped steps, their reasons and the ways out; the chat also keeps a short
  Progress card of finished rounds and task outcomes.
- `/pause` (or the palette) starts no new plan work: worker, verifier and
  audit turns wait, turns already running finish, the orchestrator still
  answers. `/continue` goes on; Ctrl+X cancels what waits too. A pause is
  stored, so it holds across a restart (design §14).
- `/mode autonomous|supervised` (or the palette) sets one conversation's mode,
  stored with its event, over the configured default. It applies to what the
  conversation does next: the next plan, the next correction round, a resume
  after a restart. It accepts no plan that was waiting and approves nothing
  after the fact; capabilities (`local_write`, `local_commits`) stay the
  policy's (design §6.2).
- A plan never widens where an agent may write: directories of a task's
  scope outside the project are passed to the CLI (`--add-dir`,
  `--include-directories`) only for read-only agents, since a writer's CLI
  would make them writable (design §18 scenario 15).
- An idle daemon whose state directory or socket is gone exits within a few
  seconds; nothing lingers after `.antislop` is deleted.
- A frontend following a conversation outlives its daemon: when the daemon
  dies it is started again (picking up the work) and following goes on.
  Only following is repeated; a request that changes something is not
  (state the user deleted is not recreated either).
- `/config` applies validated configuration through the running daemon. New
  turns take a new immutable profile snapshot; running turns retain theirs.
  File changes outside this protocol still use idle-daemon restart/reconnect
  behavior; they are not watched and applied automatically (scenario 17).
- With `NO_COLOR` set the TUI draws no color. Nothing depends on color
  alone: task states have marks (✓ ✗ ! ● ○ –), the selected node a double
  border, the active tab brackets, obligations ✓ ✗ – ?, diff lines + and −;
  relations are told apart by layout (worker/verifier share a task node,
  dependencies run between columns, messages carry counts) and `f` shows one
  kind at a time (design §17).
- Without a UTF-8 locale, or with `ANTISLOP_ASCII` set, the TUI draws in
  ASCII: box lines become `-|+`, the selected node's border `=`, marks
  `+ x ! * o`, accented letters lose their diacritics and other characters
  become `?` (design §8.3).
- Agents' text is stripped of control characters before it reaches the
  terminal, in the TUI and in line mode (design §13).
- Token use is what the CLI reported, summed per agent in its details
  (Overview). A CLI that reported nothing shows `unavailable`, a side it
  left out shows `unknown`, and turns it did not report for are counted as
  such: missing data is never a zero (design §14). Tokens are not turned
  into money.
- Team graph geometry dynamically adapts to the actual interior dimensions of
  the graph panel (`inner.width`, `inner.height`), utilizing available columns and
  rows for parallel tasks, long chains, branches, and cycles while keeping
  worker–verifier pairs together. Taller viewports fit strictly more whole readable
  cards without scrolling (20 -> 26 -> 46 lines, up to 100% visible on 160x50).
- Compacting on fold and filter: collapsing tasks (`z`/Space) compacts vertical
  space to 0 lines, pulling subsequent tasks upward. Filtering admin nodes (e.g. `task 1`)
  reclaims row 0 for task cards. Column selection maps visible task depths sequentially
  starting at column 0 (`rel_depth`), eliminating empty preceding columns.
- A single unified geometry powers rendering, orthogonal edge routing (strictly
  outside card interiors with explicit direction markers `▲`, `▼`, `◀`, `▶`),
  arrow key navigation, and scrolling. Selecting a node or updating streaming
  activity never shifts or displaces graph nodes. Arrow navigation moves strictly
  among visible nodes in the active filter.
- Orthogonal edge routing incorporates corridor bypass: non-adjacent nodes in the
  same column bypass intermediate cards through a side corridor lane
  (`from.right() + GAP_X / 2`), with 0 card interior intersections verified across
  240 node pairs on a 4x4 grid and in all filter modes.
- The 'now' progress overlay tests all candidate viewport corners and
  automatically yields (hides) if visible cards occupy candidate corners.
- Canvas memory is strictly bounded: `Canvas::new_window` allocates only
  `O(inner.width * inner.height)` viewport memory, remaining independent of
  graph size and preventing OOM crashes on large graphs (tested on 500+ tasks).
- Pinned confirmation cards (`ConfirmationCard`) with frozen `scope_hash` for
  `AcceptPlan`, `ResumePlan`, `ReviewDiff`, and `CancelWork`; background updates
  mark pending cards `Stale` (`[nieaktualne: stale_scope]`), and the daemon verifies
  `scope_hash` in a SQLite transaction with zero side effects on stale state.
- Complete parity verification against `main@7ad664d25fbeebd3e8906caf29e9fb9925f20fc8`
  with test proofs and architectural notes is documented in [parity.md](parity.md).

## Several projects in one screen

`Ctrl+G` (or `/project`, or the palette) lists the projects open on the screen
and those opened before (`~/.config/antislop/projects.json`); choosing one
switches to it, and typing a directory path (`/…`, `~/…`, `./…`) opens it. A
project is its own directory: its `antislop.toml`, its `.antislop/` state and
its own daemon, started as when antislop is run there alone, so nothing of one
project's configuration, plans or sessions reaches another. A directory that is
not trusted yet is asked about first and nothing is created in it before the
answer. The header names the project and counts what the others do
(`alpha (2 more ●1 !1`: one working, one waiting for the user), refreshed in the
background; `Ctrl+O` also lists the other projects' conversations, and choosing
one switches the project too. Drafts, open dialogs and scroll positions are
kept per conversation, so switching back finds them as they were.

## Known limits

- A writer that depends on exactly one writer starts from that writer's
  result; one that depends on several starts from `HEAD` with all their
  results laid on it (`git::combine`), and a result that does not apply is
  an error naming it.
- Tasks start as soon as their own dependencies are approved, at most
  `policy.max_parallel` at a time.
- `View::apply(plan_proposed)` replaces the active task cards of the view
  projection with the newly proposed plan's tasks; while past tasks, verdicts,
  turns, and messages remain fully persisted in `core.db`, simultaneously
  projecting historical plans and past workers/verifiers into the live team
  graph is a planned enhancement for the next plan.
- One daemon still serves one project. The screen can hold several (below);
  a project's conversations are never mixed into another's daemon.
- Resume works per plan, not per task, and `waiting` tasks are not retried
  on a timer.
- Read-only modes are prompts and CLI modes, not OS confinement; `scope`
  guides agents but does not limit what they read.
- Hostile instructions in the project's files are held by the agents'
  instructions and by review, not by OS confinement: codex's
  `workspace-write` sandbox leaves temporary directories writable.
- An interrupted verification is requeued with its task, so the worker
  gets a turn before the verifier resumes its session.
- A read-only codex agent cannot run checks that write (tests that create
  temporary directories); the orchestrator plans such checks as `writes`
  tasks.
- Prompts are passed as arguments (the orchestrator's plan report is bounded
  by 40k characters per task).
- The history records each turn's CLI, model and effort, not the CLI's
  version.
- agy and gemini offer no switch for their own subagents; the runtime cannot
  turn them off and would not see them.
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
9. ✅ TUI team graph, adaptive viewport layout, progress
   overlay and agent details (design §8.1–8.4; comprehensive App and rendering
   tests in `tests/lean_tui.rs` and `src/lean/tui`).
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

14. ✅ Control and observability: pause and continue, a conversation's mode
   switched for what comes next, why a task stopped, token use as reported,
   agents' escapes kept off
   the terminal, `NO_COLOR` and ASCII drawing, conversation search, common
   verifier checks with a `blocked` status, the CLIs' own subagents off,
   a warning about uncommitted changes writers do not see, a versioned
   `core.db` (design §6.2, §7.6, §8, §9.5, §10.3, §11.1, §13–§15).

## Design §16 build stages

| Design stage | Exit criterion | Where it is met |
| --- | --- | --- |
| M0: integration spike | Versions and limits documented; a transport chosen per CLI | [Agents](#agents) (one stream-JSON or `exec --json` transport per CLI, resume by the CLI's own session id). gemini is implemented from its documentation and has not run against a real binary. [m0/](m0/README.md) keeps the old runtime's first probes. |
| M1: local runtime | State recovered without losing approvals or doubling actions | Stages 1, 4 and 7; [acceptance](acceptance.md) scenarios 9, 10 and 16 |
| M2: vertical slice | A task finished on evidence; a weak result rejected | Stages 2 and 3; scenarios 1 and 3 |
| M2Q: quality contracts | A fake provider cannot close a task that skips a requirement, risk or gate | Stages 6 and 13 (verdict coverage of every obligation, final audit); scenarios 27–31 |
| M3: team and TUI | Real events, navigation, honest delivery states, comments answered | Stages 8–11; scenario 6 |
| M4: full MVP | Every role verified; acceptance scenarios pass | Stages 12–14; all 40 scenarios in [acceptance](acceptance.md) |

Design §20's open decisions stand as follows. Isolation and the supported
CLI versions (1) and session resume per CLI (2) are settled above and in
[Known limits](#known-limits). The name (3) is `antislop`. The licence and distribution (4) are left to
the owner, as the design asks.

The TUI configuration modal uses GetConfig and SetConfig asynchronously. It keeps a
separate field draft from chat and reports validation errors beside fields. Ctrl+S
publishes valid changes through the daemon; running turns retain their existing
profile snapshot. Failed requests keep the draft for retry. Optional overrides can
be cleared; args and command fields accept JSON arrays of strings. Saving patches the loaded TOML document and preserves unrelated comments and
formatting. Reopening refreshes a clean draft. External changes block stale drafts;
Ctrl+R explicitly reloads and discards edits. A pre-save read checks the source
and path. GetConfig returns a content revision; SetConfig requires
`expected_revision` and compares it under the daemon configuration lock before
publishing. Stale saves fail without changing the file, runtime or event log.
Clients omitting the revision are rejected. Uncoordinated external file writers
do not share this lock, so they must not write concurrently with SetConfig.


## Integrated configuration and TUI behavior

The t14 candidate combines main's unfinished-work preservation with chain
b6356f1 (t2/t3/t7/t8/t9, including the ported t4/t5/Checks changes). Configuration
resolution uses an explicit file, then the project file, then the global file in
`ANTISLOP_CONFIG_DIR`, `XDG_CONFIG_HOME/antislop`, or `~/.config/antislop`.
Resolution is read-only. Both CLI and multi-project TUI require trust before
creating project state. The `/config` modal edits every serialized option,
optional overrides, command arrays and profiles, preserving TOML comments,
drafts on failure, and the running turn's profile snapshot. A shared global
file affects other projects when they next load it; active daemons remain
independent until updated or restarted.

Worker/verifier role addresses remain compatible with stored events and mail,
but map to one graph node. Overview contains the summary and both sessions;
there is no separate Session tab. Checks invalidation handles round changes,
same-round verdict changes and live events of the selected task. Drag selection
and double-click word selection capture rendered graphemes in viewport-sized
storage; Ctrl+Y copies sanitized text. Clipboard sending rejects more than
64 KiB before allocating a base64 control string. OSC 52 failure can invoke
`pbcopy` directly (no shell), with a one-second timeout and bounded stderr.
Silent terminal refusal of OSC 52 cannot be detected.

Security review of the new code: config writes prepare a UUID-named sibling
with exclusive creation, initial mode 0600, and no following of a pre-existing
temporary symlink. They preserve the selected file's permissions, flush the
contents, atomically rename on the same filesystem, and sync the directory.
Configured target symlinks intentionally resolve to their existing destination.
Validation, stale-revision and pre-publication event failures leave old state
intact. A post-publication event-commit failure returns applied-with-warnings;
the on-disk configuration is authoritative on restart. Abrupt-exit tests before
and after rename verify that a complete old or new file remains and retry works.
An interruption may leave an unused temporary file; this test is process-exit
coverage, not a simulation of storage-device power loss.

The earlier t12 trust-order, database-trust migration, diagnostic and registry
temporary-file findings predate this chain. Existing t14 fixes are preserved;
no further baseline security refactor is included. Diagnostic redaction remains
best effort for known inherited secrets and does not cover arbitrary transcript
content.

The t13 recheck finds task-scoped Checks invalidation and single-record task
reads in this candidate. SessionCache reuses unchanged transcript records and
LayoutCache avoids repeated JSON/search rendering and wrapping on idle redraws
(covered by shared-record and wrapped-layout tests). Graph canvas and selection
storage remain bounded by viewport area. Graph layout is still reconstructed,
and the task endpoint still computes a Git freshness fingerprint; these are
remaining baseline performance costs, not an assertion of a host-overload
benchmark. No synthetic host-overload benchmark was run.


Integration validation on this candidate: `cargo check`, `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and the complete `cargo test` passed.
All 473 tests passed, with no ignored or filtered tests. Socket tests and the
process-group reap test ran successfully in this environment. The local routing
test was removed to match the user's change; `antislop.toml` was not modified.
Manual conflict resolutions were limited to `src/lean/mod.rs` (unfinished-work
and stopped-task mail semantics combined with configuration snapshots),
`src/lean/daemon.rs` (configuration stamp initialized after trust and Core open),
and this document (replace obsolete missing-feature claims with integrated
behavior). No duplicate t4/t5 or obsolete-run branches were merged.


### Contract review and integration scope

The explicit instruction to integrate chain b6356f1 also brings the minimal
manifest change required by its clipboard implementation: `base64 = "0.22.1"`
in Cargo.toml and the corresponding direct dependency entry in Cargo.lock.
That crate/version was already present transitively in the lockfile; no package
version or checksum changed. These two files are a declared exception to t14's
original source/docs-only scope, necessary to compile t8's OSC 52 encoder and
authorized by the later whole-chain integration instruction. In src/config.rs,
the only change relative to d050f2c is deletion of the 40-line local routing test
explicitly requested by the user; configuration parsing and validation are not
removed. The user's antislop.toml remains untouched.

Public protocol review: pre-existing operation names and optional conversation
fields retain their wire forms. The new GetConfig returns `toml`, `path` and a
content `revision`; SetConfig requires `toml` and `expected_revision`. Missing
revision and stale revision requests are rejected; valid updates retain the
daemon PID. The raw socket contract test checks all of those paths, file
preservation on rejection, and subsequent ordinary chat operation. Returning
`applied_with_warnings` after an event-commit failure means the file and runtime
were updated; clients must not treat that reply as an unapplied transport error.
The unit fault-injection test exercises both pre-publish rollback and post-publish
warning behavior. External writers do not participate in the daemon lock.

Conflict-resolution review: recover_task still delegates to recover_task_as with
the actor used to authorize user-only recovery. Its new configuration snapshot
is scoped to that recovery operation, and integrate receives that same snapshot.
run_task keeps mutable task-spec recovery, existing worktree/in-place protection,
and round policy snapshots. A new turn resolves its profile from the current
configuration immediately before setup; it does not replace a running process.
A regression initially failed when a verifier started in the same round retained
the worker's earlier profile; resolving each turn's profile separately fixes it.
answer_waiting still answers both worker and verifier mail for all stopped tasks,
not just approved tasks. The existing recovery, unfinished-work, mail and writer
flow suites cover these preserved paths. serve checks the user-owned trust
registry before opening Core, then initializes the configuration modification
stamp used by Identity. Role normalization changes UI node identity, not stored
worker/verifier role addresses or sessions.

The t12 fixes were reread in daemon::serve, trust::Registry::write and
agent::persisted_error. Their existing untrusted-entrypoint/prepared-database,
symlink and credential-redaction tests remain in the full suite. The t13 session-cache and
layout-cache tests verify invalidation for changed content, width, search and
raw display. Config editor tests cover drafts on transport/validation failures,
external edits, all editable field paths, global-file selection, and running
versus new turns. Selection tests cover graphemes, scrolling, content changes
and modal isolation. These are concrete contract and counterexample checks;
a fresh worker run is recorded, not a claim that an independent read-only
verifier executed commands.

Process failures keep their actionable cause in the turn, task report and
`turn_finished` event, so the TUI shows it. Credential redaction replaces known
secret environment values, URLs and labelled credentials before clipping;
provider and exit-status text and ordinary error descriptions remain visible.
Redaction is best effort: arbitrary unlabelled secrets in CLI prose cannot be
recognized reliably. The formatter is pure and does not write raw failures to
stderr or `daemon.log`. The `process_exit` event also retains the numeric exit
code or signal separately.

Check-event refreshes in the same round use `task_checks` with conversation, plan and
task authorization. The response contains only `id`, `round`, and `checks`;
it does not read reports or calculate filesystem freshness. Initial task details, verdict/task events,
and changes of round retain the full `task` read and its freshness contract.
A failed details request gets one immediate recovery attempt on the next event
batch, including an empty batch. Further failures back off exponentially from
one second to at most 30 seconds, and identical errors do not re-raise a dismissed
notice. Successful responses and changing the inspected task reset the backoff.
Task verdict events also invalidate an open Checks panel in the same round.
