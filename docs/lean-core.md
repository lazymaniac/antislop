# Lean core

The first runtime treated agents as tool-free reasoners: it copied project
sources into run state and prompts, then proved the agent had read every byte.
A 15 MB repository produced a 112 MB run document that was rewritten on every
progress step, prompts over provider limits, and runs that blocked on the first
message. The lean core (`src/lean/`, its own `core.db`) replaced that model;
the old runtime, its context reader, recovery/proof machinery and requirement
audits were deleted once the lean core handled "analyze the project
architecture" end to end.

Schema 11 provides [versioned requirements, tasks and evidence](requirement-ledgers.md),
run revisions, independent integrated-result evidence and TUI scope editing.
The historical notes below describe earlier milestones; the ledger document
defines the current completion and database compatibility contract.

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

The orchestrator plans from requirements without a global implementation-options
comparison. During execution, read-only `options` workers compare approaches
before their dependent writing tasks start. The runtime adds these dependencies
even when required quality gates are disabled.

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

Every agent is the provider's own interactive CLI running in a terminal (PTY)
that the daemon owns (`native.rs`, `terminal.rs`). A turn is the prompt typed
into that terminal (one bracketed paste, then Enter). There is one way back:
each session has a loopback MCP gateway (`mcp.rs`, bearer token per session)
with a single tool, `finish_turn(turn, text)`. The prompt ends with the turn's
id and the instruction to call the tool with the complete final answer; the
runtime hears nothing from the screen. A call for any turn but the open one is
refused with a message the model can read, so a stale or repeated call cannot
end a later turn. The gateway also tells the runtime that the CLI is up (it
asks for the tool list while starting, before it takes a prompt), which is when
the prompt is typed; a CLI that never connects (a trust question, a login) is
reported with the last lines of its terminal after 60 s, and a turn that is
not ended within 30 minutes (`ANTISLOP_TURN_TIMEOUT_SECS`) fails with the same
screen excerpt. A failure after the prompt was typed is never retried
automatically (the CLI may have acted); the raw reason stays visible and
queued mail waits for the next wakeup.

Nothing is configured in the CLIs' own files. The gateway and the per-role
permissions are passed on each launch (`launch.rs`):

| | read-only (orchestrator, verifier, analysis) | write (worker) |
|---|---|---|
| cwd | project (or the worktree being reviewed) | task worktree |
| claude | `--mcp-config … --strict-mcp-config`, default mode, `--disallowedTools Task,Agent,Edit,Write,NotebookEdit`, shell limited to `git status/diff/log/show`, `ls`, `cat`, `rg`, `grep`, `find`, `head`, `tail`, `wc` (anything else asks the person in the terminal) | `--permission-mode acceptEdits`, shell allowed |
| codex | `-c mcp_servers.antislop.url=… -s read-only -C <dir>` | `-s danger-full-access` |
| opencode | `OPENCODE_CONFIG_CONTENT` with the `antislop` agent and MCP server; `*` denied except read/search/web and the gateway | `*` allowed |
| agy | `--mode plan`, gateway through the stdio bridge below | `--mode accept-edits --sandbox` |

Claude's plan mode refuses MCP tools, so read-only Claude uses the default mode
without editing tools. Codex gets the token in the environment
(`ANTISLOP_MCP_TOKEN`), never in its arguments. Extra scope directories go
through `--add-dir`. Launcher commands (`["omlx", "launch", "opencode", …]`)
are kept as the prefix of the CLI's arguments.

Agy accepts MCP servers only from its own list. `antislop setup agy` (run once
by the user) registers `antislop mcp-bridge` there; each session passes its
gateway to the bridge through `ANTISLOP_MCP_URL`/`ANTISLOP_MCP_TOKEN`, and the
bridge forwards the CLI's JSON-RPC lines to it. Without that entry an Agy
session fails at once with the instruction to run the setup. Gemini has no
adapter yet and fails with a plain reason.

