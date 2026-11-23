# Daemon protocol 6

The local `daemon::protocol` module owns the request models, serde adapters and
request envelope. `operations.rs` declares each wire operation, its group, fields
and serde attributes once. Its local `macro_rules!` emits the five application
enums, `Request`, flat `WireRequest`, and exhaustive encode/decode matches.
The fixed group buckets preserve declaration order in the wire enum, including
serde's list of expected variants in errors. This is a local protocol declaration,
not a routing or policy framework.

Client, dispatch, idempotency and leases keep their grouped application API.
`request/conversation.rs` owns the existing idempotency classification;
`request/session.rs` owns session action/result types and their validation.
`wire_serde_impl.rs` applies the flat representation to `Request`; serialization
still clones the application value. This refactoring makes no performance claim.

Three differences require explicit adapters:

- `start`: absent/null project decodes to `ControlRequest::Start`, a string to
  `StartIn`. Start serializes with `project:null`.
- `events`: absent/null limit decodes to `HistoryRequest::Events`, a number to
  `EventsPaged`. Either application form with no limit serializes `limit:null`.
- `recover`: flat wire fields map to the existing `TaskRecoveryRequest` tuple
  variant; domain recovery rules remain outside this module.

The separate envelope requires `protocol` and `request`, defaults `client` to
None, and rejects extra fields. Lifecycle validates version 6 after decoding.
A Unix connection supports sequential request/reply exchanges. The shared
transport uses a JSON line for a small message and ordered frames for a large
one; it rejects duplicate object keys before serde discards them. Dispatch
validates each request independently. Neither request grouping nor task contracts
grant execution permissions.

## Compatibility inventory

All 58 operations below have literal expected JSON and independently constructed
grouped values in `tests/fixtures.rs`. This inventory and those fixtures are
maintained separately from the production declaration.

| Group | Operations |
| --- | --- |
| Control (13) | `identity`, `activate_runtime`, `runtime_activation_status`, `runtime_activation_operation`, `cancel_runtime_activation`, `classify_runtime_failure`, `runtime_notice`, `get_config`, `set_config`, `shutdown`, `force_shutdown`, `conversations`, `start` |
| Conversation (25) | `add_source`, `get_source`, `recover`, `comment`, `send_to_agent`, `send`, `reject`, `resume`, `take_control`, `restart_orchestrator`, `cancel`, `pause`, `set_approval_mode`, `answer_approval`, `review_comments`, `plans`, `plan`, `comments`, `comment_status`, `diff`, `review`, `task_checks`, `task`, `revise_plan`, `select_team` |
| History (7) | `task_rounds`, `task_round`, `history_content`, `history_messages`, `events`, `history`, `mail` |
| Session (8) | `session_action`, `run_context`, `session_events`, `session_content`, `result_documents`, `result_document_content`, `sessions`, `delete_session` |
| Terminal (5) | `terminal_open`, `terminal_read`, `terminal_input`, `terminal_resize`, `terminal_detach` |

For every operation the tests check encoding against literal JSON and decoding
against the constructed application value. Each fixture separately lists defaults,
nullable fields and fields omitted on serialization. Tests exercise every field
with a populated value, omission, null and a wrong type, as well as explicit
default values and simultaneous omission of all defaulted fields.
This covers false, zero, empty string/vector, optional filters/pagination and
the special omission of `activate_runtime.request_id` and empty `send.images`.

Record operations reject extra fields. Existing serde behavior for unit operations
accepts extra fields; this exception is deliberately preserved. Removed operations
and `scope_hash` on record operations remain rejected. A frozen diagnostic lists
all operations in the original order; selected missing-field, unknown-field,
numeric-range and envelope errors are checked independently. Tests also exercise
duplicate keys through the real transport and version rejection through lifecycle.
