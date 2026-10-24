# Current initial size inventory

This replaces the stale inventory for the baseline used by the current plan. The prior HEAD `9b51fe4d15364017a3cb30a9c8a90e42ab6f8bdb` record is retained in [historical-inventory-9b51fe4.md](historical-inventory-9b51fe4.md).

## Snapshot

- HEAD: `892e93d5c5b03a4da54a7c21c890acc6a0783192`.
- Worktree: clean before this inventory capture; no production files changed during measurement.
- Checker source: built from this HEAD with `CARGO_TARGET_DIR="$PWD/.antislop/target" cargo build --locked --bin diagnostic-size-check`.
- Executable: `.antislop/target/debug/diagnostic-size-check`, SHA-256 `a4c3f945161be5da130921a14213cdedfb737d1afd6bf3c77bb9ca7ae31786c3`.
- Production Rust source set: 60 paths (`src/**/*.rs` plus the path-linked `tools/size_check/checker.rs`), combined path-and-content SHA-256 `f74c2e7427453720dd3d9c1cee18880d4380fcc399f85e22565a60a2444dfd52`.
- Hash construction: Rust paths sorted lexically; each path and file content is prefixed by its 8-byte big-endian length before being added to SHA-256.
- Main source diagnostic: 59 `src/` Rust files, 0 classification errors, 380 violations (38 file and 342 callable). Raw output is [initial-diagnostic-run.txt](initial-diagnostic-run.txt), SHA-256 `ada41edec091156a49e98d350b969bf49f1f33c7b2ff52d8f2acb2b837cc4bff`.
- Linked checker diagnostic: the same source-built executable measured `checker.rs` through a temporary in-project root linking it as a production module; 0 classification errors, 16 violations (1 file and 15 callables). Raw output is [initial-checker-diagnostic-run.txt](initial-checker-diagnostic-run.txt), SHA-256 `65301d1c7f80fcac2dc3fe0c1b3487bf2b5c5ba7a8190aff02078f429e72e69b`.
- Combined baseline: 60 production files, 39 oversized files, 357 oversized callables, 396 violations, 0 classification errors. Owners are assigned using the plan ownership manifest.

## Oversized files

| Path | Physical lines | Owner |
|---|---:|---|
| `src/config.rs` | 645 | t4 |
| `src/lean/agent.rs` | 1878 | t5 |
| `src/lean/cli.rs` | 569 | t5 |
| `src/lean/daemon.rs` | 3252 | t5 |
| `src/lean/git.rs` | 1066 | t5 |
| `src/lean/history.rs` | 1134 | t4 |
| `src/lean/ledger/runtime.rs` | 628 | t4 |
| `src/lean/ledger/storage.rs` | 1517 | t4 |
| `src/lean/mail.rs` | 414 | t4 |
| `src/lean/mod.rs` | 5761 | t4 |
| `src/lean/native.rs` | 1648 | t5 |
| `src/lean/native/codex.rs` | 1402 | t5 |
| `src/lean/native/opencode.rs` | 1012 | t5 |
| `src/lean/native/opencode/stream.rs` | 577 | t5 |
| `src/lean/rounds.rs` | 1218 | t4 |
| `src/lean/run_context.rs` | 384 | t4 |
| `src/lean/store.rs` | 2986 | t4 |
| `src/lean/tasks.rs` | 3523 | t4 |
| `src/lean/terminal.rs` | 1109 | t5 |
| `src/lean/tui/config.rs` | 748 | t6 |
| `src/lean/tui/details.rs` | 2490 | t6 |
| `src/lean/tui/diffview.rs` | 573 | t6 |
| `src/lean/tui/document.rs` | 861 | t6 |
| `src/lean/tui/editor.rs` | 870 | t6 |
| `src/lean/tui/graph.rs` | 2443 | t6 |
| `src/lean/tui/inspection.rs` | 370 | t6 |
| `src/lean/tui/interview.rs` | 347 | t6 |
| `src/lean/tui/ledger.rs` | 1451 | t6 |
| `src/lean/tui/mod.rs` | 6802 | t6 |
| `src/lean/tui/native_view.rs` | 865 | t6 |
| `src/lean/tui/palette.rs` | 894 | t6 |
| `src/lean/tui/planview.rs` | 1113 | t6 |
| `src/lean/tui/session.rs` | 514 | t6 |
| `src/lean/tui/taskinfo.rs` | 823 | t6 |
| `src/lean/tui/theme.rs` | 696 | t6 |
| `src/lean/tui/unblock.rs` | 361 | t6 |
| `src/lean/tui/view.rs` | 3342 | t6 |
| `src/lean/workflow.rs` | 362 | t4 |
| `tools/size_check/checker.rs` | 1145 | t3 |

## Oversized callables

Spans are inclusive physical source lines. Prefixes, blank lines, signatures, bodies, comments and braces follow the current checker contract. The nested checker helper `walk` and nested visitor method are separate callable declarations.