The CLI's own session identifier is the only thing kept to resume it: Claude is
given one (`--session-id`), Codex reports its thread in the call, Agy's bridge
adds its conversation, and for OpenCode (which reports none) the session created
in the directory since the process started and not owned by another live
session is taken, never guessed among several. A live CLI keeps its own
conversation across turns; a new round, a changed specification or a restart of
the orchestrator starts a new one, and after a daemon restart the same role
resumes with `--resume <id>` (`codex resume <id>`, `opencode --session <id>`,
`agy --conversation <id>`). Task workers and verifiers only resume within one
round. A new round gets fresh sessions and an explicit handoff from durable task
history.

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
   start a new worker round with a fresh session and explicit context. Supervised: after three rounds the task waits
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
- `run_contexts(turn_id, profile, cwd, add_dirs, access, backend, provider_thread_id,
  provider_session_id, provider_turn_id)` — one compact execution context per new
  turn, recorded before process launch. Conversation, role and provider come from
  the owning turn. Codex resume IDs are thread IDs; other CLI resume IDs are
  session IDs. Legacy turns have no inferred execution context.
- `tasks(id, conversation_id, plan_id, position, spec, status, round, report, verdict, base, result, created_ms)`
- `mail(id, conversation_id, sender, recipient, kind, text, created_ms, delivered_turn, delivered_ms)`
- `comments(id, conversation_id, plan_id, target, label, text, status, message, created_ms, updated_ms)`
- `events(seq, conversation_id, turn_id, kind, payload, created_ms)` — append-only; tool
  calls are stored as a name and a one-line detail, never their output.
- `pauses(conversation_id)` and `modes(conversation_id, autonomous)` — what
  the user set for a conversation.

