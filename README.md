# antislop

A local team of AI agents that works on your project through the CLIs you
already use (Codex, Antigravity `agy`, Claude Code, Gemini CLI, OpenCode). You talk to an
orchestrator; it answers, or proposes a plan of tasks. Each task is done by a
worker and checked by its own reviewer, and approved changes land on your
branch. The product specification is [design.md](design.md).

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

All agent roles launch with full filesystem and tool access. There is no directory
trust registry, execution approval or restricted launch mode. The runtime has no
agent-count, concurrency, execution-time, idle-time, memory, cost or retry budget.
Manual pause and cancellation remain available; roles, dependencies, independent
review and evidence still determine whether work is complete.

Runtime files live outside source checkouts. The root is `ANTISLOP_STATE_ROOT`,
else `$XDG_STATE_HOME/antislop`, else `~/.local/state/antislop`. Each project gets
`projects/<identity>` under that root. Linked Git worktrees share the identity of
their canonical common Git directory; non-Git projects use their canonical path.
Databases, logs, settings and checkpoints survive worktree cleanup. Long socket
paths use a short private directory under `/tmp`.

Configuration precedence is `--config FILE`, then the retained project runtime
configuration, then `./antislop.toml`, then the shared user configuration
(`ANTISLOP_CONFIG_DIR`, `$XDG_CONFIG_HOME/antislop`, or `~/.config/antislop`).
The selected source seeds the runtime copy; the editor updates that retained copy.
An explicit `--config` selects a new input. Invalid files report their raw error
without falling back or silently changing the active configuration. Resolving paths
creates nothing. Per-launch provider settings never modify a provider's own files.

In the graph each task is a frame titled with the task, holding a card for its
options agent (when the plan asks for options), its worker and its reviewer,
left to right; an options task's two agents share one card. Frames stand in
rows by dependency depth, and each dependency runs from the bottom of one frame
to the top of another. Every agent's runtime session remains separate. Overview shows each agent's own terminal: the provider's CLI,
running in the daemon, which you can use as usual (`i` gives it the keyboard,
`Ctrl+]` returns to antislop). An earlier round's session is opened on request
with `o`, by resuming it. Use `m` for the complete task context.
Checks refresh after each round and verdict, and live check events refresh only the
open task. Drag to select rendered text in chat, plan or agent details; double-click
selects a word and Ctrl+Y copies up to 64 KiB through OSC 52. On an OSC 52 write
failure, macOS uses `pbcopy` as a fallback. A terminal silently ignoring OSC 52
cannot be detected. Selection respects complete grapheme clusters.

`antislop stop` does not require a configuration file and succeeds if the daemon
is already stopped. It refuses to stop while agents are working unless `--force`
is passed. Forced shutdown cancels running agents and preserves conversation history.

Several projects can be open in one screen: `Ctrl+G` switches between them and
opens another by typing its path; each keeps its own configuration,
durable runtime directory and daemon, and the header shows what the others are doing.

## How a request runs

1. The orchestrator gathers the requirements and asks only for missing product decisions.
2. A dedicated planner drafts tasks with explicit `change_kind` (`none`, `artifact`,
   or `code`), goals and dependencies. These describe deliverables, not permissions.
3. Independent plan review checks coverage and dependencies. A checked plan starts
   automatically; rejected plans are corrected without a new execution approval.
4. Code tasks get dedicated local options and any configured quality gates.
   Workers deliver changes from isolated worktrees; independent tasks run concurrently.
5. Dedicated reviewers inspect results and evidence and may run checks. Corrections
   continue as needed. Missing progress produces a notice rather than a hidden stop limit.
6. Reviewed results are integrated while preserving user edits. Contract or dependency
   changes receive independent review; unavailable evidence is never invented.
7. An independent final audit checks acceptance before the request is reported complete.

The current formats are configuration **4**, daemon protocol **5**, and database
**16**. Old formats are rejected; there are no legacy readers or automatic migrations.
Archive an old runtime before starting a fresh session with the current build.

## Build and test

Rust 1.98.1 is pinned. `cargo.sh` prefers rustup when an asdf shim
shadows the selected toolchain.

```sh
./cargo.sh build
./cargo.sh test --all-targets
./cargo.sh clippy --all-targets -- -D warnings
```

Tests use small fake CLIs written in Python (`python3` must be on `PATH`); they
consume no provider usage.

Open the configuration with F8 (also in Ctrl+P) to edit the retained project runtime configuration.
Tab/Shift+Tab select fields; Enter cycles enum and boolean values. Arrays use JSON
string-array syntax, including command tokens. Empty optional fields remove overrides.
Ctrl+N adds an entry in the selected providers, profiles, or routing section (else a profile).
Profiles get generated names. For providers and routes, Tab/Up/Down choose an
unconfigured kind, Enter adds it, and Esc cancels. Providers default to their CLI
binary; routes default to existing profiles (orchestrator profile for orchestration).
Ctrl+D removes the selected provider, profile, or route. Invalid deletions show
field errors on save and retain the draft, including removal of orchestration.
Ctrl+S validates, saves, and applies the configuration live. Running turns finish
with their old profile. Errors retain the draft; Esc closes it and F8 reopens it.

Reopening refreshes the configuration. If another client changed it, the draft is
preserved and saving is blocked. Ctrl+R explicitly reloads and discards edits.
The daemon compares the loaded revision under its configuration-write lock, so a
second client cannot overwrite a newer daemon-mediated edit. Unrelated TOML
comments and formatting are preserved.

When a structural section is empty, Ctrl+N restores access: an empty provider
section takes priority; an empty routing section is offered outside providers.

## License

[MIT](LICENSE). Copyright (c) 2026 lazymaniac.
