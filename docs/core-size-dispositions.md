# Core and persistence size dispositions

Baseline: measured source snapshot `892e93d5c5b03a4da54a7c21c890acc6a0783192`, recorded in `tests/size_check/baseline-inventory.md`. The raw current repository diagnostic is retained in `core-size-diagnostic.txt`. It was captured with `CARGO_TARGET_DIR=.antislop/target cargo run --locked --quiet --bin diagnostic-size-check -- --root .`; the expected exit code is 1 while violations remain.

Current checker result: 362 repository violations, 0 classification errors; 108 violations in t4-owned `src/` paths. The initial t4 inventory contains 141 items; 33 are resolved and 108 remain unresolved. Exit code 1.

## Every initial t4 violation

| Kind | Baseline item | Initial lines | Current disposition |
|---|---|---:|---|
| file | `src/config.rs` | 645 | resolved: original source file is now at or below 299 lines |
| file | `src/lean/history.rs` | 1134 | resolved: original source file is now at or below 299 lines |
| file | `src/lean/ledger/runtime.rs` | 628 | unresolved: current source has 628 lines at `src/lean/ledger/runtime.rs:1` (limit 299) |
| file | `src/lean/ledger/storage.rs` | 1517 | unresolved: current source has 1517 lines at `src/lean/ledger/storage.rs:1` (limit 299) |
| file | `src/lean/mail.rs` | 414 | resolved: original source file is now at or below 299 lines |
| file | `src/lean/mod.rs` | 5761 | unresolved: current source has 5761 lines at `src/lean/mod.rs:1` (limit 299) |
| file | `src/lean/rounds.rs` | 1218 | resolved: original source file is now at or below 299 lines |
| file | `src/lean/run_context.rs` | 384 | resolved: original source file is now at or below 299 lines |
| file | `src/lean/store.rs` | 2986 | unresolved: current source has 2986 lines at `src/lean/store.rs:1` (limit 299) |
| file | `src/lean/tasks.rs` | 3523 | unresolved: current source has 3523 lines at `src/lean/tasks.rs:1` (limit 299) |
| file | `src/lean/workflow.rs` | 362 | resolved: original source file is now at or below 299 lines |
| callable | `src/config.rs::resolve` | 50 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/config.rs::resolve_command` | 32 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/config/validation.rs::toml_diagnostic` | 50 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/config/validation.rs::validate_profiles` | 53 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/config/validation.rs::validate_routing` | 37 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/diff.rs::parse` | 84 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/graph_edit.rs::extend_running_graph` | 142 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/history.rs::enqueue_agent_message` | 118 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/history.rs::handoff_rounds` | 39 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/history.rs::history_content` | 40 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/history.rs::history_messages` | 76 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/history.rs::invalidate_dependents` | 34 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/history.rs::session_events_with_messages` | 53 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/history.rs::task_round_with_references` | 82 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/history.rs::task_rounds` | 41 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/interview.rs::parse` | 45 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/ledger/options.rs::recover_options_comparisons` | 37 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/ledger/options.rs::save_solution_choice` | 55 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/ledger/options.rs::validate` | 40 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/ledger/runtime.rs::apply_user_ledger_change` | 46 | unresolved: `src/lean/ledger/runtime.rs:38` still has callable `apply_user_ledger_change` at 46 lines (limit 29) |
| callable | `src/lean/ledger/runtime.rs::compliance_audit` | 83 | unresolved: `src/lean/ledger/runtime.rs:412` still has callable `compliance_audit` at 83 lines (limit 29) |
| callable | `src/lean/ledger/runtime.rs::ensure_requirements` | 171 | unresolved: `src/lean/ledger/runtime.rs:220` still has callable `ensure_requirements` at 171 lines (limit 29) |
| callable | `src/lean/ledger/runtime.rs::full_ledger_sources` | 37 | unresolved: `src/lean/ledger/runtime.rs:183` still has callable `full_ledger_sources` at 37 lines (limit 29) |
| callable | `src/lean/ledger/runtime.rs::requirements_turn` | 87 | unresolved: `src/lean/ledger/runtime.rs:96` still has callable `requirements_turn` at 87 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::add_ledger_evidence` | 89 | unresolved: `src/lean/ledger/storage.rs:985` still has callable `add_ledger_evidence` at 89 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::apply_ledger_change_as` | 136 | unresolved: `src/lean/ledger/storage.rs:619` still has callable `apply_ledger_change_as` at 136 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::bind_ledger_turn` | 44 | unresolved: `src/lean/ledger/storage.rs:1300` still has callable `bind_ledger_turn` at 44 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::bind_task_ledger` | 60 | unresolved: `src/lean/ledger/storage.rs:847` still has callable `bind_task_ledger` at 60 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::checked_graph` | 51 | unresolved: `src/lean/ledger/storage.rs:102` still has callable `checked_graph` at 51 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::create_run` | 34 | unresolved: `src/lean/ledger/storage.rs:275` still has callable `create_run` at 34 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::enqueue_requirements_resume` | 34 | unresolved: `src/lean/ledger/storage.rs:1484` still has callable `enqueue_requirements_resume` at 34 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::entries` | 39 | unresolved: `src/lean/ledger/storage.rs:27` still has callable `entries` at 39 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::impact` | 58 | unresolved: `src/lean/ledger/storage.rs:153` still has callable `impact` at 58 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::ledger_diff` | 35 | unresolved: `src/lean/ledger/storage.rs:755` still has callable `ledger_diff` at 35 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::ledger_statuses` | 106 | unresolved: `src/lean/ledger/storage.rs:1084` still has callable `ledger_statuses` at 106 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::list_ledger` | 34 | unresolved: `src/lean/ledger/storage.rs:1190` still has callable `list_ledger` at 34 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::propose_ledger_change_as` | 31 | unresolved: `src/lean/ledger/storage.rs:567` still has callable `propose_ledger_change_as` at 31 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::record_ledger_review` | 73 | unresolved: `src/lean/ledger/storage.rs:907` still has callable `record_ledger_review` at 73 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::register_document_source` | 55 | unresolved: `src/lean/ledger/storage.rs:448` still has callable `register_document_source` at 55 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::register_message_source` | 38 | unresolved: `src/lean/ledger/storage.rs:410` still has callable `register_message_source` at 38 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::requirements_gaps` | 38 | unresolved: `src/lean/ledger/storage.rs:1224` still has callable `requirements_gaps` at 38 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::source_text` | 36 | unresolved: `src/lean/ledger/storage.rs:66` still has callable `source_text` at 36 lines (limit 29) |
| callable | `src/lean/ledger/storage.rs::validate_change` | 62 | unresolved: `src/lean/ledger/storage.rs:211` still has callable `validate_change` at 62 lines (limit 29) |
| callable | `src/lean/mail.rs::answer_in_background` | 73 | unresolved: `src/lean/mail/background.rs:4` still has callable `answer_in_background` at 77 lines (limit 29) |
| callable | `src/lean/mail.rs::answer_waiting` | 175 | unresolved: `src/lean/mail/delivery.rs:4` still has callable `answer_waiting` at 175 lines (limit 29) |
| callable | `src/lean/mail.rs::mail_address_task` | 33 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/mail.rs::ready_mail` | 39 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/mod.rs::blocked_report` | 56 | unresolved: `src/lean/mod.rs:5296` still has callable `blocked_report` at 56 lines (limit 29) |
| callable | `src/lean/mod.rs::comment_at` | 50 | unresolved: `src/lean/mod.rs:793` still has callable `comment_at` at 50 lines (limit 29) |
| callable | `src/lean/mod.rs::comment_label` | 74 | unresolved: `src/lean/mod.rs:5173` still has callable `comment_label` at 74 lines (limit 29) |
| callable | `src/lean/mod.rs::conversations` | 62 | unresolved: `src/lean/mod.rs:693` still has callable `conversations` at 62 lines (limit 29) |
| callable | `src/lean/mod.rs::deliver` | 109 | unresolved: `src/lean/mod.rs:4248` still has callable `deliver` at 109 lines (limit 29) |
| callable | `src/lean/mod.rs::dependency_tasks` | 32 | unresolved: `src/lean/mod.rs:4364` still has callable `dependency_tasks` at 32 lines (limit 29) |
| callable | `src/lean/mod.rs::diff` | 38 | unresolved: `src/lean/mod.rs:1094` still has callable `diff` at 38 lines (limit 29) |
| callable | `src/lean/mod.rs::finish_in_place` | 62 | unresolved: `src/lean/mod.rs:4842` still has callable `finish_in_place` at 62 lines (limit 29) |
| callable | `src/lean/mod.rs::freshness_for_task` | 38 | unresolved: `src/lean/mod.rs:990` still has callable `freshness_for_task` at 38 lines (limit 29) |
| callable | `src/lean/mod.rs::in_place_user_edits` | 34 | unresolved: `src/lean/mod.rs:4807` still has callable `in_place_user_edits` at 34 lines (limit 29) |
| callable | `src/lean/mod.rs::integrate` | 176 | unresolved: `src/lean/mod.rs:4505` still has callable `integrate` at 176 lines (limit 29) |
| callable | `src/lean/mod.rs::land` | 41 | unresolved: `src/lean/mod.rs:5089` still has callable `land` at 41 lines (limit 29) |
| callable | `src/lean/mod.rs::open` | 128 | unresolved: `src/lean/mod.rs:266` still has callable `open` at 128 lines (limit 29) |
| callable | `src/lean/mod.rs::orchestrate` | 40 | unresolved: `src/lean/mod.rs:2302` still has callable `orchestrate` at 40 lines (limit 29) |
| callable | `src/lean/mod.rs::orchestrator_prompt` | 50 | unresolved: `src/lean/mod.rs:5353` still has callable `orchestrator_prompt` at 50 lines (limit 29) |
| callable | `src/lean/mod.rs::orchestrator_turn` | 333 | unresolved: `src/lean/mod.rs:2369` still has callable `orchestrator_turn` at 333 lines (limit 29) |
| callable | `src/lean/mod.rs::plan_report` | 122 | unresolved: `src/lean/mod.rs:4940` still has callable `plan_report` at 122 lines (limit 29) |
| callable | `src/lean/mod.rs::propose` | 187 | unresolved: `src/lean/mod.rs:2767` still has callable `propose` at 187 lines (limit 29) |
| callable | `src/lean/mod.rs::record_check` | 30 | unresolved: `src/lean/mod.rs:3385` still has callable `record_check` at 30 lines (limit 29) |
| callable | `src/lean/mod.rs::recover_task_as` | 570 | unresolved: `src/lean/mod.rs:1392` still has callable `recover_task_as` at 570 lines (limit 29) |
| callable | `src/lean/mod.rs::reject_plan` | 38 | unresolved: `src/lean/mod.rs:1284` still has callable `reject_plan` at 38 lines (limit 29) |
| callable | `src/lean/mod.rs::repair_integration` | 124 | unresolved: `src/lean/mod.rs:4682` still has callable `repair_integration` at 124 lines (limit 29) |
| callable | `src/lean/mod.rs::repair_writer_base` | 63 | unresolved: `src/lean/mod.rs:4422` still has callable `repair_writer_base` at 63 lines (limit 29) |
| callable | `src/lean/mod.rs::review` | 50 | unresolved: `src/lean/mod.rs:2955` still has callable `review` at 50 lines (limit 29) |
| callable | `src/lean/mod.rs::review_change` | 102 | unresolved: `src/lean/mod.rs:1133` still has callable `review_change` at 102 lines (limit 29) |
| callable | `src/lean/mod.rs::revise_running_graph` | 63 | unresolved: `src/lean/mod.rs:2703` still has callable `revise_running_graph` at 63 lines (limit 29) |
| callable | `src/lean/mod.rs::run_plan` | 150 | unresolved: `src/lean/mod.rs:3454` still has callable `run_plan` at 150 lines (limit 29) |
| callable | `src/lean/mod.rs::run_task` | 586 | unresolved: `src/lean/mod.rs:3605` still has callable `run_task` at 586 lines (limit 29) |
| callable | `src/lean/mod.rs::start_plan_with_scope_lock` | 122 | unresolved: `src/lean/mod.rs:2131` still has callable `start_plan_with_scope_lock` at 122 lines (limit 29) |
| callable | `src/lean/mod.rs::stop_task` | 34 | unresolved: `src/lean/mod.rs:4905` still has callable `stop_task` at 34 lines (limit 29) |
| callable | `src/lean/mod.rs::submit_addressed` | 192 | unresolved: `src/lean/mod.rs:434` still has callable `submit_addressed` at 192 lines (limit 29) |
| callable | `src/lean/mod.rs::turn_inner` | 367 | unresolved: `src/lean/mod.rs:3017` still has callable `turn_inner` at 367 lines (limit 29) |
| callable | `src/lean/mod.rs::unblock_by_orchestrator` | 74 | unresolved: `src/lean/mod.rs:2009` still has callable `unblock_by_orchestrator` at 74 lines (limit 29) |
| callable | `src/lean/mod.rs::unblock_dependent_tasks` | 45 | unresolved: `src/lean/mod.rs:1963` still has callable `unblock_dependent_tasks` at 45 lines (limit 29) |
| callable | `src/lean/mod.rs::warn_of_uncommitted` | 34 | unresolved: `src/lean/mod.rs:2254` still has callable `warn_of_uncommitted` at 34 lines (limit 29) |
| callable | `src/lean/rounds.rs::round_handoff` | 127 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/rounds.rs::session_for_turn` | 52 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/run_context.rs::record_run_context` | 65 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/store.rs::add_comment_at` | 65 | unresolved: `src/lean/store.rs:1389` still has callable `add_comment_at` at 65 lines (limit 29) |
| callable | `src/lean/store.rs::add_draft_comment` | 33 | unresolved: `src/lean/store.rs:1455` still has callable `add_draft_comment` at 33 lines (limit 29) |
| callable | `src/lean/store.rs::add_mail_from` | 36 | unresolved: `src/lean/store.rs:1341` still has callable `add_mail_from` at 36 lines (limit 29) |
| callable | `src/lean/store.rs::add_message` | 55 | unresolved: `src/lean/store.rs:477` still has callable `add_message` at 55 lines (limit 29) |
| callable | `src/lean/store.rs::add_plan` | 43 | unresolved: `src/lean/store.rs:991` still has callable `add_plan` at 43 lines (limit 29) |
| callable | `src/lean/store.rs::append_event` | 51 | unresolved: `src/lean/store.rs:2011` still has callable `append_event` at 51 lines (limit 29) |
| callable | `src/lean/store.rs::dependency_reports` | 36 | unresolved: `src/lean/store.rs:1085` still has callable `dependency_reports` at 36 lines (limit 29) |
| callable | `src/lean/store.rs::events_since_budget` | 49 | unresolved: `src/lean/store.rs:2067` still has callable `events_since_budget` at 49 lines (limit 29) |
| callable | `src/lean/store.rs::initialize` | 81 | unresolved: `src/lean/store.rs:303` still has callable `initialize` at 81 lines (limit 29) |
| callable | `src/lean/store.rs::interrupt_running_turns` | 44 | unresolved: `src/lean/store.rs:854` still has callable `interrupt_running_turns` at 44 lines (limit 29) |
| callable | `src/lean/store.rs::interrupt_unfinished_tasks` | 42 | unresolved: `src/lean/store.rs:899` still has callable `interrupt_unfinished_tasks` at 42 lines (limit 29) |
| callable | `src/lean/store.rs::last_turn_for_roles` | 37 | unresolved: `src/lean/store.rs:757` still has callable `last_turn_for_roles` at 37 lines (limit 29) |
| callable | `src/lean/store.rs::plan` | 57 | unresolved: `src/lean/store.rs:1122` still has callable `plan` at 57 lines (limit 29) |
| callable | `src/lean/store.rs::plan_history` | 164 | unresolved: `src/lean/store.rs:2390` still has callable `plan_history` at 164 lines (limit 29) |
| callable | `src/lean/store.rs::plans_page` | 45 | unresolved: `src/lean/store.rs:2344` still has callable `plans_page` at 45 lines (limit 29) |
| callable | `src/lean/store.rs::resolve_listed_plan_comments` | 30 | unresolved: `src/lean/store.rs:1603` still has callable `resolve_listed_plan_comments` at 30 lines (limit 29) |
| callable | `src/lean/store.rs::send_draft_comments` | 59 | unresolved: `src/lean/store.rs:1489` still has callable `send_draft_comments` at 59 lines (limit 29) |
| callable | `src/lean/store.rs::set_check` | 46 | unresolved: `src/lean/store.rs:1180` still has callable `set_check` at 46 lines (limit 29) |
| callable | `src/lean/store.rs::set_comment_status` | 48 | unresolved: `src/lean/store.rs:1549` still has callable `set_comment_status` at 48 lines (limit 29) |
| callable | `src/lean/store.rs::set_session` | 34 | unresolved: `src/lean/store.rs:680` still has callable `set_session` at 34 lines (limit 29) |
| callable | `src/lean/store.rs::start_turn` | 57 | unresolved: `src/lean/store.rs:622` still has callable `start_turn` at 57 lines (limit 29) |
| callable | `src/lean/store.rs::task_by_id` | 60 | unresolved: `src/lean/store.rs:2186` still has callable `task_by_id` at 60 lines (limit 29) |
| callable | `src/lean/store.rs::update_task` | 74 | unresolved: `src/lean/store.rs:1243` still has callable `update_task` at 74 lines (limit 29) |
| callable | `src/lean/tasks.rs::bound_cut` | 41 | unresolved: `src/lean/tasks.rs:1162` still has callable `bound_cut` at 41 lines (limit 29) |
| callable | `src/lean/tasks.rs::checklist` | 200 | unresolved: `src/lean/tasks.rs:1550` still has callable `checklist` at 200 lines (limit 29) |
| callable | `src/lean/tasks.rs::code_review_prompt` | 36 | unresolved: `src/lean/tasks.rs:2042` still has callable `code_review_prompt` at 36 lines (limit 29) |
| callable | `src/lean/tasks.rs::coverage` | 40 | unresolved: `src/lean/tasks.rs:1344` still has callable `coverage` at 40 lines (limit 29) |
| callable | `src/lean/tasks.rs::effect` | 35 | unresolved: `src/lean/tasks.rs:1385` still has callable `effect` at 35 lines (limit 29) |
| callable | `src/lean/tasks.rs::orchestrator_instructions` | 95 | unresolved: `src/lean/tasks.rs:1454` still has callable `orchestrator_instructions` at 95 lines (limit 29) |
| callable | `src/lean/tasks.rs::parse_effect` | 52 | unresolved: `src/lean/tasks.rs:115` still has callable `parse_effect` at 52 lines (limit 29) |
| callable | `src/lean/tasks.rs::parse_graph_edit` | 42 | unresolved: `src/lean/tasks.rs:290` still has callable `parse_graph_edit` at 42 lines (limit 29) |
| callable | `src/lean/tasks.rs::parse_progress` | 35 | unresolved: `src/lean/tasks.rs:1067` still has callable `parse_progress` at 35 lines (limit 29) |
| callable | `src/lean/tasks.rs::peers_of` | 43 | unresolved: `src/lean/tasks.rs:526` still has callable `peers_of` at 43 lines (limit 29) |
| callable | `src/lean/tasks.rs::render_plan_document` | 54 | unresolved: `src/lean/tasks.rs:570` still has callable `render_plan_document` at 54 lines (limit 29) |
| callable | `src/lean/tasks.rs::round_handoff_prompt` | 48 | unresolved: `src/lean/tasks.rs:2337` still has callable `round_handoff_prompt` at 48 lines (limit 29) |
| callable | `src/lean/tasks.rs::settle` | 47 | unresolved: `src/lean/tasks.rs:1000` still has callable `settle` at 47 lines (limit 29) |
| callable | `src/lean/tasks.rs::upstream_note` | 37 | unresolved: `src/lean/tasks.rs:1805` still has callable `upstream_note` at 37 lines (limit 29) |
| callable | `src/lean/tasks.rs::validate` | 33 | unresolved: `src/lean/tasks.rs:388` still has callable `validate` at 33 lines (limit 29) |
| callable | `src/lean/tasks.rs::verifier_prompt` | 34 | unresolved: `src/lean/tasks.rs:1974` still has callable `verifier_prompt` at 34 lines (limit 29) |
| callable | `src/lean/tasks.rs::with_options` | 38 | unresolved: `src/lean/tasks.rs:870` still has callable `with_options` at 38 lines (limit 29) |
| callable | `src/lean/tasks.rs::with_required_agents` | 115 | unresolved: `src/lean/tasks.rs:754` still has callable `with_required_agents` at 115 lines (limit 29) |
| callable | `src/lean/tasks.rs::worker_prompt` | 53 | unresolved: `src/lean/tasks.rs:1843` still has callable `worker_prompt` at 53 lines (limit 29) |
| callable | `src/lean/workflow.rs::approve_requirements` | 43 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/workflow.rs::finish_interview` | 31 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/workflow.rs::queue_plan_format_repair` | 45 | resolved: no callable with this name remains over 29 lines in its owned source area |

