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

Configure `antislop.toml` in the project or in the global configuration directory
(start from [examples/antislop.toml](examples/antislop.toml)) and run:

```sh
antislop                         # TUI on the project's latest conversation
antislop --new                   # TUI on a new conversation
antislop stop                    # stop the idle daemon (--state DIR supported)
antislop stop --force            # cancel running agents and stop the daemon
antislop "Review architecture"   # line mode, exits when idle
antislop --config FILE --state DIR   # the directory you start it in is the project
```

The first time you start antislop in a directory it asks whether you trust the
files there (agents read them, run commands and edit code). The answer is kept
per directory in `~/.config/antislop/trusted.json` (`$XDG_CONFIG_HOME` or
`ANTISLOP_CONFIG_DIR` move it), so it survives deleting `.antislop/`. Nothing is
created or started in the directory before you answer. Without a terminal, pass
`--trust` to answer yes.

The internal daemon entrypoint checks the same user-owned trust registry before
opening project state. Trust stored in a project database does not grant access.
Registry updates use an exclusively created temporary file and an atomic rename.

Configuration precedence is `--config FILE`, then `./antislop.toml`, then
`$ANTISLOP_CONFIG_DIR/antislop.toml`, `$XDG_CONFIG_HOME/antislop/antislop.toml`, or
`~/.config/antislop/antislop.toml` (the first configured global directory).
An invalid or inaccessible project file reports an error instead of falling back.
Resolving a configuration creates nothing. The editor saves to the selected file;
when using a shared global file, saving also affects future starts of other projects.

Each task has one graph node for its worker and verifier; their runtime sessions
remain separate. Overview pins a compact task summary above a session card containing both transcripts.
The card follows new output; `gg` goes to the start and `G` returns to live output.
Use `i` for the complete task context.
Checks refresh after each round and verdict, and live check events refresh only the
open task. Drag to select rendered text in chat, plan or agent details; double-click
selects a word and Ctrl+Y copies up to 64 KiB through OSC 52. On an OSC 52 write
failure, macOS uses `pbcopy` as a fallback. A terminal silently ignoring OSC 52
cannot be detected. Selection respects complete grapheme clusters.

`antislop stop` does not require a configuration file and succeeds if the daemon
is already stopped. It refuses to stop while agents are working unless `--force`
is passed. Forced shutdown cancels running agents and preserves conversation history.

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

Open `/config` (also in Ctrl+P) to edit the active project or global configuration.
Tab/Shift+Tab select fields; Enter cycles enum and boolean values. Arrays use JSON
string-array syntax, including command tokens. Empty optional fields remove overrides.
Ctrl+N adds a profile with a generated name; Ctrl+D removes the selected profile.
Ctrl+S validates, saves, and applies the configuration live. Running turns finish
with their old profile. Errors retain the draft; Esc closes it and `/config` reopens it.

Reopening refreshes the configuration. If another client changed it, the draft is
preserved and saving is blocked. Ctrl+R explicitly reloads and discards edits.
The daemon compares the loaded revision under its configuration-write lock, so a
second client cannot overwrite a newer daemon-mediated edit. Unrelated TOML
comments and formatting are preserved.
