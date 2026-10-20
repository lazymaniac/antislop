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

## Use

Put an `antislop.toml` in the project (start from
[examples/antislop.toml](examples/antislop.toml)) and run:

```sh
antislop                         # TUI on the project's latest conversation
antislop --new                   # TUI on a new conversation
antislop "Review architecture"   # line mode, exits when idle
antislop --config FILE --state DIR   # the directory you start it in is the project
```

The first time you start antislop in a directory it asks whether you trust the
files there (agents read them, run commands and edit code). The answer is kept
per directory in `~/.config/antislop/trusted.json` (`$XDG_CONFIG_HOME` or
`ANTISLOP_CONFIG_DIR` move it), so it survives deleting `.antislop/`. Nothing is
created or started in the directory before you answer. Without a terminal, pass
`--trust` to answer yes.

Several projects can be open in one screen: `Ctrl+G` switches between them and
opens another by path (`/project PATH`); each keeps its own configuration,
`.antislop/` and daemon, and the header shows what the others are doing.

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

## Task cards and plan state

Every worker and verifier card is exactly 26 columns wide and 10 rows tall,
whatever it has to say, and every card keeps its fixed slots: title, state,
one activity line, up to four summary lines, two effect lines and the meta
line. All cards share that height, so with the plan, viewport, filters and
grouping unchanged no activity, effect, verdict, retry or late event ever
moves one card or changes one height. The card carries the same honest
reading of the task, built from what the runtime actually reported: the
round (`r1`, `r2`, …) once one has started and never before; a separate
activity line per role — what the agent is doing (`doing: …`), the concrete
dependencies a queued task waits for (`waits for t1`, `D:t1`) instead of a
guessed reason, or the raw cause when a task stops (`cause not reported`
when none was) — on its own line, independent of the state word and the
provider; the worker's effect once its round ends, with `[fragment]` shown
front-first when the text was cut; the verifier's obligation counters
(`2/3`, never `0` when nothing was reported); and one landing label for the
integration (`committed`, `applied in place`, `conflict`, `on branch`, …).
A worker may add the effect its card shows as a fenced ```effect block to
the very report it gives — one line, no backticks, closing fence on its own
line; the last such block wins, the runtime bounds it (never a full report
in events), and an empty or damaged block stays in the report while the
card falls back to the diff's summary (writers) or the report's first text
line (read-only), exactly as it does for reports without the block. The
carousel and details read state, activity, effect, coverage and integration
through the same single interpretation as the cards, and the carousel adds
the full effect, obligation and integration lines, the next step naming
only keys that really work, and plan-state counters over every task.
Details keep the whole cause and diff. Cards degrade to the same words in
fewer columns, and stay readable with `NO_COLOR=1` and `ANTISLOP_ASCII=1`.

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
