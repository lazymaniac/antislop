# Core and persistence size dispositions

Baseline: measured source snapshot `892e93d5c5b03a4da54a7c21c890acc6a0783192`, recorded in `tests/size_check/baseline-inventory.md`. The raw current repository diagnostic is retained in `core-size-diagnostic.txt`. The initial capture used `CARGO_TARGET_DIR=.antislop/target cargo run --locked --quiet --bin diagnostic-size-check -- --root .`; the latest refresh ran `cargo run --locked --quiet --bin diagnostic-size-check -- --root .` against current source. The expected exit code is 1 while violations remain.

Current checker result after this repair: 293 repository violations, 0 classification errors; 39 violations in t4-owned `src/` paths. Four are file overages in extracted Core modules; the rest are callable overages in Core, Store, and Tasks. `src/lean/mod.rs` is now below the 299-line file limit. The initial t4 inventory contains 141 items. The 299-line file and 29-line callable limits remain unmet. Exit code 1. `core-size-diagnostic.txt` retains the exact raw full-repository output.

## Every initial t4 violation

| Kind | Baseline item | Initial lines | Current disposition |
|---|---|---:|---|
| file | `src/config.rs` | 645 | resolved: original source file is now at or below 299 lines |
| file | `src/lean/history.rs` | 1134 | resolved: original source file is now at or below 299 lines |
| file | `src/lean/ledger/runtime.rs` | 628 | resolved: requirements orchestration and its tests are in private `runtime/requirements.rs` and `runtime/tests.rs`; `runtime.rs` is 245 lines |
| file | `src/lean/ledger/storage.rs` | 1517 | resolved: the storage facade and cohesive run, source, entry, change, review, evidence, turn, and validation responsibilities are in private `storage/` modules; the facade is 186 lines |
| file | `src/lean/mail.rs` | 414 | resolved: original source file is now at or below 299 lines |
| file | `src/lean/mod.rs` | 5761 | resolved: declarations are in this 281-line facade; Core behavior is split into private cohesive modules |
| file | `src/lean/rounds.rs` | 1218 | resolved: original source file is now at or below 299 lines |
| file | `src/lean/run_context.rs` | 384 | resolved: original source file is now at or below 299 lines |
| file | `src/lean/store.rs` | 2986 | resolved: Store methods are split among focused private store modules; the facade is 295 lines |
| file | `src/lean/tasks.rs` | 3523 | resolved: parsers, planning, validation, feedback, prompts and test groups are in private task modules; the facade is 142 lines |
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
| callable | `src/lean/ledger/runtime.rs::apply_user_ledger_change` | 46 | resolved: exact proposal application and affected-task cancellation/announcements are separate helpers; the public entrypoint is below 30 lines and retains the integration lock boundary |
| callable | `src/lean/ledger/runtime.rs::compliance_audit` | 83 | resolved: moved the audit flow into `runtime/compliance.rs`; scope capture, analyst proposal, independent review, lock-protected persistence, and completion reporting are separate steps |
| callable | `src/lean/ledger/runtime.rs::ensure_requirements` | 171 | resolved: extraction now delegates run/source preparation, analysis, validation, application and review to focused helpers; no overage remains for this entrypoint |
| callable | `src/lean/ledger/runtime.rs::full_ledger_sources` | 37 | resolved: source iteration, interview-question context, and summary rendering are separate helpers; the entrypoint is within 29 lines |
| callable | `src/lean/ledger/runtime.rs::requirements_turn` | 87 | resolved: turn setup, metadata capture, progress publishing, agent execution, and turn binding are cohesive helpers in `src/lean/ledger/runtime/requirements/turn.rs`; the entrypoint is below 30 lines |
| callable | `src/lean/ledger/storage.rs::add_ledger_evidence` | 89 | resolved: evidence validation and insertion are separate helpers; the public method is within 29 lines |
| callable | `src/lean/ledger/storage.rs::apply_ledger_change_as` | 136 | resolved: transaction coordination delegates version writes, invalidation, decision, event, and resume work to private helpers in `storage/changes/apply.rs` |
| callable | `src/lean/ledger/storage.rs::bind_ledger_turn` | 44 | resolved: revision, owner, and source-coverage validation are separate helpers; the transaction and commit boundary remain in the entrypoint |
| callable | `src/lean/ledger/storage.rs::bind_task_ledger` | 60 | resolved: prior coverage, task input binding, assignment validation, and criterion coverage are separate helpers |
| callable | `src/lean/ledger/storage.rs::checked_graph` | 51 | resolved: validation delegates identity, criteria, relations, composite requirements, and cycle checks to focused helpers |
| callable | `src/lean/ledger/storage.rs::create_run` | 34 | resolved: start-message selection is isolated from run creation and event persistence |
| callable | `src/lean/ledger/storage.rs::enqueue_requirements_resume` | 34 | resolved: duplicate check, resume message creation, and event emission are delegated to named helpers |
| callable | `src/lean/ledger/storage.rs::entries` | 39 | resolved: row loading and entry reconstruction are separated in `storage/entries.rs` |
| callable | `src/lean/ledger/storage.rs::impact` | 58 | resolved: entry impact propagation, linked task selection, dependent task expansion, and result ordering are separate helpers |
| callable | `src/lean/ledger/storage.rs::ledger_diff` | 35 | resolved: changed identity selection and result assembly are separate helpers |
| callable | `src/lean/ledger/storage.rs::ledger_statuses` | 106 | resolved: status assignment and fixed-point dependency checks are separate helpers in `storage/evidence/status.rs` |
| callable | `src/lean/ledger/storage.rs::list_ledger` | 34 | resolved: filtering and page assembly are separate helpers in `storage/evidence/queries.rs` |
| callable | `src/lean/ledger/storage.rs::propose_ledger_change_as` | 31 | resolved: idempotent proposal lookup and proposal persistence are separate helpers |
| callable | `src/lean/ledger/storage.rs::record_ledger_review` | 73 | resolved: session, source coverage, role validation, review persistence, and event emission are separate helpers |
| callable | `src/lean/ledger/storage.rs::register_document_source` | 55 | resolved: immutable artifact writing and source row/event persistence are separate helpers |
| callable | `src/lean/ledger/storage.rs::register_message_source` | 38 | resolved: message decoding and source row/event persistence are separate helpers |
| callable | `src/lean/ledger/storage.rs::requirements_gaps` | 38 | resolved: extraction, entry, and audit gaps are gathered by separate helpers in `storage/evidence/queries.rs` |
| callable | `src/lean/ledger/storage.rs::source_text` | 36 | resolved: message-backed and document-backed source reads now have separate helpers |
| callable | `src/lean/ledger/storage.rs::validate_change` | 62 | resolved: change scope, entry authority, source spans, graph validation, and affected-task calculation are separated into focused helpers |
| callable | `src/lean/mail.rs::answer_in_background` | 73 | resolved: the entrypoint delegates the background drain to short private helpers; no callable overage remains for this behavior |
| callable | `src/lean/mail.rs::answer_waiting` | 175 | resolved: delivery is divided into recipient selection, retry, context, and acknowledgement helpers; no callable overage remains for this behavior |
| callable | `src/lean/mail.rs::mail_address_task` | 33 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/mail.rs::ready_mail` | 39 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/mod.rs::blocked_report` | 56 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/mod.rs::comment_at` | 50 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/mod.rs::comment_label` | 74 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/mod.rs::conversations` | 62 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/mod.rs::deliver` | 109 | unresolved: still over the 29-line limit at `src/lean/core_17_repeats_a_plan_that_ran.rs:60:` (109 lines) |
| callable | `src/lean/mod.rs::dependency_tasks` | 32 | unresolved: still over the 29-line limit at `src/lean/core_17_repeats_a_plan_that_ran.rs:176:` (36 lines) |
| callable | `src/lean/mod.rs::diff` | 38 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/mod.rs::finish_in_place` | 62 | unresolved: still over the 29-line limit at `src/lean/core_21_finish_in_place.rs:5:` (62 lines) |
| callable | `src/lean/mod.rs::freshness_for_task` | 38 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/mod.rs::in_place_user_edits` | 34 | unresolved: still over the 29-line limit at `src/lean/core_20_repair_integration.rs:131:` (38 lines) |
| callable | `src/lean/mod.rs::integrate` | 176 | unresolved: still over the 29-line limit at `src/lean/core_19_integrate.rs:6:` (176 lines) |
| callable | `src/lean/mod.rs::land` | 41 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/mod.rs::open` | 128 | resolved: startup locking, recovery, Core construction, and checkpoint resumption are separate helpers in `core/open.rs` |
| callable | `src/lean/mod.rs::orchestrate` | 40 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/mod.rs::orchestrator_prompt` | 50 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/mod.rs::orchestrator_turn` | 333 | unresolved: still over the 29-line limit at `src/lean/core_10_orchestrator_turn.rs:5:` (333 lines) |
| callable | `src/lean/mod.rs::plan_report` | 122 | unresolved: still over the 29-line limit at `src/lean/core_22_plan_report.rs:5:` (122 lines) |
| callable | `src/lean/mod.rs::propose` | 187 | unresolved: still over the 29-line limit at `src/lean/core_12_propose.rs:6:` (187 lines) |
| callable | `src/lean/mod.rs::record_check` | 30 | unresolved: still over the 29-line limit at `src/lean/core_15_record_check.rs:4:` (35 lines) |
| callable | `src/lean/mod.rs::recover_task_as` | 570 | unresolved: still over the 29-line limit at `src/lean/core_06_recover_task_as.rs:6:` (570 lines) |
| callable | `src/lean/mod.rs::reject_plan` | 38 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/mod.rs::repair_integration` | 124 | unresolved: still over the 29-line limit at `src/lean/core_20_repair_integration.rs:6:` (124 lines) |
| callable | `src/lean/mod.rs::repair_writer_base` | 63 | unresolved: still over the 29-line limit at `src/lean/core_18_writer_base.rs:35:` (63 lines) |
| callable | `src/lean/mod.rs::review` | 50 | unresolved: still over the 29-line limit at `src/lean/core_13_review.rs:4:` (36 lines) |
| callable | `src/lean/mod.rs::review_change` | 102 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/mod.rs::revise_running_graph` | 63 | unresolved: still over the 29-line limit at `src/lean/core_11_revise_running_graph.rs:5:` (33 lines) |
| callable | `src/lean/mod.rs::run_plan` | 150 | unresolved: still over the 29-line limit at `src/lean/core_15_record_check.rs:88:` (155 lines) |
| callable | `src/lean/mod.rs::run_task` | 586 | unresolved: still over the 29-line limit at `src/lean/core_16_run_task.rs:6:` (586 lines) |
| callable | `src/lean/mod.rs::start_plan_with_scope_lock` | 122 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/mod.rs::stop_task` | 34 | unresolved: still over the 29-line limit at `src/lean/core_21_finish_in_place.rs:68:` (34 lines) |
| callable | `src/lean/mod.rs::submit_addressed` | 192 | resolved: address parsing and direct, broadcast, task, and rejection routes are separated in `core/messages.rs` |
| callable | `src/lean/mod.rs::turn_inner` | 367 | unresolved: still over the 29-line limit at `src/lean/core_14_turn_inner.rs:4:` (367 lines) |
| callable | `src/lean/mod.rs::unblock_by_orchestrator` | 74 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/mod.rs::unblock_dependent_tasks` | 45 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/mod.rs::warn_of_uncommitted` | 34 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/rounds.rs::round_handoff` | 127 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/rounds.rs::session_for_turn` | 52 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/run_context.rs::record_run_context` | 65 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/store.rs::add_comment_at` | 65 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/store.rs::add_draft_comment` | 33 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/store.rs::add_mail_from` | 36 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/store.rs::add_message` | 55 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/store.rs::add_plan` | 43 | unresolved: still over the 29-line limit at `src/lean/store/sessions_and_plans.rs:200:` (43 lines) |
| callable | `src/lean/store.rs::append_event` | 51 | unresolved: still over the 29-line limit at `src/lean/store/mail_and_events.rs:172:` (51 lines) |
| callable | `src/lean/store.rs::dependency_reports` | 36 | unresolved: still over the 29-line limit at `src/lean/store/plan_tasks.rs:40:` (36 lines) |
| callable | `src/lean/store.rs::events_since_budget` | 49 | unresolved: still over the 29-line limit at `src/lean/store/mail_and_events.rs:228:` (49 lines) |
| callable | `src/lean/store.rs::initialize` | 81 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/store.rs::interrupt_running_turns` | 44 | unresolved: still over the 29-line limit at `src/lean/store/sessions_and_plans.rs:63:` (44 lines) |
| callable | `src/lean/store.rs::interrupt_unfinished_tasks` | 42 | unresolved: still over the 29-line limit at `src/lean/store/sessions_and_plans.rs:108:` (42 lines) |
| callable | `src/lean/store.rs::last_turn_for_roles` | 37 | unresolved: still over the 29-line limit at `src/lean/store/messages_and_turns.rs:228:` (37 lines) |
| callable | `src/lean/store.rs::plan` | 57 | unresolved: still over the 29-line limit at `src/lean/store/plan_tasks.rs:77:` (57 lines) |
| callable | `src/lean/store.rs::plan_history` | 164 | resolved: plan-history orchestration, timing, turn loading, aggregation, and report collection are separate helpers; every callable in `src/lean/store/plan_history.rs` is at or below 29 lines. Helpers share one held connection guard, preserving the original mutex boundary across all database reads. |
| callable | `src/lean/store.rs::plans_page` | 45 | unresolved: still over the 29-line limit at `src/lean/store/history_queries.rs:235:` (45 lines) |
| callable | `src/lean/store.rs::resolve_listed_plan_comments` | 30 | unresolved: still over the 29-line limit at `src/lean/store/comment_mail_state.rs:9:` (30 lines) |
| callable | `src/lean/store.rs::send_draft_comments` | 59 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/store.rs::set_check` | 46 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/store.rs::set_comment_status` | 48 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/store.rs::set_session` | 34 | unresolved: still over the 29-line limit at `src/lean/store/messages_and_turns.rs:151:` (34 lines) |
| callable | `src/lean/store.rs::start_turn` | 57 | unresolved: still over the 29-line limit at `src/lean/store/messages_and_turns.rs:93:` (57 lines) |
| callable | `src/lean/store.rs::task_by_id` | 60 | unresolved: still over the 29-line limit at `src/lean/store/history_queries.rs:77:` (60 lines) |
| callable | `src/lean/store.rs::update_task` | 74 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/tasks.rs::bound_cut` | 41 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/tasks.rs::checklist` | 200 | resolved: the dispatcher selects one short checklist helper per work kind; no callable in `src/lean/tasks/prompts/checklist.rs` exceeds 29 lines |
| callable | `src/lean/tasks.rs::code_review_prompt` | 36 | unresolved: still over the 29-line limit at `src/lean/tasks/prompts/review.rs:71:` (36 lines) |
| callable | `src/lean/tasks.rs::coverage` | 40 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/tasks.rs::effect` | 35 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/tasks.rs::orchestrator_instructions` | 95 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/tasks.rs::parse_effect` | 52 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/tasks.rs::parse_graph_edit` | 42 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/tasks.rs::parse_progress` | 35 | unresolved: still over the 29-line limit at `src/lean/tasks/validation.rs:153:` (35 lines) |
| callable | `src/lean/tasks.rs::peers_of` | 43 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/tasks.rs::render_plan_document` | 54 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/tasks.rs::round_handoff_prompt` | 48 | unresolved: still over the 29-line limit at `src/lean/tasks/prompts/requirements.rs:175:` (48 lines) |
| callable | `src/lean/tasks.rs::settle` | 47 | unresolved: still over the 29-line limit at `src/lean/tasks/validation.rs:86:` (47 lines) |
| callable | `src/lean/tasks.rs::upstream_note` | 37 | unresolved: still over the 29-line limit at `src/lean/tasks/prompts/worker.rs:57:` (37 lines) |
| callable | `src/lean/tasks.rs::validate` | 33 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/tasks.rs::verifier_prompt` | 34 | unresolved: still over the 29-line limit at `src/lean/tasks/prompts/review.rs:3:` (34 lines) |
| callable | `src/lean/tasks.rs::with_options` | 38 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/tasks.rs::with_required_agents` | 115 | resolved: no current callable with this name exceeds the 29-line limit |
| callable | `src/lean/tasks.rs::worker_prompt` | 53 | unresolved: still over the 29-line limit at `src/lean/tasks/prompts/worker.rs:95:` (53 lines) |
| callable | `src/lean/workflow.rs::approve_requirements` | 43 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/workflow.rs::finish_interview` | 31 | resolved: no callable with this name remains over 29 lines in its owned source area |
| callable | `src/lean/workflow.rs::queue_plan_format_repair` | 45 | resolved: no callable with this name remains over 29 lines in its owned source area |

