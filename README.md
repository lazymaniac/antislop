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
