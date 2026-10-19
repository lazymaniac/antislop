# antislop

A local team of AI agents that works on your project through the CLIs you
already use (Codex, Antigravity `agy`, Claude Code, Gemini CLI, OpenCode). You talk to an
orchestrator; it answers, or proposes a plan of tasks. Each task is done by a
worker and checked by its own verifier, and approved changes land on your
branch. The product specification is [design.md](design.md); how the runtime
works, what it guarantees and what is still open is in
[docs/lean-core.md](docs/lean-core.md); how each acceptance scenario of the
design is met, and which ones rest on agents' instructions rather than the
runtime, is in [docs/acceptance.md](docs/acceptance.md); the full TUI parity
matrix and verified contract evidence against `main@7ad664d25fbeebd3e8906caf29e9fb9925f20fc8`
is in [docs/parity.md](docs/parity.md).

The library keeps the `architect` crate name.

## Use

Put an `architect.toml` in the project (start from
[examples/architect.toml](examples/architect.toml)) and run:

```sh
antislop                         # TUI on the project's latest conversation
antislop --new                   # TUI on a new conversation
antislop "przeanalizuj architekturę projektu"   # line mode, exits when idle
antislop --config FILE --state DIR   # the directory you start it in is the project
antislop eval evals --cli codex  # verifiers on seeded cases vs a plain review
```

The first time you start antislop in a directory it asks whether you trust the
files there (agents read them, run commands and edit code). The answer is kept
per directory in `~/.config/antislop/trusted.json` (`$XDG_CONFIG_HOME` or
`ANTISLOP_CONFIG_DIR` move it), so it survives deleting `.antislop/`. Nothing is
created or started in the directory before you answer. Without a terminal, pass
`--trust` to answer yes.

`antislop eval` runs real CLIs on the cases in [evals/](evals) and compares
the verifier antislop gives a task with one agent asked plainly to review;
[docs/calibration.md](docs/calibration.md) reports what it measured.

OpenCode is supported directly (`binary = "opencode"`) or through oMLX:

```toml
[providers.opencode]
command = ["omlx", "launch", "opencode", "--model", "YOUR_MODEL", "--"]
isolation = "read_only"
```

Any CLI can also be started through a launcher of another platform, e.g. Codex
on local models, while the orchestrator keeps the plain `codex`:

```toml
[providers.codex]
command = ["ollama", "launch", "codex", "--"]   # or ["omlx", "launch", "codex", "--"]
isolation = "read_only"
```

The `--` ends the launcher's own options; antislop appends Codex's arguments
(`exec --json …`) exactly as it does without a launcher. Give such workers and
verifiers their own provider entry and profiles so the orchestrator is not
affected.