## Current violations at new paths

- `src/lean/mail/background.rs:4`: callable `answer_in_background` is 77 lines (limit 29).
- `src/lean/mail/delivery.rs:4`: callable `answer_waiting` is 175 lines (limit 29).

## Ownership note

The task-card scope lists `src/main.rs` and `src/transport.rs`, while the reviewed t2 ownership manifest assigns both to t5. Following the explicit instruction to follow that manifest, this worker left both paths untouched. The current diagnostic records the t5-owned transport violations `deserialize` (56 lines at line 16) and `read` (33 lines at line 119); main.rs has no current size violation.

## Behavioral fix record

- During the `session_events_with_messages` helper extraction, the focused `session_events_page_references_and_enforce_ownership` test hung because scope validation tried to reacquire the Store mutex while a database guard was live. The refactor now validates scope before taking that guard, then drops the query guard before reading messages. The same regression test passes (1 passed). This is an implementation correction required to preserve the pre-existing behavior, not an intentional API or persistence contract change.

## Characterization and validation

- Existing history unit tests were moved from inline `src/lean/history.rs` into `src/lean/history/tests.rs`, grouped as `mail_tests`, `round_tests`, and `recovery_tests`. They continue to cover user-message association and schema 11, session event/content paging and ownership, round attribution/idempotency, and rollback/restart recovery. Public-boundary integration tests remain in `tests/lean_core.rs`, `tests/lean_ledger.rs`, and `tests/lean_messages.rs`.
- `cargo test --locked --lib history::tests`: 8 passed; `cargo test --locked --test lean_core`: 21 passed; `cargo test --locked --test lean_ledger`: 25 passed; `cargo test --locked --test lean_tui`: 110 passed. The focused `direct_orchestrator_message_uses_normal_workflow_and_validates_the_run` and `direct_agent_messages_resume_work_and_recheck_without_receipt_only_turns` tests passed.
- `cargo test --locked --test lean_messages` was characterized before changing mailbox code and repeated after the enqueue helper split; both runs had 30 passed and the same 4 failures: `agent_mail_reaches_the_orchestrator_without_replanning` (assertion expected 1, got 2), `deferred_options_comment_claim_expires_on_new_user_input`, `deferred_options_proposal_prefers_the_current_explicit_comment_list`, and `deferred_options_proposal_preserves_selective_comment_claim`. The initial characterization also repeated serially with the same failures. No changes were made to those behaviors or tests.
- `cargo fmt --check`, `cargo check --locked`, `cargo clippy --locked --all-targets -- -D warnings`, and `git diff --check` passed. The current size diagnostic is the separate retained artifact linked above and still exits 1 because 108 owned violations remain.
