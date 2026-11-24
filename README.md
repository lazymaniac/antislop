# antislop

A local team of AI agents that works on your project through the CLIs you
already use (Codex, Antigravity `agy`, Claude Code, Gemini CLI, OpenCode, Pi). You talk to an
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

## Pi and Unsloth

Install and configure [Pi](https://github.com/earendil-works/pi) yourself,
including its runtime, provider credentials or local backend and available models.
The inspected Pi 1.5.2 package requires Node.js >=22.19.0.
For the wrapper, also install [Unsloth](https://github.com/unslothai/unsloth),
configure its server and download the model yourself. Both executables must be on
PATH (or use an executable path in `command`). antislop installs neither tools nor
models and does not edit their configuration. The adapter was exercised with Pi
API 1.5.2; it requires the extension API `registerMcpServer`. Other versions and
arbitrary wrappers are not guaranteed compatible.

Check your standalone tools first:

```sh
pi
unsloth start pi --model unsloth/Qwen3.8-Flash-Next-GGUF:UD-Q4_K_XL --context-length 200000 --reasoning auto
```

Add these tables to a copy of [examples/antislop.toml](examples/antislop.toml).
They are fragments; retain its schema, policy and complete routing tables:

```toml
[providers.pi]
binary = "pi"

[profiles.pi_direct]
provider = "pi"
# Omit model to use Pi's configured default; otherwise use an installed model ID.
# model = "provider/model-id"
effort = "high"

[profiles.pi_unsloth]
provider = "pi"
command = ["unsloth", "start", "pi", "--model", "unsloth/Qwen3.8-Flash-Next-GGUF:UD-Q4_K_XL", "--context-length", "200000", "--reasoning", "auto"]
effort = "high"
```

The wrapper is a command of a **Pi profile**, not a separate provider. Array
entries are individual argv tokens, without shell interpretation. A profile
`command` replaces the provider command and provider arguments; only that
profile's `args` are appended. The wrapper example already supplies `--model`:
do not add a second one in `args` or a different profile `model`. Multiple model
arguments or conflicting values fail. antislop preserves the configured prefix
and adds its own extension and session arguments. Use `start pi` as shown.

For direct Pi, profile `model` adds `--model` and `effort` accepts `low`, `medium`,
`high` or `xhigh`, passed as Pi `--thinking`. For models Pi marks as non-reasoning,
Pi uses `off`; antislop accepts this and records the effective level in its launch
status. Model support depends on your backend.
Omitting `model` leaves model selection to the CLI. `effort` is required by the
configuration schema. Unsloth `--reasoning auto` is a separate backend setting; it does not set Pi thinking. The wrapper example
uses `effort = "high"` as an explicit, independent Pi setting, adding
`--thinking high`; this addition is not part of the original shell command and is
not equivalent to `--reasoning auto`. If an Unsloth server is already running,
the observed launcher warns that it cannot send `--reasoning auto` to that server: configure
the server yourself rather than assuming this flag changed it. The Pi API checks
the Unsloth provider and repository ID, but cannot independently attest the GGUF
quantization or effective inference settings. The exact command above has been
exercised for turns, cancellation and JSONL resume; this is not a guarantee for
all model/backend combinations.

Save the tables in `antislop.toml`; reopening with `antislop --config antislop.toml`
loads them. In the TUI configuration, Pi is available in the provider choices;
Ctrl+S validates/applies and Ctrl+R reloads. Assign profile names to existing
routes, for example change `[routing.implementation]` to
`executor = "pi_unsloth"`, `reviewer = "pi_direct"`. Either profile can also serve
planning or review through the corresponding routes. To orchestrate with Pi,
set `[orchestrator].profile = "pi_direct"` and
`[routing.orchestration].executor = "pi_direct"` together. Reviewer sessions stay
separate even when they use the same profile.

`antislop doctor` and plan preflight check that the configured Pi or wrapper
executable is available. The native session then verifies the extension, actual
session settings and authenticated Gateway connection before submitting a prompt;
adapter and backend failures retain their raw diagnostic. `init` can discover `pi`
on PATH, but does not auto-create the Unsloth wrapper profile. Executable discovery
alone does not establish backend readiness.

Pi runs in the assigned working directory through the native PTY. antislop
supplies its extension and per-launch authenticated MCP Gateway without a global
Pi MCP configuration. Keep runtime-controlled session, mode, thinking and MCP
flags out of `command`/`args` (including `--session`, `--resume`, `--continue`,
`--no-session`, `--mode`, `--print`, `--thinking`, `--no-mcp` and `--`); use profile
fields for model/thinking. The launcher rejects reserved tokens explicitly.

Your wrapper's own session configuration (backend, context length, reasoning,
credentials and defaults) remains your responsibility; repeat required launch
settings in each wrapper profile. This is separate from antislop conversation
history. Pi JSONL files live under the task runtime's `pi/sessions` outside the
checkout, within the durable state root described below. Compatible processes
are reused across turns; restarting resumes the recorded JSONL path. Missing,
corrupt or unsupported history fails explicitly instead of silently starting an
empty session. Cancellation waits for Pi to confirm idle before another turn.

For failures, first read the raw CLI error and terminal tail in Workbench. Check
PATH, standalone startup, backend/authentication and model availability. Adapter
errors identify missing `registerMcpServer`, actual cwd/model/thinking mismatches,
invalid resume history or MCP transport failure; `[pi_configuration]` identifies
configuration failures. Keep the durable state and session file when diagnosing
resume problems. A successful executable/version check alone does not establish
a working Gateway or inference backend.

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

The TUI opens the Canvas. Cards represent work: orchestration, planning and plan
review, tasks with their dedicated options and applicable reviewer stages, and
final audit. Shared options and work without an established owner stay separate.
Tab selects an underlying stage in the inspector; Enter opens that stage's
Workbench. Each stage retains its own history, results, evidence, checks,
comments, recovery actions, draft, and native terminal. Agents without known file
changes initially show their PTY; selecting a tab opens content. Reviewer Summary
shows acceptance checks and their evidence. Conversation is the last tab and
shows the agent's recorded prompt and message exchange as a scrollable chat.
Prompts are retained for new turns; prompts not recorded by older runtimes cannot
be reconstructed. The orchestration inspector shows full recent messages, with
wheel and Shift+PgUp/PgDn scrolling independent of the graph.

| Context | Controls |
|---|---|
| Navigation | Ctrl+P actions; Ctrl+K composer; Ctrl+T session Workbench; F1 help |
| Sessions and projects | Ctrl+N new; Ctrl+O sessions; Ctrl+G projects |
| Workflow | `d` decisions; `h` plan history; Ctrl+L plan; Space pause/continue; Ctrl+X cancel |
| Canvas | arrows select; Enter Workbench; Tab/Shift+Tab stages; `/` search; `f` filters; `g` grouping; 1–4 relations; `z` fold/expand |
| Workbench | 1–9 tabs; Left/Right or Shift+Left/Right previous/next tab; Shift+Up/Down agents; Tab/Shift+Tab panes; `,`/`.` rounds; Alt+Left/Right result documents |
| Presentation | Ctrl+B inspector/sidebar; `p` pin terminal; `z` zoom; Alt+Up/Down resize |
| Native PTY | `t` enables input; Ctrl+] returns to navigation; Shift+Up/Down switches agents; other keys go to the CLI |
| Composer | displayed, changeable recipient; Ctrl+R recipients/broadcast; Ctrl+T queue/steer/interrupt; Enter send; Alt+Enter/Ctrl+J newline; Esc retains draft |
| Editing and exit | editors and modals own their keys; Ctrl+C exits outside native input; Ctrl+Y copies selected application text |