`PRAGMA user_version` holds database schema version 11 (independent of the TOML
configuration's `schema_version = 1`). Only version 11 and genuinely empty
unmarked databases can open. Initialization is transactional. Older, newer or
unmarked nonempty files are rejected before DDL or journal changes. Stop the
daemon, manually archive `.antislop/`, then restart for a fresh state; the app
never archives or deletes it automatically. Restart client and daemon together.
History and execution context tables are created directly without backfills.
Per-conversation reads include global config events while keeping private events
scoped to their conversation. An agent's details show time when both ends are known.

The daemon's `run_context` request takes `conversation` and `turn`, returning the
context or `null` when none is recorded for that scope. Its backend is `cli` (the
interactive CLI; `codex_app_server` and `opencode_server` only appear in runs
recorded before the terminal model). Provider thread and session IDs are recorded
separately. A failure of an accepted turn disables automatic delivery retries; it
never resubmits the prompt. User text for an agent still goes through the durable
`send_to_agent` queue, role locks and round/run checks, and reaches the CLI as the
next prompt; what a person types in the terminal itself goes straight to the CLI.
The worker mailbox is checked under the integration lock before verification.

`terminal_open`, `terminal_read`, `terminal_input`, `terminal_resize` and
`terminal_detach` take a conversation and role. Opening returns a view handle;
subsequent requests must use that handle. Reads return viewport cells, cursor,
revision and attachment errors, or `null` for an unchanged revision. Input has
an increasing sequence starting at 1; duplicate sequences do not repeat keys.
If an input acknowledgement is lost, the frontend reconnects with the same
sequence. Unconfirmed text is recovered into the conversation/role's message
draft with the raw error and a request to check the terminal before resending;
subsequent queued text is recovered there as well. It is never submitted again
automatically. Closing a view drains its accepted input before detaching.
Detached or expired handles cannot send input. Detaching preserves the PTY and
unfinished CLI input. A session nobody uses or looks at for ten minutes is
released and resumed on demand. The role `ROLE#rN` names the session of round N:
opening it resumes that round's provider session read-only, which is the only way
to look at a past session. There is no transcript or JSON view of a session; the
provider keeps its own history. Process tracking lives under `.antislop/`; a
daemon restart interrupts in-flight turns and reaps its previous processes.

The emulator uses `portable-pty` and the published WezTerm terminal core. It has
no scrollback, bounds dimensions to 500×200, and filters graphics/string controls
before allocating terminal images. A grapheme adapter keeps combining marks and
ZWJ sequences together across reads and resize. Terminal query replies, keyboard
encoding, bracketed paste and mouse input use the emulator's terminal state.
Bounded OSC palette queries are preserved for native theme detection; image and
clipboard string controls remain filtered. PTY writer backpressure cannot stop
the emulator's writer thread or discard subsequent keystrokes.

Native Overview implementation and acceptance evidence are indexed in
[native-overview.md](native-overview.md).

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
  for a verifier, its risks, followed by each agent's own terminal), Messages (mail to and from it),
  Result (a worker's report and commits), Checks (each task obligation,
  risk and checklist item, with how the latest verdict accounted for it), Review (the
  plan verifier's and the auditor's latest verdict) and Comments. A dedicated conversation
  inspector opens via `Ctrl+I` or palette (`Ctrl+P`). Within inspectors, `/` searches whole records (`DetailRecord`) with query highlighting, and per-tab
  scroll and search memory is preserved across tab and conversation switches. Explicit notices state
  historical data gaps (`unavailable (CLI did not report them)`, `unknown`).
  Every Overview panel displays its agent's terminal; an earlier round shows a
  hint, and `o` resumes that round's session read-only. Two panels appear from
  80 columns; `Ctrl+W` changes the active panel outside terminal focus. `i` focuses
  the CLI and `Ctrl+]` returns to antislop. In terminal focus, Escape, Ctrl+C,
  paste and mouse events go to the CLI. Outside focus, `a` opens the existing
  message composer and `r` retries attachment.
  Existing drafts, search state, small-terminal guards, `NO_COLOR` and
  `ANTISLOP_ASCII` remain available.
  `/` in the graph finds an agent by role or task title; `@` on an empty draft or `Ctrl+P > Address a message`
  opens a recipient picker (`@orchestrator`, `@all`, `@worker:t1`, `@verifier:t1`), validated by the daemon.
  Ctrl+F searches the conversation (newest first; Enter brings the entry to the top, highlighted);
  Ctrl+P opens a palette of every command; `?` (F1 anywhere) shows the keys of the active view. `c` comments
  on the selected agent or sub-element with precise targets (`plan:`, `task:`, `check:`, `finding:`,
  `report:`, `artifact:`, `diff:`, `worker:`, `verifier:`, `conversation`); a box shows how many comments
  on it are open. On Comments, `r` resolves, `o` reopens and `d` deletes the selected one; a comment
  made on an earlier plan is marked so (`[applies to previous plan]`).
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
  to terminal coordinates. Bracketed paste normalizes CRLF/CR to LF, sanitizes control characters without clipping content,
  and routes paste according to modal ownership. The layout
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
- A failed prerequisite leaves its dependents queued. Recovery never discards their obligations.
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
  unfinished tasks remain queued with the reason. The orchestrator answers
  what it was answering, and both supervised and autonomous conversations
  resume previously accepted, unfinished plans. Explicit pauses remain effective.
- A task that stops short of approval (`needs_user`, `waiting`, `failed`,
  `cancelled`) says why in its event; the chat, the line mode and the task's
  Overview show the reason (design §8.4).
- Before writers start in worktrees, the user is told which files have
  uncommitted changes: writers include tracked and untracked local changes
  in their starting snapshot. Integration protects edits in the primary tree.
- Every worker and verifier is told to keep to the project's own
  instructions (AGENTS.md, CLAUDE.md, GEMINI.md…) whichever CLI it is, to
  name contradictions, and never to change its own permissions; a worker
  stops and says so when its goal needs something outside its task (design
  §10.3).
- A turn that fails because the CLI needs a login or hit a usage/rate limit
  leaves its task `waiting` (not `failed`), with the CLI's message.
- `resume` (Ctrl+R, `/resume`) runs again every task of a plan that ended
  without approval. Agents start fresh round sessions with their last report,
  findings and recorded change history; a writer continues in its worktree (or from its last result); a
  change that was approved but could not be integrated is integrated again
  without another worker turn.
- Tasks whose dependencies are approved run up to `policy.max_parallel` at
  a time, subject to write-scope isolation. Writers land one at a time; in-place
  writers also execute one at a time.
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

- Writers start from a snapshot of local user changes with all transitive
  dependency results composed in dependency order. Genuine conflicts are handed
  to the writer in an isolated worktree with the exact input commits and error.
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
  `danger-full-access` writer mode does not impose OS filesystem confinement.
- An interrupted verification is requeued with its task, so the worker
  gets a turn before the verifier starts a fresh session in the new round.
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

1. ✅ Store, agent adapters (codex, claude, opencode and agy as interactive CLIs
   in daemon terminals, ending turns through the MCP gateway; gemini has no
   adapter), conversation with the
   orchestrator, `antislop chat` CLI. Accepted on a real run: the orchestrator
   answered "analyze the project architecture" by reading the project itself;
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
| M0: integration spike | Versions and limits documented; a transport chosen per CLI | [Agents](#agents) (one interactive terminal and one MCP return channel for every CLI, resume by the CLI's own session id). gemini has no adapter. [m0/](m0/README.md) keeps the old runtime's first probes. |
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
creating project state. The `/config` modal edits every serialized user option
(excluding the fixed protocol constant `schema_version`),
optional overrides, command arrays and profiles, preserving TOML comments,
drafts on failure, and the running turn's profile snapshot. A shared global
file affects other projects when they next load it; active daemons remain
independent until updated or restarted.

Worker/verifier role addresses remain compatible with stored events and mail,
but map to one graph node. Overview pins a summary of at most three lines above the session card;
there is no separate Session tab. The card opens at the latest output and follows
new entries. Scrolling upward or `gg` pauses following; `G`/End resumes it.
`m` switches the card to the full task context. `i` edits a role-specific message.
Left/Right select a durable round, Tab/Shift+Tab select a tab, Ctrl+W selects a role,
and Enter folds the trace while leaving the full prompt and final answer visible.
The summary stays fixed and shrinks away on the smallest terminals. Checks invalidation handles round changes,
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
Prepared writes hold an advisory lock. After trust is checked, startup removes
unlocked, user-owned abandoned files for that exact configuration target,
without following symlinks or removing another daemon's active write. This test is process-exit
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

Checks uses the existing conversation-scoped `task_checks` operation for both
check events and task verdicts. It returns a full task snapshot with checks and
performs no Git freshness calculation. The TUI applies all task snapshots through
one `Update::Task` path, including reports and verdicts. On older daemons, only an
unsupported `task_checks` operation triggers the existing fallback to scoped
`task`. Authorization and other validation errors never trigger fallback.
A failed details request gets one immediate recovery attempt on the next event
batch, including an empty batch. Further failures back off exponentially from
one second to at most 30 seconds, and identical errors do not re-raise a dismissed
notice. Successful responses and changing the inspected task reset the backoff.
Task verdict events also invalidate an open Checks panel in the same round.


## Mail delivery and rendering costs

Receipt turns have their own cancellable jobs and never occupy the plan
scheduler's ownership slot. A role's normal turns and receipt turns are
serialized. Stopped workers/verifiers, the orchestrator, plan verifier and
auditor can acknowledge durable mail without approving tasks or proposing work.
A queued worker consumes its instructions in its next task turn.

Transient delivery errors receive up to three attempts with cancellable backoff.
Authentication failures retain the original message without a retry loop; a new
message or restart wakes the recipient again. Reply chains have a bounded budget
and leave remaining messages queued. Receipts are at least once, recorded only
after a successful turn. User copies for the orchestrator remain passive context.

Idle redraws reuse graph layout until an event, viewport, filter, grouping,
collapsed state, conversation generation or mail-highlight expiry changes it.
Line rasterization clips paths to the viewport; route/card intersection uses
interval bounds. The canvas remains O(viewport area). Session text/layout caches
and per-task Checks invalidation avoid unrelated repeated work. Recovery freshness
tokens still check Git state for recovery actions. The Checks tab uses the
conversation-scoped `task_checks` snapshot, which performs no Git fingerprint.
The ordinary `task` response retains a fresh recovery token. No claim of constant
total frame cost or host-overload resilience is made.


## t8 integration gate (2026-10-21)

The merge was checked against saved commits, without access to other worktrees.
Reproducible code-comparison evidence:

- `git diff aa13f6e0782289cca44a1d762486bd201dfd7cd6 5c32b3d -- src tests`
  shows only `src/lean/tui/details.rs` (4 additions, 30 deletions): `switch_tab`
  and `switch_key` use existing `save_current_tab`/`restore_current_tab` helpers,
  preserving scroll, query, follow, and pending_g reset.
- `git diff 58cfacbb1884b351aa4c595656459fa809c7f868 5c32b3d -- src tests`
  is empty: final code/tests match t7.
- `git show --stat 5c32b3d` confirms the previous round's existing commit,
  “Integration gate with requirements list (round 1),” touching only documentation
  and todo.md. The earlier statement “No commit was made” was imprecise: the agent
  did not invoke committing, but the round's outcome was recorded in Git.

The conflict was resolved by preserving t5 helpers and t4 behavior.
This revision changed only documentation/todo.md; src/tests were unchanged.

Six requirements — code/test evidence (test names locate results in the log):

| Requirement | Evidence |
| --- | --- |
| Global configuration when project configuration is absent | `src/lean/cli.rs::resolve_config`; `tests/lean_projects.rs::configuration_precedence_and_errors_without_creating_state` checks precedence, errors, and no created state. |
| Editing every configuration option in the TUI | `tests/lean_tui.rs::every_config_field_is_edited_and_written_to_the_selected_file` compares fields with the serialized schema and saves changes to local/global files. |
| Applying configuration live | `tests/lean_protocol.rs::live_config_updates_are_validated_and_preserve_daemon_identity` and `tests/lean_core.rs::next_turn_uses_live_profile_while_running_turn_keeps_its_profile`: new turns get the new profile, active turns retain the old one. |
| Text selection and copying | `tests/lean_tui.rs::text_selection_copies_rendered_graphemes_and_clears_on_content_change`, `text_selection_integrates_chat_agent_details_plan_and_modal_isolation`; `src/lean/tui/clipboard.rs` tests the 64 KiB limit and fallback after write/flush failure. |
| One worker/verifier node and Overview merged with Session | `src/lean/tui/graph.rs::task_node_key`, `tasks_sit_in_columns_by_dependency_depth_with_one_card_per_task`; `task_card_mouse_selection_uses_one_identity_for_both_phases`, `overview_includes_the_terminal_transcript_without_database_mirrors`, `overview_opens_at_latest_session_for_all_agent_roles`. |
| Current Checks across rounds | `tests/lean_tui.rs::checks_three_rounds_ignore_reverse_ordered_old_replies`, `checks_refetch_on_task_verdict_in_the_same_round`, `checks_retry_failed_fetch_on_next_empty_event_batch`, `persistent_checks_fetch_failure_backs_off_and_does_not_repeat_notice`. |

Implementation/assertions were read and compared with README, this document,
and todo.md. Descriptions of configuration, single nodes, Overview, copying, and
Checks match code. Checked TODO items mean implementation; they do not replace
manual terminal validation or full design.md fulfillment.

Manual checks in a real terminal:

- Drag/double-click selection, Unicode, Ctrl+Y, and pasting into another app.
  OSC 52 depends on the terminal; silent rejection cannot be detected.
- `/config`, editing, Ctrl+S, and reopening during an active session; new turns
  use changes without restarting the already working agent.
- One task node during worker/verifier phases; both transcripts in Overview.
- Open Checks across rounds, same-round verdict changes, retry after read errors;
  old responses must not roll the view backward.

Instruction constraints: AGENTS.md requires fmt, clippy, check, and tests;
standard Cargo commands were run. No additional CLAUDE, GEMINI, or CONTRIBUTING
instructions were found. Tool configuration/permissions were not changed.
During the first round, sandbox Git-index writes were denied (`index.lock:
Operation not permitted`), so commit comparison and a file-based merge were used
without changing the index.

First-round history: `/tmp/antislop-t8-verification.log` records 465/496 and exit
101, with 31 failures under socket/daemon/process-cleanup restrictions. This was
not a green result. The current run below supersedes the earlier conclusion that
a complete local gate was unavailable: the environment was provided without
those restrictions. The agent did not change environment permissions.

Current t8 result after verifier feedback — run and passed on `5c32b3d`
(only documentation/todo.md changed afterward):

| Command | Exit |
| --- | --- |
| `cargo fmt --check` | 0 |
| `cargo clippy --all-targets -- -D warnings` | 0 |
| `cargo check` | 0 |
| `cargo test --no-fail-fast` | 0 |

Complete run log: `/tmp/antislop-t8-final-verification.log`.
**496/496 tests passed, 0 failures, 0 skipped.** Suite results:
lib 271/271, lean_core 20/20, lean_messages 22/22, lean_plan 5/5,
lean_plan_history 4/4, lean_projects 3/3, lean_protocol 14/14,
lean_recovery 16/16, lean_review 10/10, lean_tui 101/101, lean_writer 30/30.
Main/doctests: 0 tests, exit 0. The complete run includes required lean_core and
lean_tui suites. Automatic comparison of all 31 earlier failure names against
the new log confirmed `ok` for each.

Below is execution evidence for the six-requirement matrix and write-interruption/
process-cleanup tests. Numbers identify lines in the new complete log.

| Test | Log line | Result |
| --- | ---: | --- |
| `configuration_precedence_and_errors_without_creating_state` | 383 | OK |
| `every_config_field_is_edited_and_written_to_the_selected_file` | 563 | OK |
| `live_config_updates_are_validated_and_preserve_daemon_identity` | 416 | OK |
| `next_turn_uses_live_profile_while_running_turn_keeps_its_profile` | 317 | OK |
| `text_selection_copies_rendered_graphemes_and_clears_on_content_change` | 521 | OK |
| `text_selection_integrates_chat_agent_details_plan_and_modal_isolation` | 556 | OK |
| `tasks_sit_in_columns_by_dependency_depth_with_one_card_per_task` | 168 | OK |
| `task_card_mouse_selection_uses_one_identity_for_both_phases` | 494 | OK |
| `overview_includes_the_terminal_transcript_without_database_mirrors` | 489 | OK |
| `overview_opens_at_latest_session_for_all_agent_roles` | 128 | OK |
| `checks_three_rounds_ignore_reverse_ordered_old_replies` | 462 | OK |
| `checks_refetch_on_task_verdict_in_the_same_round` | 463 | OK |
| `checks_retry_failed_fetch_on_next_empty_event_batch` | 467 | OK |
| `persistent_checks_fetch_failure_backs_off_and_does_not_repeat_notice` | 495 | OK |
| `abrupt_exit_before_and_after_config_publish_keeps_a_complete_file` | 54 | OK |
| `only_the_turns_own_process_group_is_reaped` | 56 | OK |
| `a_dead_daemons_agent_is_ended_before_its_turn_resumes` | 307 | OK |

Findings R1/V1/V5/C1/C3/R030/R046 now have evidence from the current green run,
including intermodule flows. R078: green lean_recovery 16/16 and interruption
checks before/after configuration publication. R098: green process-group and
orphaned-CLI cleanup tests. These results do not establish general resilience
against arbitrary host overload or correctness of every possible scenario.
The manual real-terminal checks above were not performed; they remain user-terminal
integration checks, especially OSC 52. No automated-gate execution blocker remains.

### Configuration editor structural actions

In `/config`, Ctrl+N adds an entry to the selected providers, profiles, or routing
section (other sections add a profile). Provider and route additions offer only
unconfigured kinds: Tab/Up/Down choose, Enter confirms, Esc cancels. New providers
use their CLI name as binary; routes use an existing profile for both roles,
with the orchestrator profile as executor for orchestration. Ctrl+D removes the
selected entry. Ctrl+S validates and applies the draft; invalid deletions leave
all draft edits intact and show field errors. Providers referenced by profiles,
required gate routes, and orchestration cannot be removed successfully. The
orchestration deletion check belongs to the editor; Config::validate is unchanged.

When a structural section is empty, Ctrl+N restores access: an empty provider
section takes priority; an empty routing section is offered outside providers.

### Integration gate t8 (2026-10-21)

The code remains at `3f4bd5eeb09dd1510e16905427a1493905f6b4ea`;
t8 changes documentation only. README's configuration editor instructions match
the key handling, draft validation and TOML patching implementation.

| Operation | Evidence in `tests/lean_tui.rs` |
| --- | --- |
| Add/remove a profile | `config_editor_acceptance::every_config_field_is_edited_and_written_to_the_selected_file`: actual SetConfig writes, reload and presence/absence assertions, for both project and global files. |
| Add/remove a provider | `configuration_provider_and_route_structural_edits_round_trip`: add, serialize, reload, delete, serialize and reload. |
| Add/remove a route | `configuration_provider_and_route_structural_edits_round_trip`: the same round trip for a non-required route. |
| Edit every user option | `config_editor_acceptance::every_config_field_is_edited_and_written_to_the_selected_file`: independently enumerates serialized fields, changes each value and checks persisted values; also clears optional overrides. `schema_version` is deliberately fixed. |
| Reject invalid deletions without losing edits | `configuration_invalid_structural_deletion_preserves_draft_and_marks_field`: referenced provider, required gate route and orchestration. |
| Recover empty sections / enumerate missing kinds | `configuration_empty_sections_can_be_restored`, `configuration_kind_picker_includes_every_missing_kind`. |

Executed in t8: `cargo test --test lean_tui configuration_` ran eight passing
tests and one environment-blocked test (exit 101). The blocked test,
`configuration_load_and_save_use_live_daemon_requests`, failed at socket binding
with `PermissionDenied: Operation not permitted`, before exercising the behavior.
Log: `/tmp/antislop-t8.3lecSu/structural-tests.log`.

Read in t8, without rerunning their daemon acceptance scenarios: the every-field
and profile persistence test above, configuration source, and README instructions.
The approved t7 handoff reports the complete suite on the same code outside the
sandbox: 502 passed, no failures or ignored/filtered tests; fmt, clippy and check
also passed. Its logs are at `/tmp/antislop-t7-rerun.E9NuAx/`; these are inherited
results, not a new unrestricted execution by t8. No manual terminal session or
performance benchmark was run in t8.

User decisions remain open: accept best-effort secret redaction; decide whether
to introduce coordination across processes for external configuration-file writers
(the daemon lock does not cover them); decide whether to require a performance
benchmark. This gate makes no new guarantees for these three limitations.

### Task rounds and provider sessions

Each worker and task verifier starts a fresh provider session in each round.
Mail follow-ups and verdict coverage corrections within that round reuse the
role's session, provided its provider, profile, directory and access still match.
A known resumed CLI identity is persisted before execution, so providers that
omit repeated session announcements do not break same-round continuity.
Compatibility also checks a SHA-256 fingerprint of the resolved provider,
profile name, executable, launcher arguments, model and effort. The fingerprint
is recorded in the existing `turn_started` event; changing settings under the
same profile name starts a new session with a full handoff. Missing fingerprints
in older task turns also start fresh. Terminal attachment refuses a task session
whose recorded execution settings differ from the current profile.
The persisted `turn_scopes`, `run_contexts` and execution event enforce this after daemon restart;
orchestrator, plan verifier and code review session policies are unchanged.
Native session rotation retires the old gateway, terminal and provider process.
Historical provider identifiers remain available: the provider keeps the transcript, and an earlier round's session is opened by resuming it.

Fresh task sessions receive the full task, latest complete worker report (without
truncation), previous verifier findings and evidence, dependencies, current commit
range and the sorted union of paths changed in earlier recorded round ranges.
This includes deletions, both sides of renames and paths subsequently restored.
Agents inspect the files and current diff themselves. An interrupted round with
no report falls back to the latest earlier report and explicitly directs the
agent to inspect uncommitted work. Completed worker reports are persisted before
mail follow-ups. Missing first reports are normal; content and Git read failures
are surfaced with their underlying cause. Integration repair uses the same
handoff alongside its conflict instructions, even when continuing a session.
Dependency collection reads only requested approved reports, without loading
unrelated task reports or dependency verdicts; team instructions read task metadata only.
The runtime gathers a typed task, dependencies, requirement entries, comments and protected
primary-tree paths; the prompt module formats them. In-place protection names
files the worker must not edit. Worktree protection identifies concurrent primary
changes to preserve during integration without forbidding the task's own edits.
A completed `agent_final` recovers the window before the scheduler saves a report;
receipt-only replies never replace task reports. Oversized context is sent whole;
a provider rejection is retained as a raw turn failure without shortening reports.
The handoff also names interrupted roles and their recorded failure reasons from
the previous round, including interruption after the worker report but before a verdict.

Offline tests cover native thread/session creation and gateway/process retirement
for both Codex and OpenCode, same-round continuity, configuration changes under
one profile name, restart before a report or verdict, long-context rejection,
historical rename/deletion/revert paths, and integration-repair handoffs.
No extra model call, configuration
flag, database schema or report archive is required.
