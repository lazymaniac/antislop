# antislop

A local team of AI agents that works on your project through the CLIs you
already use (Codex, Antigravity `agy`, Claude Code, Gemini CLI, OpenCode). You talk to an
orchestrator; it answers, or proposes a plan of tasks. Each task is done by a
worker and checked by its own reviewer, and approved changes land on your
branch.

## Install

Prebuilt binaries for macOS and Linux (arm64, x86_64) are attached to each
[release](https://github.com/lazymaniac/antislop/releases); every archive has a
`.sha256` next to it. With Homebrew:

```sh
brew install lazymaniac/tap/antislop
```

From source (Rust 1.98):

```sh
cargo install --locked --git https://github.com/lazymaniac/antislop
```

antislop drives the agent CLIs you have installed and logged in to; it installs
none of them.

## First run

```sh
cd your-project
antislop init      # writes antislop.toml from the CLIs found on PATH
antislop doctor    # checks the configuration and that every profile's CLI launches
antislop           # opens the TUI
```

`antislop init` routes all work to the CLIs it finds; edit `antislop.toml` to
pick models, efforts and which CLI does which kind of work (see
[examples/antislop.toml](examples/antislop.toml)). A plan never starts while a
CLI it needs cannot be launched: the refusal names the profiles that would.

## Use

Configure `antislop.toml` in the project or in the global configuration directory
(start from [examples/antislop.toml](examples/antislop.toml)) and run:

```sh
antislop                         # TUI on the project's latest conversation
antislop --new                   # TUI on a new conversation
antislop doctor                  # configuration, state directory and CLI checks
antislop stop                    # stop the idle daemon (--state DIR supported)
antislop stop --force            # cancel running agents and stop the daemon
antislop activate --binary FILE  # check and activate a new runtime after active work finishes
antislop activate --status       # inspect the durable activation result
antislop "Review architecture"   # line mode, exits when idle
antislop --config FILE --state DIR   # the directory you start it in is the project
```

The default TUI opens the Graph with chat and the selected card's inspector.
Enter opens the selected agent in Workbench; Ctrl+T opens the session Workbench.
Worker deliveries start on Diff; other roles use their available tabs. The
Workbench keeps each target's tab, round, scroll, query and draft independently.

| Context | Keys |
| --- | --- |
| Graph | arrows select cards; Enter opens Workbench; Ctrl+B toggles inspector; `/` searches; `f` filters; `g` groups; 1–4 toggle relation types; `z`/Space folds cards; `d` opens task Diff |
| Workbench | Ctrl+Left/Right changes tabs; Tab/Shift+Tab changes content/terminal focus; Ctrl+B toggles list; `z` zooms; Alt+Up/Down resizes the split; `p` pins the terminal owner |
| Workbench history | `,`/`.` selects rounds; newest follows live; Alt+Left/Right selects result documents; historical PTY is read-only and unavailable snapshots are explicit |
| Native PTY | focus terminal, `i` enters native input; Ctrl+] returns to Workbench navigation; every other key goes to the CLI, including Ctrl+C and function keys |
| Agent composer | `a` opens draft; Ctrl+T cycles queue/steer/interrupt; Enter sends; Esc returns without discarding draft |
| Global frontend navigation | F1 help, F2 decision inbox, Ctrl+P palette, Ctrl+T session Workbench (Ctrl+I where distinct from Tab), Ctrl+F search, Ctrl+K session composer, Ctrl+Y copies selected application text |
| Global workflow keys | Ctrl+N new conversation, Ctrl+O conversation picker, Ctrl+G project picker, Ctrl+L plan, Ctrl+Enter / Ctrl+M rigor (Ctrl+M if distinct from Enter), F3 interview, F4 plan history, F5 task actions, F6 pause/continue, F7 reject plan, F8 configuration, Ctrl+X cancel |
| Exit | Esc leaves the active editor/modal or zoom first; Ctrl+C quits the frontend outside native PTY; daemon work continues |

Ctrl+R resumes the plan or opens history in Chat/Graph. Ctrl+W switches the
worker/reviewer target in Workbench. Comment/agent editors and forms own their
editing keys; Ctrl+K in main chat opens the session composer. Ctrl+P also exposes workflow actions; F1 shows contextual controls. In native PTY, leave input with
Ctrl+] before using application shortcuts. Graph contains no native PTY.
Configuration uses Ctrl+Left/Right sections, Up/Down fields, Enter choices,
Ctrl+N/Ctrl+D add/delete, Ctrl+S validate/apply and Ctrl+R refresh while retaining
a dirty draft. Chat sends with Enter/Ctrl+S; Alt+Enter/Ctrl+J inserts a newline,
Ctrl+V attaches a clipboard image and Alt+Backspace removes the latest attachment.

Runtime files live outside source checkouts. The root is `ANTISLOP_STATE_ROOT`,
else `$XDG_STATE_HOME/antislop`, else `~/.local/state/antislop`. Each project gets
`projects/<identity>` under that root. Linked Git worktrees share the identity of
their canonical common Git directory; non-Git projects use their canonical path.
Databases, logs, session settings and checkpoints survive worktree cleanup. Long socket
paths use a short private directory under `/tmp`.

Runtime activation preserves the daemon's environment and merges the requesting
client's absolute PATH entries. The target checks its configuration, required CLIs
and database compatibility before a handoff is queued and again before switching.
Current turns finish normally; new tasks wait until the successor confirms readiness.
The handoff is durable and runs inside the daemon, without a background helper service.
When activation is part of a task, pass `--conversation ID --task ID` (and the same
`--config FILE --state DIR`); that task waits for confirmation and independent review
before its dependents run. Failures retain their raw cause and existing checkpoints.

Several terminals may follow one conversation; the first to act on it steers
and the others watch until they take control from the palette.

## How a request runs

1. The orchestrator gathers the scope and asks only for missing product decisions.
   The confirmed scope and original sources stay attached to the plan.
2. A dedicated planner drafts tasks with explicit `change_kind` (`none`, `artifact`,
   or `code`), goals, dependencies and task-local acceptance criteria. These describe
   deliverables, not permissions.
3. Independent plan review checks coverage and dependencies. You accept the checked
   plan before it runs; rejected plans are corrected and reviewed again.
4. Code tasks get dedicated local options and any configured quality gates.
   Workers deliver changes from isolated worktrees; independent tasks run concurrently.
5. Dedicated reviewers inspect results and evidence and may run checks. Corrections
   continue as needed. Missing progress produces a notice rather than a hidden stop limit.
6. Reviewed results are integrated while preserving user edits. Contract or dependency
   changes receive independent review; unavailable evidence is never invented.
7. One independent final audit checks the integrated result against the scope, original
   sources and task criteria before the request is reported complete.

Scope changes appear alongside the reviewed plan for your acceptance. Technical
revisions within the accepted scope keep independent review. There is no separate
requirements extraction, ledger browser or ledger approval step.

This version uses runtime database schema 17. Older databases are rejected without
modification or migration. Stop the daemon, archive the project runtime directory
containing `core.db`, and restart both client and daemon to create fresh state.

## Build and test

```sh
./cargo.sh build
./cargo.sh test --all-targets
./cargo.sh clippy --all-targets -- -D warnings
```

## Release

Tag `vX.Y.Z` after adding its section to [CHANGELOG.md](CHANGELOG.md); the
release workflow builds and publishes the archives. Then regenerate the tap's
formula with `packaging/homebrew-formula.sh vX.Y.Z`.

## License

[MIT](LICENSE). Copyright (c) 2026 lazymaniac.