At 160×50 the Canvas inspector is 36 columns and the Workbench sidebar is 30
columns, with equal content and PTY shares. Below 120 columns secondary panes
hide until requested with Ctrl+B, the Canvas **inspector** control, or the Workbench
**Tasks** control. Below 90 columns these panels use the full width; selecting a
task or pressing Enter returns to its content. Workbench tab labels are clickable,
and **Pane** cycles focus with the current pane named in the heading.
Below 30 rows Tab switches content and terminal
instead of splitting. Layout preferences survive resizing. New decisions update
badges; open full questions and approvals with `d` or the inspector. Ctrl+P keeps
plan acceptance/rejection/revision, task editing/addition/splitting/holding/retirement,
dependency linking, direct execution, rigor, recovery, approval mode, takeover,
configuration and completion actions available.

A proposed plan opens for review on Canvas when no editor, dialog or native
terminal owns input. After dismissal, **Review plan** remains above the direction
bar and opens the same plan with its comment, review and acceptance controls.
Ctrl+P uses a compact, borderless list; typing filters all available actions.

The top bar shows `> antislop <branch>` and the session's status and total work
time. The clock counts overlapping agent turns once, excludes idle gaps, and
restores its total from recorded events when the TUI is reopened.

Ctrl+K turns the bottom direction bar into an inline editor. It grows for
multiline text; Enter sends and Esc restores navigation while keeping the draft.
Questions and available session actions appear above it. Click an answer to
select it, then Enter to submit; choose Other to type in the same bottom input.
Long scope questions retain PgUp/PgDn and wheel scrolling. New questions never
take keyboard focus from an editor or native terminal.