| Path | Callable | Inclusive span | Lines | Owner |
|---|---|---:|---:|---|
| `src/config.rs` | `resolve` | 282–331 | 50 | t4 |
| `src/config.rs` | `resolve_command` | 225–256 | 32 | t4 |
| `src/config/validation.rs` | `toml_diagnostic` | 31–80 | 50 | t4 |
| `src/config/validation.rs` | `validate_profiles` | 131–183 | 53 | t4 |
| `src/config/validation.rs` | `validate_routing` | 185–221 | 37 | t4 |
| `src/lean/agent.rs` | `agy` | 808–864 | 57 | t5 |
| `src/lean/agent.rs` | `claude` | 865–947 | 83 | t5 |
| `src/lean/agent.rs` | `codex` | 729–807 | 79 | t5 |
| `src/lean/agent.rs` | `command` | 329–478 | 150 | t5 |
| `src/lean/agent.rs` | `gemini` | 948–1027 | 80 | t5 |
| `src/lean/agent.rs` | `opencode` | 1028–1109 | 82 | t5 |
| `src/lean/agent.rs` | `reap` | 496–538 | 43 | t5 |
| `src/lean/agent.rs` | `redact_error` | 609–647 | 39 | t5 |
| `src/lean/agent.rs` | `run` | 132–164 | 33 | t5 |
| `src/lean/agent.rs` | `run_process_with_timeout` | 190–316 | 127 | t5 |
| `src/lean/cli.rs` | `chat` | 208–332 | 125 | t5 |
| `src/lean/cli.rs` | `describe` | 342–439 | 98 | t5 |
| `src/lean/cli.rs` | `ensure_trusted` | 144–181 | 38 | t5 |
| `src/lean/cli.rs` | `parse` | 60–104 | 45 | t5 |
| `src/lean/cli.rs` | `review_lines` | 441–492 | 52 | t5 |
| `src/lean/daemon.rs` | `call` | 2951–2989 | 39 | t5 |
| `src/lean/daemon.rs` | `cleanup_config_temporaries` | 2663–2709 | 47 | t5 |
| `src/lean/daemon.rs` | `connect` | 2859–2949 | 91 | t5 |
| `src/lean/daemon.rs` | `from` | 676–1151 | 476 | t5 |
| `src/lean/daemon.rs` | `from` | 1155–1644 | 490 | t5 |
| `src/lean/daemon.rs` | `handle` | 1788–2517 | 730 | t5 |
| `src/lean/daemon.rs` | `handle_events` | 2551–2591 | 41 | t5 |
| `src/lean/daemon.rs` | `prepare` | 2712–2749 | 38 | t5 |
| `src/lean/daemon.rs` | `serve` | 1710–1786 | 77 | t5 |
| `src/lean/daemon.rs` | `stop` | 2821–2857 | 37 | t5 |
| `src/lean/diff.rs` | `parse` | 93–176 | 84 | t4 |
| `src/lean/git.rs` | `add_worktree` | 185–215 | 31 | t5 |
| `src/lean/git.rs` | `changed_paths` | 397–430 | 34 | t5 |
| `src/lean/git.rs` | `combine` | 291–348 | 58 | t5 |
| `src/lean/git.rs` | `commit_paths` | 92–127 | 36 | t5 |
| `src/lean/git.rs` | `integrate_scoped` | 462–589 | 128 | t5 |
| `src/lean/git.rs` | `snapshot` | 32–90 | 59 | t5 |
| `src/lean/git.rs` | `working_tree_fingerprint` | 231–263 | 33 | t5 |
| `src/lean/graph_edit.rs` | `extend_running_graph` | 11–152 | 142 | t4 |
| `src/lean/history.rs` | `enqueue_agent_message` | 93–210 | 118 | t4 |
| `src/lean/history.rs` | `handoff_rounds` | 652–690 | 39 | t4 |
| `src/lean/history.rs` | `history_content` | 227–266 | 40 | t4 |
| `src/lean/history.rs` | `history_messages` | 268–343 | 76 | t4 |
| `src/lean/history.rs` | `invalidate_dependents` | 596–629 | 34 | t4 |
| `src/lean/history.rs` | `session_events_with_messages` | 541–593 | 53 | t4 |
| `src/lean/history.rs` | `task_round_with_references` | 446–527 | 82 | t4 |
| `src/lean/history.rs` | `task_rounds` | 400–440 | 41 | t4 |
| `src/lean/interview.rs` | `parse` | 39–83 | 45 | t4 |
| `src/lean/ledger/options.rs` | `recover_options_comparisons` | 108–144 | 37 | t4 |
| `src/lean/ledger/options.rs` | `save_solution_choice` | 161–215 | 55 | t4 |
| `src/lean/ledger/options.rs` | `validate` | 65–104 | 40 | t4 |
| `src/lean/ledger/runtime.rs` | `apply_user_ledger_change` | 38–83 | 46 | t4 |
| `src/lean/ledger/runtime.rs` | `compliance_audit` | 412–494 | 83 | t4 |
| `src/lean/ledger/runtime.rs` | `ensure_requirements` | 220–390 | 171 | t4 |
| `src/lean/ledger/runtime.rs` | `full_ledger_sources` | 183–219 | 37 | t4 |
| `src/lean/ledger/runtime.rs` | `requirements_turn` | 96–182 | 87 | t4 |
| `src/lean/ledger/storage.rs` | `add_ledger_evidence` | 985–1073 | 89 | t4 |
| `src/lean/ledger/storage.rs` | `apply_ledger_change_as` | 619–754 | 136 | t4 |
| `src/lean/ledger/storage.rs` | `bind_ledger_turn` | 1300–1343 | 44 | t4 |
| `src/lean/ledger/storage.rs` | `bind_task_ledger` | 847–906 | 60 | t4 |
| `src/lean/ledger/storage.rs` | `checked_graph` | 102–152 | 51 | t4 |
| `src/lean/ledger/storage.rs` | `create_run` | 275–308 | 34 | t4 |
| `src/lean/ledger/storage.rs` | `enqueue_requirements_resume` | 1484–1517 | 34 | t4 |
| `src/lean/ledger/storage.rs` | `entries` | 27–65 | 39 | t4 |
| `src/lean/ledger/storage.rs` | `impact` | 153–210 | 58 | t4 |
| `src/lean/ledger/storage.rs` | `ledger_diff` | 755–789 | 35 | t4 |
| `src/lean/ledger/storage.rs` | `ledger_statuses` | 1084–1189 | 106 | t4 |
| `src/lean/ledger/storage.rs` | `list_ledger` | 1190–1223 | 34 | t4 |
| `src/lean/ledger/storage.rs` | `propose_ledger_change_as` | 567–597 | 31 | t4 |
| `src/lean/ledger/storage.rs` | `record_ledger_review` | 907–979 | 73 | t4 |
| `src/lean/ledger/storage.rs` | `register_document_source` | 448–502 | 55 | t4 |
| `src/lean/ledger/storage.rs` | `register_message_source` | 410–447 | 38 | t4 |
| `src/lean/ledger/storage.rs` | `requirements_gaps` | 1224–1261 | 38 | t4 |
| `src/lean/ledger/storage.rs` | `source_text` | 66–101 | 36 | t4 |
| `src/lean/ledger/storage.rs` | `validate_change` | 211–272 | 62 | t4 |
| `src/lean/mail.rs` | `answer_in_background` | 149–221 | 73 | t4 |
| `src/lean/mail.rs` | `answer_waiting` | 223–397 | 175 | t4 |
| `src/lean/mail.rs` | `mail_address_task` | 75–107 | 33 | t4 |
| `src/lean/mail.rs` | `ready_mail` | 109–147 | 39 | t4 |
| `src/lean/mod.rs` | `blocked_report` | 5296–5351 | 56 | t4 |
| `src/lean/mod.rs` | `comment_at` | 793–842 | 50 | t4 |
| `src/lean/mod.rs` | `comment_label` | 5173–5246 | 74 | t4 |
| `src/lean/mod.rs` | `conversations` | 693–754 | 62 | t4 |
| `src/lean/mod.rs` | `deliver` | 4248–4356 | 109 | t4 |
| `src/lean/mod.rs` | `dependency_tasks` | 4364–4395 | 32 | t4 |
| `src/lean/mod.rs` | `diff` | 1094–1131 | 38 | t4 |
| `src/lean/mod.rs` | `finish_in_place` | 4842–4903 | 62 | t4 |
| `src/lean/mod.rs` | `freshness_for_task` | 990–1027 | 38 | t4 |
| `src/lean/mod.rs` | `in_place_user_edits` | 4807–4840 | 34 | t4 |
| `src/lean/mod.rs` | `integrate` | 4505–4680 | 176 | t4 |
| `src/lean/mod.rs` | `land` | 5089–5129 | 41 | t4 |
| `src/lean/mod.rs` | `open` | 266–393 | 128 | t4 |
| `src/lean/mod.rs` | `orchestrate` | 2302–2341 | 40 | t4 |
| `src/lean/mod.rs` | `orchestrator_prompt` | 5353–5402 | 50 | t4 |
| `src/lean/mod.rs` | `orchestrator_turn` | 2369–2701 | 333 | t4 |
| `src/lean/mod.rs` | `plan_report` | 4940–5061 | 122 | t4 |
| `src/lean/mod.rs` | `propose` | 2767–2953 | 187 | t4 |
| `src/lean/mod.rs` | `record_check` | 3385–3414 | 30 | t4 |
| `src/lean/mod.rs` | `recover_task_as` | 1392–1961 | 570 | t4 |
| `src/lean/mod.rs` | `reject_plan` | 1284–1321 | 38 | t4 |
| `src/lean/mod.rs` | `repair_integration` | 4682–4805 | 124 | t4 |
| `src/lean/mod.rs` | `repair_writer_base` | 4422–4484 | 63 | t4 |
| `src/lean/mod.rs` | `review` | 2955–3004 | 50 | t4 |
| `src/lean/mod.rs` | `review_change` | 1133–1234 | 102 | t4 |
| `src/lean/mod.rs` | `revise_running_graph` | 2703–2765 | 63 | t4 |
| `src/lean/mod.rs` | `run_plan` | 3454–3603 | 150 | t4 |
| `src/lean/mod.rs` | `run_task` | 3605–4190 | 586 | t4 |
| `src/lean/mod.rs` | `start_plan_with_scope_lock` | 2131–2252 | 122 | t4 |
| `src/lean/mod.rs` | `stop_task` | 4905–4938 | 34 | t4 |
| `src/lean/mod.rs` | `submit_addressed` | 434–625 | 192 | t4 |
| `src/lean/mod.rs` | `turn_inner` | 3017–3383 | 367 | t4 |
| `src/lean/mod.rs` | `unblock_by_orchestrator` | 2009–2082 | 74 | t4 |
| `src/lean/mod.rs` | `unblock_dependent_tasks` | 1963–2007 | 45 | t4 |
| `src/lean/mod.rs` | `warn_of_uncommitted` | 2254–2287 | 34 | t4 |
| `src/lean/native.rs` | `capable_in_path` | 301–372 | 72 | t5 |
| `src/lean/native.rs` | `drop` | 786–815 | 30 | t5 |
| `src/lean/native.rs` | `enqueue` | 511–560 | 50 | t5 |
| `src/lean/native.rs` | `interrupt_turn` | 566–634 | 69 | t5 |
| `src/lean/native.rs` | `prepare_scoped_with` | 232–285 | 54 | t5 |
| `src/lean/native.rs` | `recover` | 818–867 | 50 | t5 |
| `src/lean/native.rs` | `restore_with` | 108–199 | 92 | t5 |
| `src/lean/native.rs` | `run` | 450–509 | 60 | t5 |
| `src/lean/native.rs` | `spawn` | 701–765 | 65 | t5 |
| `src/lean/native.rs` | `start` | 375–416 | 42 | t5 |
| `src/lean/native/codex.rs` | `check_steer_target` | 433–466 | 34 | t5 |
| `src/lean/native/codex.rs` | `connection` | 468–672 | 205 | t5 |
| `src/lean/native/codex.rs` | `delivered_turn` | 394–431 | 38 | t5 |
| `src/lean/native/codex.rs` | `gateway` | 339–369 | 31 | t5 |
| `src/lean/native/codex.rs` | `run` | 196–337 | 142 | t5 |
| `src/lean/native/codex.rs` | `start` | 125–194 | 70 | t5 |
| `src/lean/native/opencode.rs` | `raw` | 34–74 | 41 | t5 |
| `src/lean/native/opencode.rs` | `read` | 409–459 | 51 | t5 |
| `src/lean/native/opencode.rs` | `serve` | 218–400 | 183 | t5 |
| `src/lean/native/opencode.rs` | `start` | 92–161 | 70 | t5 |
| `src/lean/native/opencode/stream.rs` | `canonical` | 170–199 | 30 | t5 |
| `src/lean/native/opencode/stream.rs` | `feed` | 266–390 | 125 | t5 |
| `src/lean/native/opencode/stream.rs` | `run` | 55–143 | 89 | t5 |
| `src/lean/native/opencode/stream.rs` | `subscribe` | 12–53 | 42 | t5 |
| `src/lean/rounds.rs` | `round_handoff` | 135–261 | 127 | t4 |
| `src/lean/rounds.rs` | `session_for_turn` | 82–133 | 52 | t4 |
| `src/lean/run_context.rs` | `record_run_context` | 53–117 | 65 | t4 |
| `src/lean/store.rs` | `add_comment_at` | 1389–1453 | 65 | t4 |
| `src/lean/store.rs` | `add_draft_comment` | 1455–1487 | 33 | t4 |
| `src/lean/store.rs` | `add_mail_from` | 1341–1376 | 36 | t4 |
| `src/lean/store.rs` | `add_message` | 477–531 | 55 | t4 |
| `src/lean/store.rs` | `add_plan` | 991–1033 | 43 | t4 |
| `src/lean/store.rs` | `append_event` | 2011–2061 | 51 | t4 |
| `src/lean/store.rs` | `dependency_reports` | 1085–1120 | 36 | t4 |
| `src/lean/store.rs` | `events_since_budget` | 2067–2115 | 49 | t4 |
| `src/lean/store.rs` | `initialize` | 303–383 | 81 | t4 |
| `src/lean/store.rs` | `interrupt_running_turns` | 854–897 | 44 | t4 |
| `src/lean/store.rs` | `interrupt_unfinished_tasks` | 899–940 | 42 | t4 |
| `src/lean/store.rs` | `last_turn_for_roles` | 757–793 | 37 | t4 |
| `src/lean/store.rs` | `plan` | 1122–1178 | 57 | t4 |
| `src/lean/store.rs` | `plan_history` | 2390–2553 | 164 | t4 |
| `src/lean/store.rs` | `plans_page` | 2344–2388 | 45 | t4 |
| `src/lean/store.rs` | `resolve_listed_plan_comments` | 1603–1632 | 30 | t4 |
| `src/lean/store.rs` | `send_draft_comments` | 1489–1547 | 59 | t4 |
| `src/lean/store.rs` | `set_check` | 1180–1225 | 46 | t4 |
| `src/lean/store.rs` | `set_comment_status` | 1549–1596 | 48 | t4 |
| `src/lean/store.rs` | `set_session` | 680–713 | 34 | t4 |
| `src/lean/store.rs` | `start_turn` | 622–678 | 57 | t4 |
| `src/lean/store.rs` | `task_by_id` | 2186–2245 | 60 | t4 |
| `src/lean/store.rs` | `update_task` | 1243–1316 | 74 | t4 |
| `src/lean/tasks.rs` | `bound_cut` | 1162–1202 | 41 | t4 |
| `src/lean/tasks.rs` | `checklist` | 1550–1749 | 200 | t4 |
| `src/lean/tasks.rs` | `code_review_prompt` | 2042–2077 | 36 | t4 |
| `src/lean/tasks.rs` | `coverage` | 1344–1383 | 40 | t4 |
| `src/lean/tasks.rs` | `effect` | 1385–1419 | 35 | t4 |
| `src/lean/tasks.rs` | `orchestrator_instructions` | 1454–1548 | 95 | t4 |
| `src/lean/tasks.rs` | `parse_effect` | 115–166 | 52 | t4 |
| `src/lean/tasks.rs` | `parse_graph_edit` | 290–331 | 42 | t4 |
| `src/lean/tasks.rs` | `parse_progress` | 1067–1101 | 35 | t4 |
| `src/lean/tasks.rs` | `peers_of` | 526–568 | 43 | t4 |
| `src/lean/tasks.rs` | `render_plan_document` | 570–623 | 54 | t4 |
| `src/lean/tasks.rs` | `round_handoff_prompt` | 2337–2384 | 48 | t4 |
| `src/lean/tasks.rs` | `settle` | 1000–1046 | 47 | t4 |
| `src/lean/tasks.rs` | `upstream_note` | 1805–1841 | 37 | t4 |
| `src/lean/tasks.rs` | `validate` | 388–420 | 33 | t4 |
| `src/lean/tasks.rs` | `verifier_prompt` | 1974–2007 | 34 | t4 |
| `src/lean/tasks.rs` | `with_options` | 870–907 | 38 | t4 |
| `src/lean/tasks.rs` | `with_required_agents` | 754–868 | 115 | t4 |
| `src/lean/tasks.rs` | `worker_prompt` | 1843–1895 | 53 | t4 |
| `src/lean/terminal.rs` | `character` | 757–840 | 84 | t5 |
| `src/lean/terminal.rs` | `feed` | 636–756 | 121 | t5 |
| `src/lean/terminal.rs` | `feed` | 861–933 | 73 | t5 |
| `src/lean/terminal.rs` | `input` | 308–410 | 103 | t5 |
| `src/lean/terminal.rs` | `snapshot` | 227–306 | 80 | t5 |
| `src/lean/terminal.rs` | `spawn` | 111–225 | 115 | t5 |
| `src/lean/terminal.rs` | `terminal_open` | 461–532 | 72 | t5 |
| `src/lean/terminal.rs` | `terminal_resize` | 571–600 | 30 | t5 |
| `src/lean/tui/clipboard.rs` | `copy_with_fallback` | 20–56 | 37 | t6 |
| `src/lean/tui/clipboard.rs` | `native_copy` | 70–130 | 61 | t6 |
| `src/lean/tui/config.rs` | `begin_addition` | 429–468 | 40 | t6 |
| `src/lean/tui/config.rs` | `draw_with_theme` | 574–641 | 68 | t6 |
| `src/lean/tui/config.rs` | `emit_draft` | 306–358 | 53 | t6 |
| `src/lean/tui/config.rs` | `flatten` | 145–179 | 35 | t6 |
| `src/lean/tui/config.rs` | `key` | 501–561 | 61 | t6 |
| `src/lean/tui/config.rs` | `new` | 114–143 | 30 | t6 |
| `src/lean/tui/config.rs` | `validate_draft` | 251–287 | 37 | t6 |
| `src/lean/tui/details.rs` | `check_rows` | 2061–2126 | 66 | t6 |
| `src/lean/tui/details.rs` | `details_footer_hint` | 1916–1956 | 41 | t6 |
| `src/lean/tui/details.rs` | `draw` | 1653–1904 | 252 | t6 |
| `src/lean/tui/details.rs` | `draw_overview` | 1313–1651 | 339 | t6 |
| `src/lean/tui/details.rs` | `format_tabs` | 1958–2045 | 88 | t6 |
| `src/lean/tui/details.rs` | `load_task` | 274–314 | 41 | t6 |
| `src/lean/tui/details.rs` | `new` | 316–368 | 53 | t6 |
| `src/lean/tui/details.rs` | `records` | 66–109 | 44 | t6 |
| `src/lean/tui/details.rs` | `records` | 627–1311 | 685 | t6 |
| `src/lean/tui/details.rs` | `render_records` | 579–625 | 47 | t6 |
| `src/lean/tui/details.rs` | `summary` | 536–568 | 33 | t6 |
| `src/lean/tui/details.rs` | `update` | 124–155 | 32 | t6 |
| `src/lean/tui/diffview.rs` | `diff_header_with_reviewer` | 362–414 | 53 | t6 |
| `src/lean/tui/diffview.rs` | `draw` | 159–235 | 77 | t6 |
| `src/lean/tui/diffview.rs` | `draw_files` | 442–497 | 56 | t6 |
| `src/lean/tui/diffview.rs` | `draw_lines` | 237–359 | 123 | t6 |
| `src/lean/tui/diffview.rs` | `key` | 118–157 | 40 | t6 |
| `src/lean/tui/document.rs` | `append` | 110–157 | 48 | t6 |
| `src/lean/tui/document.rs` | `build_index` | 376–499 | 124 | t6 |
| `src/lean/tui/document.rs` | `deduplicate` | 159–205 | 47 | t6 |
| `src/lean/tui/document.rs` | `finish` | 207–246 | 40 | t6 |
| `src/lean/tui/document.rs` | `range_matches` | 502–550 | 49 | t6 |
| `src/lean/tui/document.rs` | `viewport_options` | 292–374 | 83 | t6 |
| `src/lean/tui/document.rs` | `wrap_range` | 552–630 | 79 | t6 |
| `src/lean/tui/editor.rs` | `compute_visual_lines` | 33–127 | 95 | t6 |
| `src/lean/tui/editor.rs` | `edit_key` | 147–178 | 32 | t6 |
| `src/lean/tui/editor.rs` | `find_byte_for_col` | 470–499 | 30 | t6 |
| `src/lean/tui/editor.rs` | `move_word_left` | 351–382 | 32 | t6 |
| `src/lean/tui/editor.rs` | `move_word_right` | 384–413 | 30 | t6 |
| `src/lean/tui/editor.rs` | `visual_cursor` | 501–549 | 49 | t6 |
| `src/lean/tui/graph.rs` | `card_lines` | 1178–1220 | 43 | t6 |
| `src/lean/tui/graph.rs` | `describe` | 1112–1166 | 55 | t6 |
| `src/lean/tui/graph.rs` | `draw_node` | 1276–1485 | 210 | t6 |
| `src/lean/tui/graph.rs` | `get` | 275–318 | 44 | t6 |
| `src/lean/tui/graph.rs` | `health` | 1036–1084 | 49 | t6 |
| `src/lean/tui/graph.rs` | `neighbour` | 811–842 | 32 | t6 |
| `src/lean/tui/graph.rs` | `new_with_viewport` | 370–800 | 431 | t6 |
| `src/lean/tui/graph.rs` | `path` | 1600–1635 | 36 | t6 |
| `src/lean/tui/graph.rs` | `render_with` | 963–1023 | 61 | t6 |
| `src/lean/tui/graph.rs` | `route` | 844–901 | 58 | t6 |
| `src/lean/tui/inspection.rs` | `append_session` | 225–309 | 85 | t6 |
| `src/lean/tui/inspection.rs` | `message_document` | 182–223 | 42 | t6 |
| `src/lean/tui/inspection.rs` | `run_snapshot` | 311–370 | 60 | t6 |
| `src/lean/tui/inspection.rs` | `task_snapshot` | 9–157 | 149 | t6 |
| `src/lean/tui/interview.rs` | `draw` | 129–265 | 137 | t6 |
| `src/lean/tui/interview.rs` | `key` | 51–127 | 77 | t6 |
| `src/lean/tui/ledger.rs` | `detail_lines` | 925–1086 | 162 | t6 |
| `src/lean/tui/ledger.rs` | `draw` | 1145–1408 | 264 | t6 |
| `src/lean/tui/ledger.rs` | `draw_form` | 840–924 | 85 | t6 |
| `src/lean/tui/ledger.rs` | `key` | 422–625 | 204 | t6 |
| `src/lean/tui/ledger.rs` | `mouse` | 626–742 | 117 | t6 |
| `src/lean/tui/ledger.rs` | `new` | 83–130 | 48 | t6 |
| `src/lean/tui/ledger.rs` | `new` | 266–303 | 38 | t6 |
| `src/lean/tui/ledger.rs` | `preview_lines` | 1088–1143 | 56 | t6 |
| `src/lean/tui/ledger.rs` | `reply` | 743–830 | 88 | t6 |
| `src/lean/tui/ledger.rs` | `spec` | 131–191 | 61 | t6 |
| `src/lean/tui/mod.rs` | `apply_update_inner` | 1576–2189 | 614 | t6 |
| `src/lean/tui/mod.rs` | `ascii` | 5224–5278 | 55 | t6 |
| `src/lean/tui/mod.rs` | `config_request` | 2890–2921 | 32 | t6 |
| `src/lean/tui/mod.rs` | `confirm_action` | 965–1040 | 76 | t6 |
| `src/lean/tui/mod.rs` | `details_key` | 3688–3941 | 254 | t6 |
| `src/lean/tui/mod.rs` | `diff_key` | 1094–1123 | 30 | t6 |
| `src/lean/tui/mod.rs` | `dispatch` | 2191–2338 | 148 | t6 |
| `src/lean/tui/mod.rs` | `draw` | 3999–4310 | 312 | t6 |
| `src/lean/tui/mod.rs` | `draw_chat` | 4418–4547 | 130 | t6 |
| `src/lean/tui/mod.rs` | `draw_graph` | 4831–4967 | 137 | t6 |
| `src/lean/tui/mod.rs` | `draw_main` | 4370–4416 | 47 | t6 |
| `src/lean/tui/mod.rs` | `draw_transcript` | 4549–4829 | 281 | t6 |
| `src/lean/tui/mod.rs` | `fetch_run` | 815–866 | 52 | t6 |
| `src/lean/tui/mod.rs` | `fetch_task` | 727–813 | 87 | t6 |
| `src/lean/tui/mod.rs` | `get_comment_draft` | 435–469 | 35 | t6 |
| `src/lean/tui/mod.rs` | `graph_key` | 3399–3491 | 93 | t6 |
| `src/lean/tui/mod.rs` | `handle_plan_action` | 2780–2832 | 53 | t6 |
| `src/lean/tui/mod.rs` | `header_line` | 2834–2872 | 39 | t6 |
| `src/lean/tui/mod.rs` | `is_confirmation_stale` | 921–963 | 43 | t6 |
| `src/lean/tui/mod.rs` | `key_inner` | 3001–3397 | 397 | t6 |
| `src/lean/tui/mod.rs` | `ledger_action` | 1453–1568 | 116 | t6 |
| `src/lean/tui/mod.rs` | `mouse` | 5009–5197 | 189 | t6 |
| `src/lean/tui/mod.rs` | `mouse_scroll` | 4969–5007 | 39 | t6 |
| `src/lean/tui/mod.rs` | `open` | 489–646 | 158 | t6 |
| `src/lean/tui/mod.rs` | `open_conversation_picker` | 3569–3655 | 87 | t6 |
| `src/lean/tui/mod.rs` | `open_project` | 2504–2534 | 31 | t6 |
| `src/lean/tui/mod.rs` | `open_projects_picker` | 2536–2573 | 38 | t6 |
| `src/lean/tui/mod.rs` | `open_unblock` | 2340–2384 | 45 | t6 |
| `src/lean/tui/mod.rs` | `paste` | 3943–3997 | 55 | t6 |
| `src/lean/tui/mod.rs` | `picker_key` | 2642–2730 | 89 | t6 |
| `src/lean/tui/mod.rs` | `recall_history` | 1125–1164 | 40 | t6 |
| `src/lean/tui/mod.rs` | `recover` | 5310–5356 | 47 | t6 |
| `src/lean/tui/mod.rs` | `refetch_stale_details` | 673–725 | 53 | t6 |
| `src/lean/tui/mod.rs` | `register_project` | 2432–2465 | 34 | t6 |
| `src/lean/tui/mod.rs` | `request_confirmation` | 889–919 | 31 | t6 |
| `src/lean/tui/mod.rs` | `run` | 2732–2778 | 47 | t6 |
| `src/lean/tui/mod.rs` | `run` | 5358–5517 | 160 | t6 |
| `src/lean/tui/mod.rs` | `set_graph_filter` | 3526–3567 | 42 | t6 |
| `src/lean/tui/mod.rs` | `submit` | 1205–1254 | 50 | t6 |
| `src/lean/tui/mod.rs` | `submit_agent_message` | 1256–1328 | 73 | t6 |
| `src/lean/tui/mod.rs` | `submit_comment` | 1330–1445 | 116 | t6 |
| `src/lean/tui/mod.rs` | `switch_project` | 2467–2502 | 36 | t6 |
| `src/lean/tui/mod.rs` | `test_app_with_client_and_updates` | 326–404 | 79 | t6 |
| `src/lean/tui/mod.rs` | `unblock_choice` | 2386–2416 | 31 | t6 |
| `src/lean/tui/native_view.rs` | `recover_native_input` | 401–446 | 46 | t6 |
| `src/lean/tui/native_view.rs` | `render` | 243–280 | 38 | t6 |
| `src/lean/tui/native_view.rs` | `start` | 45–225 | 181 | t6 |
| `src/lean/tui/native_view.rs` | `sync_native_views` | 283–371 | 89 | t6 |
| `src/lean/tui/palette.rs` | `draw_scroll` | 542–595 | 54 | t6 |
| `src/lean/tui/palette.rs` | `draw_with_cursor` | 277–365 | 89 | t6 |
| `src/lean/tui/palette.rs` | `items` | 164–271 | 108 | t6 |
| `src/lean/tui/palette.rs` | `lines` | 380–475 | 96 | t6 |
| `src/lean/tui/palette.rs` | `lines_for_width` | 477–520 | 44 | t6 |
| `src/lean/tui/planhistory.rs` | `draw` | 88–189 | 102 | t6 |
| `src/lean/tui/planhistory.rs` | `key` | 55–86 | 32 | t6 |
| `src/lean/tui/planview.rs` | `build_lines` | 132–183 | 52 | t6 |
| `src/lean/tui/planview.rs` | `draw` | 525–748 | 224 | t6 |
| `src/lean/tui/planview.rs` | `key` | 209–345 | 137 | t6 |
| `src/lean/tui/planview.rs` | `layout_rows` | 396–523 | 128 | t6 |
| `src/lean/tui/planview.rs` | `synthesize_document` | 751–790 | 40 | t6 |
| `src/lean/tui/sanitize.rs` | `push` | 26–92 | 67 | t6 |
| `src/lean/tui/selection.rs` | `mouse` | 156–188 | 33 | t6 |
| `src/lean/tui/selection.rs` | `select_word` | 190–223 | 34 | t6 |
| `src/lean/tui/selection.rs` | `update_snapshot` | 90–128 | 39 | t6 |
| `src/lean/tui/session.rs` | `look` | 77–116 | 40 | t6 |
| `src/lean/tui/session.rs` | `overview` | 276–370 | 95 | t6 |
| `src/lean/tui/session.rs` | `render` | 155–274 | 120 | t6 |
| `src/lean/tui/taskinfo.rs` | `card_meta` | 360–389 | 30 | t6 |
| `src/lean/tui/taskinfo.rs` | `coverage_full` | 270–328 | 59 | t6 |
| `src/lean/tui/taskinfo.rs` | `effect_pieces` | 186–216 | 31 | t6 |
| `src/lean/tui/theme.rs` | `card` | 554–602 | 49 | t6 |
| `src/lean/tui/theme.rs` | `clean` | 349–392 | 44 | t6 |
| `src/lean/tui/unblock.rs` | `choices_for` | 29–85 | 57 | t6 |
| `src/lean/tui/unblock.rs` | `draw` | 182–249 | 68 | t6 |
| `src/lean/tui/view.rs` | `apply` | 634–1700 | 1067 | t6 |
| `src/lean/tui/view.rs` | `main_footer_hint` | 2164–2211 | 48 | t6 |
| `src/lean/tui/view.rs` | `progress` | 1756–1814 | 59 | t6 |
| `src/lean/tui/view.rs` | `render_header_with` | 2046–2162 | 117 | t6 |
| `src/lean/tui/view.rs` | `report_progress` | 534–601 | 68 | t6 |
| `src/lean/tui/view.rs` | `stage` | 389–432 | 44 | t6 |
| `src/lean/tui/view.rs` | `target_name` | 1827–1871 | 45 | t6 |
| `src/lean/workflow.rs` | `approve_requirements` | 152–194 | 43 | t4 |
| `src/lean/workflow.rs` | `finish_interview` | 120–150 | 31 | t4 |
| `src/lean/workflow.rs` | `queue_plan_format_repair` | 15–59 | 45 | t4 |
| `src/transport.rs` | `deserialize` | 16–71 | 56 | t5 |
| `src/transport.rs` | `read` | 119–151 | 33 | t5 |
| `tools/size_check/checker.rs` | `callable` | 358–387 | 30 | t3 |
| `tools/size_check/checker.rs` | `cfg_attr_errors` | 255–291 | 37 | t3 |
| `tools/size_check/checker.rs` | `cfg_attr_token_errors` | 314–357 | 44 | t3 |
| `tools/size_check/checker.rs` | `collect` | 1116–1145 | 30 | t3 |
| `tools/size_check/checker.rs` | `imported_external_macros` | 721–759 | 39 | t3 |
| `tools/size_check/checker.rs` | `module_paths` | 1023–1054 | 32 | t3 |
| `tools/size_check/checker.rs` | `module_references` | 878–1005 | 128 | t3 |
| `tools/size_check/checker.rs` | `prefix_line_kinds` | 75–192 | 118 | t3 |
| `tools/size_check/checker.rs` | `run` | 1055–1115 | 61 | t3 |
| `tools/size_check/checker.rs` | `scan_file` | 827–877 | 51 | t3 |
| `tools/size_check/checker.rs` | `visit_item` | 489–534 | 46 | t3 |
| `tools/size_check/checker.rs` | `visit_item_mod` | 540–603 | 64 | t3 |
| `tools/size_check/checker.rs` | `visit_item_mod` | 886–915 | 30 | t3 |
| `tools/size_check/checker.rs` | `visit_macro` | 412–482 | 71 | t3 |
| `tools/size_check/checker.rs` | `walk` | 722–751 | 30 | t3 |

