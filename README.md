# antislop

A local team of AI agents that works on your project through the CLIs you
already use (Codex, Antigravity `agy`, Claude Code, Gemini CLI). You talk to an
orchestrator; it answers, or proposes a plan of tasks. Each task is done by a
worker and checked by its own verifier, and approved changes land on your
branch. The product specification is [design.md](design.md); how the runtime
works, what it guarantees and what is still open is in
[docs/lean-core.md](docs/lean-core.md); how each acceptance scenario of the
design is met, and which ones rest on agents' instructions rather than the
runtime, is in [docs/acceptance.md](docs/acceptance.md).

The library keeps the `architect` crate name.

## Use

Put an `architect.toml` in the project (start from
[examples/architect.toml](examples/architect.toml)) and run:

```sh
antislop                         # TUI on the project's latest conversation
antislop --new                   # TUI on a new conversation
antislop "przeanalizuj architekturę projektu"   # line mode, exits when idle
antislop --project DIR --config FILE --state DIR
antislop export > history.jsonl  # the conversation's history, secrets redacted
antislop eval evals --cli codex  # verifiers on seeded cases vs a plain review
```

`antislop eval` runs real CLIs on the cases in [evals/](evals) and compares
the verifier antislop gives a task with one agent asked plainly to review;
[docs/calibration.md](docs/calibration.md) reports what it measured.

The first call starts the project's daemon (state in `PROJECT/.antislop`), so
agents keep working when the frontend exits; the next call reattaches.

The TUI shows the chat beside the team graph: every agent as a box, edges for
dependencies, verification, messages and supervision (`f` filters them), and
an overlay with what is running right now. Tab moves between the panels; in
the graph, arrows select an agent, Enter opens its task, session, messages,
result, the checks its verifier accounted for and comments (`/` finds an
agent by name or task), and `c` comments on it: the orchestrator gets the comment
at once and keeps seeing it until you resolve it. `d` opens a task's change as
a diff with line numbers, where `c` comments on a line and `R` asks for a code
review whose findings go to the orchestrator.

Keys and editing:
- `Enter`: send message
- `Alt+Enter` or `Ctrl+J`: new line in draft
- `↑` / `↓`: vertical cursor movement across wrapped lines in draft (when draft is empty: scroll transcript)
- `PgUp` / `PgDn`: scroll conversation transcript
- `Ctrl+A`: accept/run proposed plan on empty draft; move to start of line (`Home`) when drafting
- `Ctrl+R`: resume unfinished plan on empty draft; search conversation history when drafting
- `Ctrl+U`: delete to start of line; `Ctrl+K`: delete to end of line
- `Ctrl+W` / `Alt+Backspace`: delete word backward
- `Ctrl+E`: move cursor to end of line; `Home` / `End`: line start / end (with `Alt`/`Ctrl`: buffer start / end)
- `Alt+Left` / `Ctrl+Left` and `Alt+Right` / `Ctrl+Right`: word navigation
- `Esc`: clear non-empty draft; when draft is empty, detach/exit (work continues in daemon)
- `Ctrl+X`: cancel running work; `Ctrl+N`: new conversation; `Ctrl+F`: search conversation; `Ctrl+P`: command palette
- `F1`: context-sensitive help in every view, including comment drafts and search lists. In the graph, details and diff, `?` also opens help; in text fields it types a question mark. Scroll help with `↑`/`↓`, `k`/`j`, `PgUp`/`PgDn`, `Home`, `End`; close it with `Esc`, `F1` or `?`.

Paste and Unicode semantics:
- Bracketed paste normalizes `\r\n` and `\r` to `\n`, sanitizes control characters, caps at 64 KB, and truncates strictly at UTF-8 character boundaries (`is_char_boundary`).
- Single-line paste (in search queries and palettes) flattens newlines to spaces and trims whitespace.
- All cursor navigation, text editing, line wrapping, and deletions operate strictly on UTF-8 code point boundaries, safely handling multibyte sequences (Polish diacritics, emoji) and double-width CJK ideographs (2 visual display columns) without character splitting or border overflow.
- Chat and comment drafts reserve a display column for the insertion cursor, so it stays after the final character of a full line. Multiline drafts keep their cursor and viewport position beyond 65,535 lines; long pasted search queries and activity labels are clipped to the available display width.
- Modals isolate key ownership: Help ignores typing, paste and action shortcuts until explicitly closed, hides the cursor beneath it and preserves drafts and search queries. Comment editor modal routes paste and key events to comment text. `Ctrl+C` detaches from any view.

Responsive layout and display:
- Minimum terminal size: 60x12 columns and rows; smaller viewports display a centered guard (`Terminal zbyt mały (wymagane min. 60x12)`).
- Breakpoint at 100 columns: `>= 100` columns renders side-by-side split (Chat and Team Graph); `< 100` columns renders single-panel compact mode with active panel toggle badge `[Czat]` / `[Zespół] (Tab: przełącz)`.
- Main footer action hints dynamically adapt to available terminal width (at 115+, 75+, 50+, <50 columns).
- Progress overlay dynamically moves between corners or hides to avoid collisions with the selected graph node.
- `NO_COLOR=1` disables ANSI color while preserving `dim` and `reversed` modifiers; `ANTISLOP_ASCII=1` (or non-UTF-8 locales) renders borders and status icons in plain ASCII.

Commands:
- `/pause` stops starting new work (running turns finish) until `/continue`.
- `/mode autonomous` or `/mode supervised` sets execution mode for subsequent plans (header displays mode). In line mode, type `/accept`, `/resume`, `/pause`, `/continue`, `/mode …` or `/cancel`.
- `@t2 text` sends text to the worker of task `t2` (delivered in its next turn; the orchestrator gets a copy). Agents message each other the same way.

An agent's details show its CLI, model and effort, tokens as the CLI reported
them (`niedostępne` when it did not), time spent in turns and, for a stopped
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