Set executor/verifier entries in `[role_providers]` and their profiles to
`"opencode"`, with profile `model = "omlx/YOUR_MODEL"`. The launcher model
avoids an interactive picker; the profile model pins each turn. Both CLIs must
be on `PATH`, and the oMLX server must already be running. This repository's
configuration uses OpenCode workers and verifiers with the existing Qwen
model, while Codex orchestrates. OpenCode runs with a dedicated agent:
writers can edit and use shell tools, read-only agents can read/search and
fetch web sources, and native subagents are disabled. Its
[JSON transport and session resume](https://docs.opencode.ai/docs/cli/)
use `run --format json` and `--session`; permissions are set through
[inline configuration](https://docs.opencode.ai/docs/config/).

The first call starts the project's daemon (state in `PROJECT/.antislop`), so
agents keep working when the frontend exits; the next call reattaches.

The TUI shows the chat beside the team graph: every agent as a box, edges for
dependencies, verification, messages and supervision (`f` filters them), and
an overlay with what is running right now. Tab moves between the panels (`[Chat]`,
`[Tasks]`, `[Team]`); in the graph, arrows select an agent, Enter opens its
inspector details full screen. The tabs depend on the agent: a worker has
`Overview`, `Session`, `Messages`, `Result` and `Comments`; its verifier has `Checks`
instead of `Result`; the plan verifier and auditor have a `Review` tab; the
conversation has `Overview`, `Messages` and `Comments`. The Session tab shows the
CLI's transcript (reasoning, commands and their output, answers) and follows the
newest entry until you scroll up. A dedicated conversation inspector opens
via `Ctrl+I` or palette (`Ctrl+P`). Within inspectors, `v` toggles between
Readable and Raw JSON view, `/` searches whole records with highlighting, `n`/`p`
paginates session history, and per-tab scroll and query memory is preserved across
tab switches and conversation switches. Explicit notices state historical data gaps
(`unavailable (CLI did not report them)`, `unknown`). `c`
comments on an agent or sub-element with precise targets (`plan:`, `task:`,
`check:`, `finding:`, `report:`, `artifact:`, `diff:`, `worker:`, `verifier:`,
`conversation`). `d` opens a task's change as a diff with line numbers and reviewer
profile, where `c` comments on a line and `R` asks for a code review.
Critical actions (accepting plans, resuming plans, reviewing diffs, cancelling)
present a pinned confirmation card (`ConfirmationCard`) with frozen `scope_hash`;
`Enter` or `y`/`Y` confirms, `Esc` or `n`/`N` rejects. If an asynchronous background
event alters the plan or scope before execution, the card transitions to `Stale`
(`[nieaktualne: stale_scope]`), and execution is safely blocked.

Keys and editing:
- `Enter` or `Ctrl+S`: send message (submit draft)
- `Alt+Enter` or `Ctrl+J`: new line in draft
- `↑` / `↓`: vertical cursor movement across wrapped lines in draft (preserving `preferred_col`); when on row 0 (or draft is empty), `↑` stashes the draft into `history_draft` and recalls sent message history (successive `↑` moves backward in history); when browsing history, `↓` on the last row moves forward to newer entries and restores `history_draft` with its exact cursor and column position
- `Esc`: when browsing history, cancels history recall and restores the in-progress draft; when drafting, clears a non-empty draft (note: pressing `↑` on a cleared draft starts history recall and overwrites the history draft with empty text, so a draft cleared by `Esc` is not retrievable via `↑`/`↓`); when draft is empty, detaches/exits from TUI (work continues in daemon). In modals, help, diff, details, and pickers, `Esc` dismisses the overlay and preserves underlying drafts and search queries without typing anything
- `PgUp` / `PgDn`: scroll conversation transcript
- `Ctrl+A`: accept proposed plan on empty draft; move to start of line (`Home`) when drafting
- `Ctrl+R`: resume unfinished plan on empty draft; search conversation history when drafting
- `Ctrl+U`: delete to start of line; `Ctrl+K`: delete to end of line
- `Ctrl+W` / `Alt+Backspace`: delete word backward
- `Ctrl+E`: move cursor to end of line; `Home` / `End`: line start / end (with `Alt`/`Ctrl`: buffer start / end)
- `Alt+Left` / `Ctrl+Left` and `Alt+Right` / `Ctrl+Right`: word navigation
- `@` on empty draft or `Ctrl+P > Adresuj wiadomość`: open recipient picker (`@orchestrator`, `@all`, `@worker:t1`, `@verifier:t1`), validated by daemon
- `Ctrl+X`: cancel running work; `Ctrl+N`: new conversation; `Ctrl+O`: switch conversation; `Ctrl+I`: conversation details inspector; `Ctrl+F`: search conversation; `Ctrl+P`: command palette
- `F1`: context-sensitive help in every view, including comment drafts and search lists. In the graph, details and diff, `?` also opens help; in text fields it types a question mark. Scroll help with `↑`/`↓`, `k`/`j`, `PgUp`/`PgDn`, `Home`, `End`; close it with `Esc`, `F1` or `?`.

Paste, Unicode, and draft recovery:
- Text editing, cursor navigation, wrapping, and deletions operate strictly on Unicode extended grapheme cluster boundaries (`unicode_segmentation`), safely handling combining characters (`a\u{301}`) and ZWJ emoji sequences (`👩‍💻`) without breaking glyphs or splitting UTF-8 bytes on `Backspace` or `Delete`.
- Unsent draft retention and recovery: network or daemon errors never lose text — drafts are automatically restored into `input` if empty, or preserved in `failed_send` if user started typing anew; conversation switching (`Ctrl+O`) isolates and preserves drafts per conversation (`A → B → A`); per-target comment drafts are remembered on modal close and restored when commenting on that target again.
- Bracketed paste normalizes `\r\n` and `\r` to `\n`, sanitizes control characters, caps at 64 KB, and truncates strictly at UTF-8 character boundaries (`is_char_boundary`).
- Single-line paste (in search queries and palettes) flattens newlines to spaces and trims whitespace.
- Chat and comment drafts reserve a display column for the insertion cursor, so it stays after the final character of a full line. Multiline drafts keep their cursor and viewport position beyond 65,535 lines; long pasted search queries and activity labels are clipped to the available display width.
- Modals isolate key ownership: Help ignores typing, paste and action shortcuts until explicitly closed, hides the cursor beneath it and preserves drafts and search queries. Comment editor modal routes paste and key events to comment text. `Ctrl+C` detaches from any view.

Responsive layout, carousel and adaptive team graph:
- Minimum terminal size: 12x5 columns and rows; smaller viewports display a centered guard (`Terminal too small (min. 12x5 required)`).
- Breakpoint at 85 columns: `>= 85` columns renders side-by-side split (Chat beside Team Graph or Tasks Carousel); `< 85` columns renders single-panel compact mode with clickable panel buttons `[Chat]` `[Tasks]` `[Team]` (`Tab` switches).
- Tasks Carousel (`t` key): dedicated full-screen or side-by-side panel for cycling through tasks with `←`/`→`, `h`/`l`, jumping to worker (`w`) or verifier (`v`), and quick actions (`Enter` for Details, `d` for Diff, `c` for Comment).
- Adaptive Graph Geometry: node layout dynamically adapts to actual interior dimensions of the graph panel (`inner.width`, `inner.height`), utilizing available columns and rows.
- Panel height utilization: taller viewports fit strictly more whole readable cards without scrolling (e.g. 20 -> 26 -> 46 lines).
- Compaction on folding and filtering: collapsing verifiers (`z`/Space) compacts vertical space to 0 lines, pulling subsequent tasks upward. Filtering admin nodes (e.g. `task 1`) reclaims row 0 for task cards. Column selection maps visible task depths sequentially starting at column 0 (`rel_depth`), eliminating empty preceding columns.
- Orthogonal edge routing with corridor bypass: non-adjacent nodes in the same column bypass intermediate cards through a side corridor lane (`from.right() + GAP_X / 2`), verified with 0 card interior intersections across 240 node pairs on a 4x4 grid and in all filter modes.
- Spatial navigation: arrow keys move strictly among visible nodes, skipping filtered or collapsed nodes.
- Stability: selecting a node or streaming activity updates never shifts or displaces graph nodes.
- Progress overlay collision avoidance: the 'now' progress box tests all four viewport corners and automatically yields (hides) if visible cards occupy candidate corners.
- Bounded Canvas memory: viewport windowing (`Canvas::new_window`) limits buffer allocation to `O(viewport_area)` (~99 KB–300 KB) regardless of graph size, preventing OOM crashes on large plans (tested on 500+ tasks).
- Main footer action hints dynamically adapt to available terminal width (at 115+, 75+, 50+, <50 columns).
- `NO_COLOR=1` disables ANSI color while preserving `dim` and `reversed` modifiers; `ANTISLOP_ASCII=1` (or non-UTF-8 locales) renders borders and status icons in plain ASCII.

### Graph screen utilization and layout measurements

| Terminal | Inner panel (`inner.width x inner.height`) | Whole visible cards without scroll | Columns & rows layout | Compaction & scroll impact | Edge routing safety (corridor bypass) |
|---|---|---|---|---|---|
| **80x24** (compact) | 78 x 20 | **6 cards** | 2 cols (x=3, 37) x 3 rows (y=0, 6, 12) | Fits completely in height 20 (`offset = (0, 0)`) | 0 collisions; Auditor bypassed via side corridor |
| **85x24** (split) | 48 x 20 | **3 cards** | 1 col (x=3) x 3 rows (y=0, 6, 12) | Admin nodes in col 0 without horizontal clipping | 0 collisions; clean vertical corridor routing |
| **120x30** (split) | 77 x 26 | **7 cards** | 2 cols (x=3, 37) x 4 rows (y=0, 6, 12, 18) | 4 row levels utilized; `offset = (0, 0)` | 0 collisions; Verifier bypassed via side corridor |
| **160x50** (split) | 104 x 46 | **11 cards** | 3 cols (x=3, 37, 71) x 7 rows | 100% of nodes visible without scrolling | 0 collisions across 240 node pairs on 4x4 grid |
| **Any terminal** | Any inner | — | Collapsed verifiers (`z`/Space) | 0 vertical blank lines (cards pulled upward) | Traces dynamically adjust to compacted coordinates |
| **Any terminal** | Any inner | — | Filtered tasks (e.g. "task 1") | Row 0 reclaimed, active depth mapped to col 0 | Routes restricted strictly to visible cards |

Commands:
- `/pause` stops starting new work (running turns finish) until `/continue`.
- `/unblock [task]` (or `u` on a selected step, or the `[Unblock]` button in the header) opens the way out for a step that stopped: retry it with a hint, give it more rounds, apply or keep its change, approve it as it is, or cancel it. The orchestrator is told once when a plan is blocked and explains the problem in the chat.
- `Ctrl+L` opens the plan document from any panel.
- `/mode autonomous` or `/mode supervised` sets execution mode for subsequent plans (header displays mode). In line mode, type `/accept`, `/resume`, `/pause`, `/continue`, `/mode …` or `/cancel`.
- `@t2 text` sends text to the worker of task `t2` (delivered in its next turn; the orchestrator gets a copy). Agents message each other the same way.

An agent's details show its CLI, model and effort, tokens as the CLI reported
them (`unavailable` when it did not), time spent in turns and, for a stopped
task, why it stopped.

## How a request runs

1. The orchestrator works in the project directory, read-only, with its CLI's
   own tools. It answers, asks, or proposes a plan: tasks with a goal, the
   directories in scope, dependencies, whether the task changes files, and what
   its verifier should focus on (including the risks of that task's domain).
2. An independent plan verifier checks the plan against your request and the
   project before you see it; a flawed plan goes back to the orchestrator.
3. `supervised`: you accept the plan. `autonomous`: a checked plan starts by
   itself, and the next one too, until a plan would repeat one that already
   ran.
4. Workers read and edit the project's files themselves. A task that changes
   files works in its own git worktree; the runtime commits its work there.
   With `workspace.mode = "in-place"` writers work in your directory itself,
   one at a time, and stop if they touch a file you were editing. A later
   writer can extend an earlier approved result without committing it, as
   long as that file still matches its latest approved version; your later
   edits and unapproved changes remain protected.
5. The task's verifier reads the result (and the diff) and must account for
   each obligation by id: the task's risks, the checks of every verification
   and the checklist for that kind of work (`ok`, `issue`, `blocked` or
   `n/a` with a reason). It approves or lists findings; findings go back to the
   same worker session (supervised: three rounds, then you decide;
   autonomous: until approval, or until the verifier repeats itself).
6. Approved changes are integrated into your branch one at a time, never over
   files you are editing and never with conflict markers.
7. An independent auditor checks that every part of your request was
   delivered; the orchestrator receives the results and the audit and
   answers you, or proposes a plan for what is missing.

## Build and test

Rust 1.98.1 is pinned. `scripts/cargo.sh` prefers rustup when an asdf shim
shadows the selected toolchain.

```sh
scripts/cargo.sh build
scripts/cargo.sh test --all-targets
scripts/cargo.sh clippy --all-targets -- -D warnings
```

Tests use small fake CLIs written in Python (`python3` must be on `PATH`); they
consume no provider usage.