## Current violations at new paths

The latest full diagnostic reports 39 t4-owned violations: four oversized Core files and 35 callable overages. The inventory below is the exact current checker output for t4-owned source paths; it includes extracted paths so complexity relocation is visible.

```text
VIOLATION src/lean/core_06_recover_task_as.rs:1: file is 576 lines (limit 299)
VIOLATION src/lean/core_06_recover_task_as.rs:6: callable recover_task_as is 570 lines (limit 29)
VIOLATION src/lean/core_10_orchestrator_turn.rs:1: file is 338 lines (limit 299)
VIOLATION src/lean/core_10_orchestrator_turn.rs:5: callable orchestrator_turn is 333 lines (limit 29)
VIOLATION src/lean/core_12_propose.rs:6: callable propose is 187 lines (limit 29)
VIOLATION src/lean/core_14_turn_inner.rs:1: file is 371 lines (limit 299)
VIOLATION src/lean/core_14_turn_inner.rs:4: callable turn_inner is 367 lines (limit 29)
VIOLATION src/lean/core_15_record_check.rs:98: callable run_plan is 155 lines (limit 29)
VIOLATION src/lean/core_16_run_task.rs:1: file is 592 lines (limit 299)
VIOLATION src/lean/core_16_run_task.rs:6: callable run_task is 586 lines (limit 29)
VIOLATION src/lean/core_17_repeats_a_plan_that_ran.rs:60: callable deliver is 109 lines (limit 29)
VIOLATION src/lean/core_18_writer_base.rs:35: callable repair_writer_base is 63 lines (limit 29)
VIOLATION src/lean/core_19_integrate.rs:6: callable integrate is 176 lines (limit 29)
VIOLATION src/lean/core_20_repair_integration.rs:131: callable in_place_user_edits is 38 lines (limit 29)
VIOLATION src/lean/core_20_repair_integration.rs:6: callable repair_integration is 124 lines (limit 29)
VIOLATION src/lean/core_21_finish_in_place.rs:5: callable finish_in_place is 62 lines (limit 29)
VIOLATION src/lean/core_21_finish_in_place.rs:68: callable stop_task is 34 lines (limit 29)
VIOLATION src/lean/core_22_plan_report.rs:5: callable plan_report is 122 lines (limit 29)
VIOLATION src/lean/store/comment_mail_state.rs:9: callable resolve_listed_plan_comments is 30 lines (limit 29)
VIOLATION src/lean/store/comments_and_mail/comments/insert.rs:38: callable persist_comment is 30 lines (limit 29)
VIOLATION src/lean/store/history_queries.rs:235: callable plans_page is 45 lines (limit 29)
VIOLATION src/lean/store/history_queries.rs:77: callable task_by_id is 60 lines (limit 29)
VIOLATION src/lean/store/mail_and_events.rs:172: callable append_event is 51 lines (limit 29)
VIOLATION src/lean/store/mail_and_events.rs:228: callable events_since_budget is 49 lines (limit 29)
VIOLATION src/lean/store/messages_and_turns.rs:151: callable set_session is 34 lines (limit 29)
VIOLATION src/lean/store/messages_and_turns.rs:228: callable last_turn_for_roles is 37 lines (limit 29)
VIOLATION src/lean/store/messages_and_turns.rs:93: callable start_turn is 57 lines (limit 29)
VIOLATION src/lean/store/plan_tasks.rs:40: callable dependency_reports is 36 lines (limit 29)
VIOLATION src/lean/store/plan_tasks.rs:77: callable plan is 57 lines (limit 29)
VIOLATION src/lean/store/sessions_and_plans.rs:108: callable interrupt_unfinished_tasks is 42 lines (limit 29)
VIOLATION src/lean/store/sessions_and_plans.rs:200: callable add_plan is 43 lines (limit 29)
VIOLATION src/lean/store/sessions_and_plans.rs:63: callable interrupt_running_turns is 44 lines (limit 29)
VIOLATION src/lean/tasks/prompts/requirements.rs:175: callable round_handoff_prompt is 48 lines (limit 29)
VIOLATION src/lean/tasks/prompts/review.rs:3: callable verifier_prompt is 34 lines (limit 29)
VIOLATION src/lean/tasks/prompts/review.rs:71: callable code_review_prompt is 36 lines (limit 29)
VIOLATION src/lean/tasks/prompts/worker.rs:57: callable upstream_note is 37 lines (limit 29)
VIOLATION src/lean/tasks/prompts/worker.rs:95: callable worker_prompt is 53 lines (limit 29)
VIOLATION src/lean/tasks/validation.rs:153: callable parse_progress is 35 lines (limit 29)
VIOLATION src/lean/tasks/validation.rs:86: callable settle is 47 lines (limit 29)
```

