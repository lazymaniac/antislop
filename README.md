# antislop

A local team of AI agents that works on your project through the CLIs you
already use (Codex, Antigravity `agy`, Claude Code). You talk to an
orchestrator; it answers, or proposes a plan of tasks. Each task is done by a
worker and checked by its own verifier, and approved changes land on your
branch. The product specification is [design.md](design.md); how the runtime
works, what it guarantees and what is still open is in
[docs/lean-core.md](docs/lean-core.md).

The library keeps the `architect` crate name.

## Use

Put an `architect.toml` in the project (start from
[examples/architect.toml](examples/architect.toml)) and run:

```sh
antislop                         # TUI on the project's latest conversation
antislop --new                   # TUI on a new conversation
antislop "przeanalizuj architekturę projektu"   # line mode, exits when idle
antislop --project DIR --config FILE --state DIR
```

The first call starts the project's daemon (state in `PROJECT/.antislop`), so
agents keep working when the frontend exits; the next call reattaches.

TUI keys: Enter send, Alt+Enter new line, Ctrl+A run the proposed plan,
Ctrl+X cancel running work, PgUp/PgDn scroll, Ctrl+N new conversation, Esc
detach. In line mode, type `/accept` or `/cancel`.

## How a request runs

1. The orchestrator works in the project directory, read-only, with its CLI's
   own tools. It answers, asks, or proposes a plan: tasks with a goal, the
   directories in scope, dependencies, whether the task changes files, and what
   its verifier should focus on (including the risks of that task's domain).
2. You accept the plan (supervised policy).
3. Workers read and edit the project's files themselves. A task that changes
   files works in its own git worktree; the runtime commits its work there.
4. The task's verifier reads the result (and the diff) and approves or lists
   findings; findings go back to the same worker session, for a bounded number
   of rounds.
5. Approved changes are integrated into your branch one at a time, never over
   files you are editing and never with conflict markers.
6. The orchestrator receives the results and answers you.

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