Configuration uses Shift+Left/Right sections, Up/Down fields, Enter choices,
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
2. The planner calls Research, Analysis, Dependencies, Architecture and Test Design
   before drafting, then uses their retained findings to draft tasks with explicit `change_kind` (`none`, `artifact`,
   or `code`), goals, dependencies and task-local acceptance criteria. These describe
   deliverables, not permissions.
3. Independent plan review checks coverage and dependencies. You accept the checked
   plan before it runs; rejected plans are corrected and reviewed again.
4. Code tasks get local Options and quality gates defined by the selected rigor.
   Workers deliver changes from isolated worktrees; independent tasks run concurrently.
5. Dedicated reviewers inspect results and evidence and may run checks. Corrections
   continue as needed. Missing progress produces a notice rather than a hidden stop limit.
6. Reviewed results are integrated while preserving user edits. Contract or dependency
   changes receive independent review; unavailable evidence is never invented.
7. One independent final audit checks the integrated result against the scope, original
   sources and task criteria before the request is reported complete.

Agents reuse successful builds, tests and static analysis when the command,
covered targets, declared environment and source files match. Validation receipts
include uncommitted/untracked source changes, retained raw evidence and execution
ownership, so successors do not rerun identical checks or start parallel duplicates.
Failures, changed inputs and invalid evidence require fresh execution. Reviewers
still inspect implementation and acceptance coverage independently.

Scope changes appear alongside the reviewed plan for your acceptance. Technical
revisions within the accepted scope keep independent review. There is no separate
requirements extraction, ledger browser or ledger approval step.

This version uses runtime database schema 17. Older databases are rejected without
modification or migration. Stop the daemon, archive the project runtime directory
containing `core.db`, and restart both client and daemon to create fresh state.

## Feature rigor and configuration

The planner calls Research and impact Analysis before drafting. Dependencies uses
Research; Architecture uses Analysis and Dependencies; Test Design uses Analysis
and Architecture. Reports are retained and reused for corrections and replans when
requirements stay the same. The plan cites their references and maps requirements
to implementation owners and observable checks. Workers consume those findings;
completed preparation is not scheduled again as execution tasks.

Each implementation writer gets its own Options selection and review, with focused
tests alongside the code. Feature rigor adds six final checks: Refactoring, parallel
Static Analysis, Testing, Security and Performance, then Integration. Final Delivery
reconciles requirements, observed results, usage and limitations after all deliverables
and checks. These checks verify the implemented result rather than repeating design.

Configuration now requires `schema_version = 5`. Remove `policy.required_gates` and
`policy.local_options`; the selected rigor alone defines gates and Options.
`policy.local_commits` is unchanged. Existing configurations fail with migration
instructions and are never rewritten automatically. Keep routes for the specialist
kinds as shown in `examples/antislop.toml`.

Accepted older graphs remain unchanged on restart. Selecting Feature for an existing
graph queues a reviewed replan when its delivery or final checks are missing, preserving
approved results and retained checkpoints.

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