## Scope and classification

- `src/` is collected recursively, including unreferenced Rust files. The diagnostic binary path-links `tools/size_check/checker.rs`; the pre-change checker did not collect outside `src/`, so the exact source-built checker measured the linked file separately as a production module. The checker extension will make this path automatic.
- Test-only callable declarations are excluded from callable limits. Test-only files are excluded only when no production path reaches them. Mixed production/test files count all physical lines; no test span is subtracted.
- No generated-source exclusions are configured. Every unresolved or ambiguous classification is a gate failure.
- No production refactoring had started when this inventory was captured. The raw outputs above preserve the source-current pre-change evidence.

## Baseline checks

- `cargo build --locked --bin diagnostic-size-check` with `CARGO_TARGET_DIR` inside `.antislop/`: passed.
- Main inventory command: `.antislop/target/debug/diagnostic-size-check --root .`; expected exit 1 for size violations, 0 classification errors.
- Linked checker reproduction: copy the checker under `.antislop/checker-baseline/src/checker.rs`, make `src/lib.rs` path-link it, then invoke the same executable with `--root .antislop/checker-baseline`; expected exit 1 for 16 size violations, 0 classification errors.
- Current-worktree verification after the fixture additions: `python3 tests/size_check/check.py` passed, including the repository diagnostic capture; `cargo fmt --check` passed; `CARGO_TARGET_DIR=.antislop/target cargo check --locked` passed; and `CARGO_TARGET_DIR=.antislop/target cargo clippy --locked --all-targets -- -D warnings` passed. Cargo artifacts were kept under `.antislop/target`. These are observed current-worktree results, not claims about checks run against the historical pre-change snapshot.
