# Agent sessions in Overview

Every agent is the provider's own interactive CLI, running in a terminal that
the daemon owns. Overview shows that terminal; the person can use the CLI as
usual (answer a question, change its model or mode, run a slash command) and the
runtime keeps working through it. There is no JSON, transcript or event view of
a session: the provider keeps its own history, and an earlier session is opened
by resuming it.

## How a turn runs

1. The daemon starts the CLI in a PTY with its gateway, permissions and
   working directory passed on the command line or in the environment
   (`src/lean/launch.rs`). Nothing is written to the CLIs' own configuration.
   The CLI's environment carries an `ANTISLOP_AGENT` marker that every command
   it starts inherits. When its terminal closes, the daemon kills the CLI's
   descendants and every process with that marker, also commands that run in
   their own process group or that the CLI abandoned. After a crash, the next
   daemon kills the processes an earlier daemon of the same state directory
   left (`src/lean/process_tree.rs`).
2. When the CLI has asked the gateway for its tools (every supported CLI does
   so while starting) and its screen is quiet, the prompt is typed into it as
   one bracketed paste followed by Enter (`src/lean/native.rs`).
3. The prompt names the turn and asks the agent to end it by calling
   `finish_turn(turn, text)` on the session's MCP gateway (`src/lean/mcp.rs`)
   with its complete final answer. That call is the only way back; the screen
   is never read for results.

A CLI that asks a question before it works (Claude and Agy ask whether to trust a
new directory; Claude inherits the answer from a trusted parent, Agy asks for
each directory) waits for the person: answer it in the terminal. Real runs of
Claude, Codex, OpenCode and Agy (orchestrator turn, answer back, terminal view,
resume after a daemon restart for the first three) are recorded in the evidence
below.

A call for a turn that is not the open one is refused with a message the model
can read. A CLI that does not connect within 60 s, exits, prints nothing for
15 minutes outside a wait for an answer (`ANTISLOP_TURN_IDLE_SECS`), or does
not end the turn within 2 hours (`ANTISLOP_TURN_TIMEOUT_SECS`) fails the turn
with the raw reason and the last lines of its terminal. A failure after the prompt was
typed is never retried automatically.

## Providers

| CLI | Gateway | Session identity | Resume |
|---|---|---|---|
| Claude | `--mcp-config` (HTTP, bearer token) | given with `--session-id` | `--resume <id>` |
| Codex | `-c mcp_servers.antislop.*`, token in the environment | `threadId` in the call | `codex resume <id>` |
| OpenCode | `OPENCODE_CONFIG_CONTENT` | the one new session in the directory | `--session <id>` |
| Agy | stdio bridge (`antislop mcp-bridge`) registered once by `antislop setup agy` | the conversation, added by the bridge | `--conversation <id>` |

Claude's plan mode refuses MCP tools, so read-only Claude runs in the default
mode without editing tools. Its shell is a normal one, but runs in the
operating system's sandbox with the working and extra directories write-protected
(checked: `touch` and `>` there fail with "Operation not permitted").

## Interaction

Overview displays worker/verifier terminals side by side from 80 columns.
`Ctrl+W` changes the active panel, `i` gives the CLI keyboard focus, and
`Ctrl+]` returns focus to antislop. While focused, keys, paste and mouse events
go to the CLI before global application shortcuts. Outside terminal focus, `a`
opens the shared message composer and `r` retries attachment.

Detaching or switching views keeps the daemon's PTY, including incomplete CLI
input. A session nobody uses or looks at for ten minutes is released; opening it
again resumes the provider's session. An earlier round shows a hint instead of a
terminal; `o` resumes that round's session read-only (role `ROLE#rN`). A new
task round uses its own session; an old terminal cannot enqueue work into the
new round.

Input has per-view sequence numbers. Lost acknowledgements are retried with the
same sequence; repeated delivery has no effect. When delivery cannot be
confirmed, text is recovered into the role's composer for the user to inspect.
It is never automatically replayed into a replacement terminal. Existing drafts
in other conversations remain independent.

## Evidence

| Behavior | Test |
|---|---|
| Gateway ends only the open turn, refuses wrong tokens and stale turns, reports the CLI's thread, cancels and notices a dead agent | `mcp::tests` |
| Per-launch commands and permissions for each CLI, no token in arguments, no inherited parent-session markers | `launch::tests` |
| A typed prompt reaches a bracketed-paste CLI byte for byte, however long | `terminal::tests::a_typed_prompt_arrives_exactly_as_written` |
| Panels show the terminal, the raw attachment failure or the earlier-round hint | `tui::details::tests`, `tui::native_view::tests` |
| Whole runs (plans, rounds, mail, restarts, recovery) on interactive fixtures | `tests/lean_*.rs` with `tests/support/pty_shim.py` |

The fixtures stand in for the CLIs: `pty_shim.py` runs in a terminal, connects
to the gateway as the real CLIs do, takes the pasted prompt and ends the turn
through `finish_turn`, while the test's own script computes the answer.
Real Claude, Codex and OpenCode sessions were exercised by hand: a prompt
typed into the daemon-owned terminal, the answer returned through the gateway,
and the terminal rendered in the view.
