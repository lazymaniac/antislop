# Native sessions in Overview

Codex uses a daemon-owned app-server, and OpenCode uses a dedicated serve
process. Their interactive CLIs attach through a managed gateway. The structured
driver and terminal therefore use the same provider identity and execution
queue. Claude, Gemini, Agy, unsupported CLIs and custom launchers keep JSON
history with an explicit attachment-unavailable notice.
Orchestrator mail records its originating user message in the append-only
event log. A turn only acknowledges mail within its prepared message boundary;
later input remains queued for a subsequent turn. Existing schema 11 records
retain their atomic mail/message event association and need no migration.

The database schema remains **11**. No migration is performed. Existing
incompatible databases are rejected without modifying or deleting their data.

## Interaction

Overview displays worker/verifier terminals side by side from 80 columns.
`Ctrl+W` changes the active panel, `i` gives the CLI keyboard focus, and
`Ctrl+]` returns focus to antislop. While focused, keys, paste and mouse events
go to the CLI before global application shortcuts. Outside terminal focus,
`a` opens the shared message composer, `h` switches to JSON history, and `r`
retries attachment. Search and raw JSON views expose the JSON history.

Detaching or switching views keeps the daemon's PTY, including incomplete CLI
input. Historical rounds display their own JSON records. A new task round uses
its own managed session; an old terminal cannot enqueue work into the new round.

The gateway accepts text messages through `send_to_agent`, with conversation,
role, task, round and run validation. It queues them without steering an active
execution. Explicit interrupts are logged separately. Unsupported mutations,
foreign sessions and attempts to change the managed agent are rejected.
Codex client message IDs remain idempotent across reconnects. Interrupts must
target the active turn; stale controls cannot interrupt its successor. Steering
also accepts the latest completed turn while its completion notification is
still in transit and queues the correction as new work. Queued steering
acknowledges the turn that actually consumes the message. Delivered receipts
survive cache expiry and restart: their provider turn is read from durable run
context and paged native history, without executing the prompt again.
If abort cannot be confirmed, the backend is stopped before the
role scheduler proceeds, and the server's failure reason remains visible.

Input has per-view sequence numbers. Lost acknowledgements are retried with the
same sequence; repeated delivery has no effect. When delivery cannot be
confirmed, text is recovered into the role's composer for the user to inspect.
It is not automatically replayed into a replacement terminal. Existing drafts
in other conversations remain independent.

## Acceptance coverage

| Requirement | Automated evidence |
|---|---|
| Shared native/JSON identity, provider turn IDs, live output and token usage | `native::codex::tests::structured_turns_share_the_managed_thread_and_keep_provider_turn_ids`; OpenCode direct HTTP/SSE and stream normalizer tests |
| Queued native correction executes once in the same session | `native::codex::tests::gateway_messages_join_the_core_queue_and_execute_in_the_same_session` |
| Two OpenCode corrections arriving during work are delivered in order; repeated input is deduplicated and later mail is not acknowledged by an earlier prompt | `opencode::tests::http_gateway_authenticates_and_limits_sessions_and_mutations` |
| Reconnects do not duplicate mail; stale controls are rejected; queued steering follows the RPC schema | Codex reconnect and gateway control tests |
| Interrupt preserves the provider session | Codex interruption test; `opencode::tests::explicit_interrupt_keeps_the_opencode_backend_and_records_control` |
| Failed abort stops the backend and preserves the raw cause | `opencode::tests::a_failed_abort_stops_the_backend_and_preserves_its_failure_reason` |
| No automatic execution fallback after acceptance | Codex backend failure test; OpenCode lost accepted stream test; mail retry classification |
| Old API falls back before a prompt | `native::tests::unsupported_server_protocol_falls_back_before_sending_a_prompt` |
| Restart cleans processes, interrupts turns, preserves identity/history, rejects stale handles | `native::tests::daemon_restart_reaps_native_processes_and_restores_the_same_identity` for both providers |
| Round/worktree/profile isolation | Native round rotation tests for both providers; gateway foreign-session tests |
| Messages arriving during verification require another round | `lean_messages::worker_message_during_verification_requires_another_round_before_approval` and direct worker correction tests |
| PTY draft survives detach; resize and handles are scoped | `native::codex::tests::pty_retains_unfinished_text_across_detach_and_scopes_handles` |
| Resize while waiting for the first session is used when attaching | `tui::native_view::tests::resize_while_waiting_for_a_session_is_used_by_the_next_open` |
| Focus precedes global shortcuts; two panels and historical JSON remain available | `tui::native_view::tests` |
| Lost acknowledgement and permanent errors preserve input | Native view acknowledgement, recovery and close-drain tests |
| Unicode/ZWJ, split ANSI, alternate screen, bounded buffers and terminal queries | `terminal::tests` |
| NO_COLOR, ASCII, small screens and existing composer behavior | `lean_tui` and native cell rendering tests |

The ignored `installed_codex_attach_smoke` and
`installed_opencode_attach_smoke` tests exercise installed native CLIs through
real PTYs: they open the interface and submit a message into durable user mail.
Each test begins attaching while one short model turn is executing and checks
the reply from that session, including its presence on the rendered PTY screen.
Submission is tested while holding the role
lock, then cancelled, so it cannot trigger an additional model call. The
ordinary adapter/runtime tests verify subsequent queued execution offline.
The OpenCode test uses the current OpenCode provider configuration. For an oMLX
configuration, its local server and selected model must be running first.
`omlx launch opencode` configures that provider; a custom launcher configured
as the antislop command deliberately keeps the legacy JSON driver.

Required checks are `cargo fmt --check`, `cargo check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test --test lean_core`,
`cargo test --test lean_tui`, and the native/terminal library tests. Integration
tests require local Unix/TCP sockets, PTYs and permission to inspect their own
process groups.