## Ownership note

The task-card scope lists `src/main.rs` and `src/transport.rs`, while the reviewed t2 ownership manifest assigns both to t5. Following the explicit instruction to follow that manifest, this worker left both paths untouched. The current diagnostic records the t5-owned transport violations `deserialize` (56 lines at line 16) and `read` (33 lines at line 119); main.rs has no current size violation.

## Behavioral fix record

- During the `session_events_with_messages` helper extraction, the focused `session_events_page_references_and_enforce_ownership` test hung because scope validation tried to reacquire the Store mutex while a database guard was live. The refactor now validates scope before taking that guard, then drops the query guard before reading messages. The same regression test passes (1 passed). This is an implementation correction required to preserve the pre-existing behavior, not an intentional API or persistence contract change.

## Characterization and validation

- Existing history unit tests were moved from inline `src/lean/history.rs` into `src/lean/history/tests.rs`, grouped as `mail_tests`, `round_tests`, and `recovery_tests`. They continue to cover user-message association and schema 11, session event/content paging and ownership, round attribution/idempotency, and rollback/restart recovery. Public-boundary integration tests remain in `tests/lean_core.rs`, `tests/lean_ledger.rs`, and `tests/lean_messages.rs`.
- Store unit tests moved from inline `src/lean/store.rs` into `src/lean/store/tests/group_1.rs` through `group_3.rs`; they retain schema compatibility, initialization atomicity, comment-claim expiry, output ordering, row/event serialization, recovery, and trust coverage (12 passed). Task unit tests moved from inline `src/lean/tasks.rs` into `src/lean/tasks/tests/group_1.rs` through `group_7.rs`; they retain parser, planning, option binding, verifier obligations, grapheme bounds, and feedback coverage (48 passed). The requirements runtime tests in `src/lean/ledger/runtime/tests.rs` cover atomic invalid-parent repair and source/interview/summary content and ordering (2 passed). Existing public-boundary assertions remain in `tests/lean_core.rs` (21 passed) and `tests/lean_ledger.rs` (25 passed).
- `cargo test --locked --lib history::tests`: 8 passed; `cargo test --locked --test lean_core`: 21 passed; `cargo test --locked --test lean_ledger`: 25 passed; `cargo test --locked --test lean_tui`: 110 passed. The focused `direct_orchestrator_message_uses_normal_workflow_and_validates_the_run` and `direct_agent_messages_resume_work_and_recheck_without_receipt_only_turns` tests passed.
- `cargo test --locked --test lean_messages` was characterized before changing mailbox code and repeated after the enqueue helper split; both runs had 30 passed and the same 4 failures: `agent_mail_reaches_the_orchestrator_without_replanning` (assertion expected 1, got 2), `deferred_options_comment_claim_expires_on_new_user_input`, `deferred_options_proposal_prefers_the_current_explicit_comment_list`, and `deferred_options_proposal_preserves_selective_comment_claim`. The initial characterization also repeated serially with the same failures. No changes were made to those behaviors or tests.
- In this repair round, `Core::open` moved to `core/open.rs` and now delegates state locking, orphan recovery, Core construction, and checkpoint restoration to focused helpers. Cut-off conversations and interrupted plans are retained and resumed from the same recovery results. `Core::submit_addressed` moved to `core/messages.rs`, where recipient parsing and direct, broadcast, task, and rejection paths are separated; routing, mail fields, copies, events, and response JSON are preserved. No intended API or behavior changes were made. `cargo test --locked --test lean_core` passed (21) after the startup extraction. `cargo test --locked --test lean_messages` after message extraction had 30 passed and the same four failures listed above; the focused `direct_orchestrator_message_uses_normal_workflow_and_validates_the_run` test passed.
- Ledger storage now separates entry-row decoding, source text loading, evidence validation/status calculation, query filtering, and transactional change application. The change transaction still performs validation, version and relation writes, affected task invalidation, revision update, decision insertion, event emission, optional resume enqueue, and commit in one transaction. No API, serialization, transaction, event, or recovery changes were intended. `cargo test --locked --test lean_ledger` passed (25) after these changes.
- Latest checks in this repair: `cargo fmt`, `cargo check --locked`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked --test lean_core` (21 passed), `cargo test --locked --test lean_ledger` (25 passed), `cargo test --locked --lib lean::tasks` (48 passed), `cargo test --locked --lib lean::reliability_tests` (5 passed), `cargo test --locked --lib lean::plan_tests` (2 passed), and `git diff --check` passed. The raw size checker was rebuilt and run against current source; it exits 1 with 293 repository violations, 0 classification errors, and 39 t4-owned violations. `src/lean/mod.rs` is below 299 lines, but four extracted Core files and 35 owned callables remain over their limits. The selected V2 work is still incomplete. No intentional behavior change was made; this repair did not fix a reported behavior bug.
- Earlier current-worktree checks included `cargo fmt --check`, `cargo check --locked`, `cargo clippy --locked --all-targets -- -D warnings`, and `git diff --check`. For the latest requirements extraction refactor, `cargo fmt --check`, `CARGO_TARGET_DIR=.antislop/target cargo check --locked`, `cargo clippy --locked --all-targets -- -D warnings`, and `cargo test --locked --lib lean::ledger::runtime::tests` (2 passed) passed. Earlier focused suites recorded task unit tests (48), Store unit tests (12), `lean_core` (21), `lean_ledger` (25), and `lean_tui` (110). `agent_mail_reaches_the_orchestrator_without_replanning` still fails with expected 1, got 2; the same failure was recorded before the mailbox refactor and after the earlier enqueue split. The current diagnostic has 351 repository violations and 97 in t4-owned paths. The earlier 103, 101, 100, 354, and 353 counts were stale.
- In the previous repair round, `cargo test --locked --test lean_plan_history` passed (4 tests), and the focused task unit suite passed (48 tests). `cargo test --locked --test lean_plan` reproducibly failed serially in `malformed_plan_json_is_corrected_automatically_with_a_finite_retry_limit`, `orchestrator_extends_a_running_graph_before_the_pending_verification`, and `repairing_json_after_options_reuses_requirements_and_the_comparison`. Before changes in this repair round, I repeated the full serial `lean_plan` suite and observed the same three failures. The test file `tests/lean_plan.rs` and the implementation file `src/lean/mod.rs` are byte-identical to base commit `77d51af1f4739b1f86ee552888b71fe8154c4f29` (`git diff --quiet` returned 0 for both), so the malformed-JSON and post-options failures execute unchanged test and core orchestration code from the pre-t4 implementation. To characterize the graph-extension failure against the pre-extraction implementation, I temporarily substituted `src/lean/graph_edit.rs` from that base commit, ran the focused test, and restored the current file; the same assertion failed (`left: 2`, `right: 1`). Thus all three failures reproduce without t4's implementation changes. The refreshed diagnostic exits 1 with 97 t4-owned violations; the remaining core and persistence size work is incomplete.
- In the previous repair round, `requirements_turn` was split into a short entrypoint and private helpers for progress events, turn setup, source/revision capture, agent execution, failure reporting, and ledger-turn binding. In this round, `ensure_requirements` was divided into source preparation, extraction analysis and validation, proposal application, and independent review helpers, with prompt text, retry behavior, approval checks, and the integration lock preserved. The helper module is `src/lean/ledger/runtime/requirements/extraction.rs`. `full_ledger_sources` was shortened by extracting source rendering, interview-question context lookup, and interview-summary rendering; its focused test asserts content and ordering. `apply_user_ledger_change` delegates exact proposal application and affected-task notifications through `runtime/ledger_change.rs` while holding the same integration lock through the same critical section. No API, serialization, transaction, event, or recovery changes are intended. The size diagnostic reports 351 repository violations, 0 classification errors, and 97 t4-owned violations. The required size refactor remains incomplete.
- In this repair round, `compliance_audit` moved into `runtime/compliance.rs`. Audit scope capture, analyst parsing, independent verification, scope revalidation, evidence recording, review recording, and completion reporting now have separate responsibilities. The integration mutex still spans revalidation through persistence and completion, and the same audit prompts, evidence validation, events, and store calls remain. Ledger graph validation and impact calculation were decomposed into named helpers without changing checks, dependency closure, or result ordering. No intentional behavior or persistence changes were made. `cargo fmt --check`, `cargo check --locked`, and `cargo clippy --locked --all-targets -- -D warnings` passed. `cargo test --locked --lib lean::ledger::runtime::tests` passed (2); `cargo test --locked --test lean_ledger` passed (25); `cargo test --locked --test lean_core` passed (21). The refreshed size diagnostic reports 348 repository violations, 0 classification errors, and 94 t4-owned violations. The required size refactor remains incomplete: `src/lean/mod.rs` is still 5,761 lines, and callable overages remain in ledger storage, Store, Tasks, and other owned modules.

- In this repair round, `src/lean/ledger/storage.rs` was reduced from 1,572 lines to 197 lines. Cohesive Store APIs and ledger validation/impact logic now live in private `storage/` modules. Public method paths and signatures, SQL, transaction boundaries, events, and recovery operations were preserved. No intentional behavior or persistence changes were made. `cargo test --locked --test lean_ledger` passed (25); `cargo test --locked --lib lean::ledger::runtime::tests` passed (2); `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, and `git diff --check` passed. `python3 tests/size_check/check.py` passed its checker fixtures. The full diagnostic still exits 1 with 347 repository violations and 89 t4-owned violations; 17 t4 ledger storage callable overages remain, along with the oversized `src/lean/mod.rs` and violations in Store and Tasks. No behavior bug was changed or fixed in this round.
