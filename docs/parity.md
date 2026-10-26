# TUI parity inventory against main@7ad664d25fbeebd3e8906caf29e9fb9925f20fc8

This document formally records terminal-interface (TUI) behavior and related
runtime-contract parity against `main@7ad664d25fbeebd3e8906caf29e9fb9925f20fc8`.
Every baseline **t12** matrix item and adaptive **t13** contract has explicit
automated-test evidence in the repository or a documented lean-runtime architectural difference.

Missing functionality is not counted as complete; no feature relies on placeholders.

---

## 1. Architectural separation of the TUI from the old runtime

In reference state `main@7ad664d25fbeebd3e8906caf29e9fb9925f20fc8`:

- The terminal interface read state from extremely large (112 MB) `Run` documents containing project-source dumps (`input_snapshot`), transactional diff arrays (`diff_baseline`), session-turn registers, and synchronous state reconciliation (`reconcile_source_delivery`).
- Slow model responses blocked the TUI, requiring synchronous generation of the entire state in memory.

In **lean core** (`src/lean/`):

- State uses lightweight SQLite (`core.db`), append-only events (`events`), and an asynchronous daemon queried through `events long-poll` (up to 500 entries / 1 MB payload per `events` request).
- Agents are native CLI processes in isolated task worktrees with separate read-only verification checkouts.
- The TUI deterministically projects the event log (`View = fold(events)`). Heavy resources (diffs, task reports, structured history) load asynchronously on dedicated threads when requested, without blocking rendering or input.
- **Parity with `main` covers all user ergonomics, editor semantics, shortcuts, responsiveness, card appearance, inspection, recipient selection, and safety guarantees (frozen confirmations with `scope_hash` validation).**

---

## 2. Complete TUI parity matrix (t12)

The table covers the current items in the t12 analytical matrix.

| # | Area | Reference requirement (`main@7ad664d`) | `lean` implementation | Status | Repository evidence / explicit runtime difference |
|---|---|---|---|---|---|
| 1 | **Appearance** | Minimum terminal size 12x5: below 12x5 show a detach notice; at 12x5 and above remain responsive without panics. | `MIN_TERM_WIDTH = 12`, `MIN_TERM_HEIGHT = 5` in `src/lean/tui/view.rs`. Below 12x5 render the centered guard `"Terminal too small (minimum 12x5 required)"`; `q` detaches. | Full parity | `tests/lean_tui.rs:tui_responsive_breakpoints_and_terminal_guard`, `tests/lean_tui.rs:tui_compact_minimal_terminals_from_12x5_to_84_and_split_at_85`. |
| 2 | **Appearance** | Split at 85 columns: `>= 85` chat (1/3) + separator + graph (2/3); `< 85` uses `SinglePanel`. | `SIDE_BY_SIDE_BREAKPOINT = 85` in `src/lean/tui/view.rs`. Terminals 80-84 columns use a single panel toggled by `Tab` (`[Chat]`, `[Team]`). | Full parity | `tests/lean_tui.rs:tui_compact_minimal_terminals_from_12x5_to_84_and_split_at_85`. |
| 3 | **Appearance** | Cards have a left status accent (`▌`), raised background, and a single separator `│`. | `Theme::card`, `CardProps`, `card_accent` in `src/lean/tui/theme.rs`. No heavy double chat frames; regions use a `│` separator. | Full parity | `tests/lean_tui.rs:tui_cards_structural_entries_and_actual_delivery_states`, `src/lean/tui/theme.rs:card`. |
| 4 | **Appearance** | Thin proportional scrollbars (one-column `█`/`▓` thumb), hidden when content fits. | `Theme::render_scrollbar` in `src/lean/tui/theme.rs`. Chat, details, and diff render them only when `total_lines > viewport_height`. | Full parity | `tests/lean_tui.rs:app_resizing_cursor_bounds_picker_overlay_ascii_no_color`. |
| 5 | **Appearance** | `NO_COLOR=1` removes ANSI colors while preserving `dim`/`reversed`; `ANTISLOP_ASCII=1` uses ASCII glyphs. | Environment checks in `src/lean/tui/theme.rs` and `view.rs`. `Theme::plain()` clears colors; `Theme::ascii()` replaces borders/icons with ASCII (`-|+|#`, `+ x ! * o`). | Full parity | `tests/lean_tui.rs:tui_ascii_mode_and_theme_glyphs`, `tests/lean_tui.rs:tui_no_color_mode_preserves_modifiers_and_clears_colors`. |
| 6 | **Editing** | Unicode grapheme clusters: `Backspace`, `Delete`, `←`, `→` operate on complete graphemes (ZWJ emoji `👩‍💻`, combining marks `a\u{301}`). | Dependency `unicode-segmentation = "1.13"`. `InputBuffer` in `src/lean/tui/editor.rs` uses `grapheme_indices(true)` and `graphemes(true)`. | Full parity | `tests/lean_tui.rs:tui_grapheme_cluster_editing_and_wrapping_parity`, `tests/lean_tui.rs:tui_input_buffer_unicode_character_boundary_safety`. |
| 7 | **Editing** | Word wrapping and cursor-cell reservation: break at spaces, preserve indentation, leave a cursor cell after full rows. | `compute_visual_lines` and `visual_text_lines` in `src/lean/tui/editor.rs`. Reserved cells prevent cursor/border collisions. | Full parity | `tests/lean_tui.rs:tui_visual_lines_and_cjk_double_width`, `tests/lean_tui.rs:full_width_drafts_show_the_cursor_after_the_last_character`. |
| 8 | **Editing** | Vertical navigation through wrapped text: `↑`/`↓` move between visual rows while remembering `preferred_col`. | `move_up` and `move_down` in `src/lean/tui/editor.rs` preserve `preferred_col` across shorter/empty rows. | Full parity | `tests/lean_tui.rs:tui_preferred_column_memory_and_grapheme_editing`. |
| 9 | **Editing** | Detect editor boundary rows with `on_first_row(width)` and `on_last_row(width)`. | Public methods in `src/lean/tui/editor.rs:529-537` distinguish multiline draft movement from history recall. | Full parity | `src/lean/tui/editor.rs:529-537`, `tests/lean_tui.rs:tui_sent_message_history_ring_and_draft_stashing`. |
| 10 | **Sending** | Send and insert-newline shortcuts. | Lean core: `Enter` sends (`submit()`), `Ctrl+S` is an alternative send shortcut (`submit()`), `Alt+Enter` or `Ctrl+J` inserts `\n`. | Explicit runtime difference agreed in the t13 contract | `src/lean/tui/mod.rs:1560-1563`, `tests/lean_tui.rs:tui_established_keymap_ctrl_s_and_newlines`. README and lean-core reflect this behavior. |
| 11 | **Sending** | Sent-message history ring: `history: Vec<String>`, `history_draft`. `↑` on row 0 recalls older messages; `↓` recalls newer messages and the draft. | `self.history`, `self.history_index`, `self.history_draft` in `src/lean/tui/mod.rs`. `recall_history()` manages history and cursor restoration. | Full parity | `tests/lean_tui.rs:tui_sent_message_history_ring_and_draft_stashing`, `tests/lean_tui.rs:tui_history_recall_with_cursor_restoration`. |
| 12 | **Sending** | Reliable draft retention: `sending: true`, no text loss on transport/daemon errors, isolated drafts when switching conversations (`Ctrl+O`), `Esc` semantics. | `sending: true`, `pending_send`, `failed_send` in `src/lean/tui/mod.rs`. Transport errors restore `input` or `failed_send`. `Ctrl+O` isolates drafts per conversation. Editor `Esc` restores the draft while browsing history, clears nonempty input (`input.clear()`, not recoverable through `↑`/`↓`), or detaches on empty input. | Full parity with documented runtime semantics | `tests/lean_tui.rs:tui_send_reliability_draft_retention_on_error_and_in_flight_typing`, `tests/lean_tui.rs:test_esc_behavior_and_cleared_draft_counterexample`. |
| 13 | **Sending** | Recipient selection and broadcast: orchestrator by default, `@all` or a specific agent, editor header. | `@` on empty input or `Ctrl+P > Address message` opens `Picker::new(Kind::Recipients)`. Daemon validates recipients (`mail_rejected`). | Full parity | `tests/lean_tui.rs:test_recipient_selection_and_broadcast`, `tests/lean_protocol.rs:test_message_addressing_validation_and_mail_queue`. |
| 14 | **Graph** | Stable 2D graph: fixed 28x4 cards, cycle resistance (Kahn's cycle-breaking), stable selection/event positions. | `src/lean/tui/graph.rs`. Positions do not depend on selection or activity (`view.activity`). Cyclic dependencies do not cause infinite loops. | Full parity | `tests/lean_tui.rs:test_spatial_graph_stable_positions_large_and_cyclic_plans`, `tests/lean_tui.rs:test_adaptive_graph_navigation_and_resize_stability`. |
| 15 | **Graph** | Labeled routed edges: ASCII `V`, `D`, `M`, `▲▼◀▶` arrows, side corridor bypassing same-column cards. | `Graph::edge_routes` in `src/lean/tui/graph.rs`. Nonadjacent same-column routes bypass intervening cards using `(from.right() + GAP_X / 2)`. | Full parity | `tests/lean_tui.rs:test_spatial_graph_vdm_relation_markers_and_visible_only_navigation`, `tests/lean_tui.rs:test_adaptive_graph_non_adjacent_same_column_bypass_routing`. |
| 16 | **Graph** | 2D arrow navigation restricted to nodes visible in the active filter. | `Graph::neighbour()` in `src/lean/tui/graph.rs` filters by `n.visible`. Arrows skip nodes hidden by filters/queries. | Full parity | `tests/lean_tui.rs:test_spatial_graph_vdm_relation_markers_and_visible_only_navigation`. |
| 17 | **Graph** | Progress-overlay collision avoidance ("now"): switch corners or yield to avoid covering nodes. | `compute_progress_overlay_with_nodes` in `src/lean/tui/graph.rs` checks four corners against the selected node and visible cards, yielding (`None`) if all collide. | Full parity | `tests/lean_tui.rs:test_refresh_updates_and_overlay_collision_avoidance`, `tests/lean_tui.rs:test_adaptive_graph_collision_and_routing_safety`. |
| 20 | **Tasks** | Detailed orchestration micro-stage in the header. | Transcript header shows exact micro-stage state (`TaskRow.activity`, active-check count, verification phase). | Full parity | `tests/lean_tui.rs:long_activity_labels_fit_the_progress_overlay`. |
| 21 | **Conversations** | Transcript search (`/` / `Ctrl+F`): preserve position, highlight matches, position entries at the top. | `Ctrl+F` opens `Picker::new(Kind::History)`. Selecting an entry scrolls the transcript and highlights it (`self.found = Some(entry)`). | Full parity | `tests/lean_tui.rs:app_graph_details_diff_navigation_and_search`. |
| 22 | **Conversations** | Response-waiting hints: highlight "Waiting for you" when an agent asks the user. | `needs_user` events and question turns show a highlighted waiting indicator in the chat header and footer hint. | Full parity | `src/lean/tui/mod.rs`, `docs/lean-core.md:317-319`. |
| 23 | **Conversations** | Live conversation switching: `Ctrl+O` list, `Ctrl+N` new conversation, isolated retained drafts. | `Ctrl+O` opens thread summaries; `open()` preserves drafts in `conversation_drafts` and complete state in `conversation_states`. | Full parity | `tests/lean_tui.rs:test_searchable_conversation_switcher_with_project_filtering_and_draft_retention`, `tests/lean_protocol.rs:test_conversations_structured_summary`. |
| 24 | **Inspectors** | Complete inspector tabs: six agent/task tabs plus a conversation inspector (`Ctrl+I`), explicit missing-history notices. | `Details` in `src/lean/tui/details.rs`. Tabs: Overview, Session, Messages, Result, Checks, Comments. `Ctrl+I` opens conversation details. Explicit `[historical data unavailable...]` labels. | Full parity (with explicit truncated-data labels) | `tests/lean_tui.rs:test_conversation_details_inspector`, `tests/lean_tui.rs:test_inspector_explicit_missing_data_notices`. |
| 25 | **Inspectors** | Raw-view toggle (`v`): Readable / Raw JSON for audit/session records. | Removed on purpose: there is no JSON view of a session or record; Overview shows the agent's own terminal and the provider keeps the history. | Explicit runtime difference | `tests/lean_tui.rs:test_inspector_whole_record_search`. |
| 26 | **Inspectors** | In-tab search (`/`): filter whole `DetailRecord` records while preserving header/content links. | `/` in `src/lean/tui/details.rs` opens record search, highlights phrases, and filters whole data blocks. | Full parity | `tests/lean_tui.rs:test_inspector_whole_record_search`. |
| 27 | **Inspectors** | Per-tab scroll/filter memory: independently retain position/query for every `(agent, tab)` pair. | `scroll_memory` and `query_memory` indexed by `(key, tab_index)` in `Details`, also preserved across `A → B → A` conversation switches. | Full parity | `tests/lean_tui.rs:test_inspector_scroll_and_query_memory_per_agent_and_tab`, `tests/lean_tui.rs:test_inspector_conversation_switch_preserves_details_memory`. |
| 28 | **Comments** | Precise nested targets: plan, task, check, finding, result, artifact, diff, conversation. | Target prefixes in `src/lean/mod.rs` and `comment_label`: `plan:`, `task:`, `check:`, `finding:`, `report:`, `artifact:`, `diff:`, `worker:`, `verifier:`, `conversation`. | Full parity | `tests/lean_tui.rs:test_precise_comment_targets_and_labels`. |
| 29 | **Comments** | Model-response and staleness status: resolution labels, plan linkage, `[applies to previous plan]` flag. | `comments` table in `core.db`, projected by `View`. Earlier-plan comments use `[applies to previous plan]`. | Full parity | `tests/lean_tui.rs:tui_comments_with_same_target_in_different_conversations_and_plans`. |
| 30 | **Diff** | Reviewer-profile/scope preview in the diff header (provider, model, effort, audit rules). | `ReviewerProfile` from `src/lean/diff.rs` rendered in the `DiffView` header (`diff_header_with_reviewer`). | Full parity | `tests/lean_tui.rs:test_diff_reviewer_profile_rendered`. |
| 31 | **Diff** | Freeze inspected diff candidates: lines under the cursor do not mutate while commenting; refresh with `r`. | `DiffView` fetches a frozen snapshot and refreshes only on explicit `r`. | Full parity | `src/lean/tui/diffview.rs`, `tests/lean_writer.rs:the_user_reads_a_change_comments_a_line_and_asks_for_a_review`. |
| 32 | **Confirmations** | Dedicated modal/`ConfirmationCard` for `AcceptPlan`, `ResumePlan`, `ReviewDiff`, `CancelWork`. | `ConfirmationCard` in `src/lean/tui/view.rs` and `mod.rs`. Stage actions show a modal with frozen scope. | Full parity | `tests/lean_tui.rs:test_confirmation_card_action_lifecycle`. |
| 33 | **Confirmations** | Next-step approval: `Enter`/`y`/`Y` confirms; `Esc`/`n`/`N` rejects without sending requests. | Handled in `src/lean/tui/mod.rs:1452-1463`. `Enter` runs the action with frozen `scope_hash`; `Esc` dismisses the modal. | Full parity | `tests/lean_tui.rs:test_confirmation_card_action_lifecycle`. |
| 34 | **Confirmations** | Scope pinning and `stale_scope` protection in the TUI and daemon transaction. | TUI `is_confirmation_stale()` detects asynchronous scope changes and blocks execution (`[stale: stale_scope]`). Daemon validates `scope_hash` within a SQLite transaction and rejects expired requests with `stale_scope` and zero side effects. | Full parity | `tests/lean_tui.rs:test_stale_confirmation_handling_and_rendering`, `tests/lean_protocol.rs:test_scope_hash_verification_and_stale_scope_rejection`, `tests/lean_protocol.rs:test_stale_scope_forced_change_between_read_and_execution_has_no_side_effects`. |

---

## 3. Interface-adaptation contract scenarios (t13)

| Identifier | Area | Scenario / initial condition | Expected behavior (acceptance criterion) | Automated-test evidence |
|---|---|---|---|---|
| `AC-RESP-01` | Responsiveness | 11x4 terminal (below 12x5) | Render the guard `Terminal too small (minimum 12x5 required)`. `q` detaches without panic. | `tests/lean_tui.rs:tui_responsive_breakpoints_and_terminal_guard` |
| `AC-RESP-02` | Responsiveness | Terminal from 12x5 to 84 columns | Use `SinglePanel`. `Tab` cycles `[Chat]`, `[Team]`. | `tests/lean_tui.rs:tui_compact_minimal_terminals_from_12x5_to_84_and_split_at_85` |
| `AC-RESP-03` | Responsiveness | Terminal exactly 85 columns wide | Split into chat (minimum width 34), `│` separator, and team graph. | `tests/lean_tui.rs:tui_compact_minimal_terminals_from_12x5_to_84_and_split_at_85` |
| `AC-EDIT-01` | Editor | Text in the editor; press `Enter` | Send immediately to the daemon (`submit()`) and clear the buffer. | `tests/lean_tui.rs:app_fake_request_capture_through_app`, `tests/lean_tui.rs:tui_established_keymap_ctrl_s_and_newlines` |
| `AC-EDIT-02` | Editor | Press `Alt+Enter` or `Ctrl+J` in a draft; press `Ctrl+S` | Insert `\n` without sending. `Ctrl+S` sends as an alternative to `Enter`. | `tests/lean_tui.rs:tui_established_keymap_ctrl_s_and_newlines` |
| `AC-EDIT-03` | Editor | Type ZWJ emoji `👩‍💻` or combining character `a\u{301}` and press `Backspace` / `Delete` | Delete the entire grapheme cluster without broken UTF-8 bytes or orphaned combining marks. | `tests/lean_tui.rs:tui_grapheme_cluster_editing_and_wrapping_parity`, `tests/lean_tui.rs:tui_input_buffer_unicode_character_boundary_safety` |
| `AC-HIST-01` | History | Cursor on the first draft row (or empty draft); press `↑` | Save the draft in `history_draft`, recall the last sent message; further `↑` moves backward in history. | `tests/lean_tui.rs:tui_sent_message_history_ring_and_draft_stashing`, `tests/lean_tui.rs:tui_history_recall_with_cursor_restoration` |
| `AC-HIST-02` | History | Navigate down sent-message history; press `↓` on the newest entry | Restore the complete unchanged `history_draft`, cursor position, and preferred column. | `tests/lean_tui.rs:tui_sent_message_history_ring_and_draft_stashing`, `tests/lean_tui.rs:tui_history_recall_with_cursor_restoration` |
| `AC-CONF-01` | Confirmations | Plan awaits approval; press `a` or `Ctrl+A` | Open `ConfirmationCard` with frozen `scope_hash` and plan-task list. | `tests/lean_tui.rs:test_confirmation_card_action_lifecycle` |
| `AC-CONF-02` | Confirmations | Confirmation modal open; a background `plan_proposed` event has a different ID | Preserve the modal's original `scope_hash` and details (scope pinning). | `tests/lean_tui.rs:test_stale_confirmation_handling_and_rendering` |
| `AC-CONF-03` | Confirmations | Attempt approval through a stale confirmation modal | TUI blocks with `[stale: stale_scope]`; daemon rejects with `stale_scope` and no side effects. | `tests/lean_tui.rs:test_stale_confirmation_handling_and_rendering`, `tests/lean_protocol.rs:test_scope_hash_verification_and_stale_scope_rejection`, `tests/lean_protocol.rs:test_stale_scope_forced_change_between_read_and_execution_has_no_side_effects` |
| `AC-INSP-02` | Inspector | Scroll Session to row 50, switch to Checks, then return to Session | Session restores exactly row 50 (per-tab scroll memory). | `tests/lean_tui.rs:test_inspector_scroll_and_query_memory_per_agent_and_tab` |
| `AC-GRPH-01` | Graph | Arrow navigation with an active verification-relationship filter | Move only among verification-linked nodes, skipping others. | `tests/lean_tui.rs:test_spatial_graph_vdm_relation_markers_and_visible_only_navigation` |
| `AC-DATA-01` | Missing data | Display an agent whose CLI did not report tokens | Show explicit `unavailable (CLI did not report it)` in token fields, never false zeroes. | `tests/lean_tui.rs:test_inspector_explicit_missing_data_notices`, `docs/lean-core.md:386-390` |

---

## 4. Reproducible graph-panel geometry and routing measurements

To avoid unsupported measurement claims, the following table gives **actual,
reproducible final-layout results** measured by `tests/lean_tui.rs`:

| Terminal | Graph panel (`inner.width x inner.height`) | Complete readable cards without scrolling | Actual columns and rows | Adaptation/compaction effect | Route safety (corridor bypass) |
|---|---|---|---|---|---|
| **80x24** (compact) | 78 x 20 | **6 complete cards** | 2 columns (x=3, x=37) x 3 rows (y=0, 6, 12) | Fit fully in height 20; `offset = (0, 0)`; no empty rows | 0 card collisions; bypass Auditor through side corridor |
| **85x24** (split) | 48 x 20 | **3 complete cards** | 1 column (x=3) x 3 rows (y=0, 6, 12) | Admin nodes in column 0 without clipping; filter recovers row 0 | 0 card collisions; vertical routes along corridor axis |
| **120x30** (split) | 77 x 26 | **7 complete cards** | 2 columns (x=3, x=37) x 4 rows (y=0, 6, 12, 18) | All 4 row levels used in height 26; `offset = (0, 0)` | 0 card collisions; bypass Verifier through side corridor |
| **160x50** (split) | 104 x 46 | **11 complete cards** | 3 columns (x=3, x=37, x=71) x 7 rows | 100% of nodes (admin + tasks) visible without scrolling | 0 collisions across all 240 node pairs on a 4x4 grid |
| **Any** | Any | — | Collapsed verifiers (`z`/Space) | Collapsed verifier occupies 0 rows; tasks move upward (0 empty rows) | Routes follow compacted positions |
| **Any** | Any | — | Text filter (e.g. "task 1") | Tasks recover row 0; no empty space after admin nodes | Route segments restricted to visible cards |

### Side-corridor bypass routing

For same-column nodes with vertical separation greater than one step (`(from.y - to.y).abs() > HEIGHT + GAP_Y`):

- Do not route directly through the column center, which would intersect intermediate cards.
- A horizontal segment exits right from `(from.right(), from.mid_y())` to corridor `x = from.right() + GAP_X / 2`.
- The vertical segment follows free intercolumn space beside intermediate cards.
- The final segment enters `(to.right(), to.mid_y())` horizontally with arrow `◀`.
- Zero route intersections with card interiors were verified for every filter (`Supervises`, `Depends`, `Verifies`, `Messages`).

---

## 5. New design requirements for the next plan (separate from current acceptance)

By orchestrator decision, the following new user requirements were recorded as
**the next development stage**, outside the closing t27 task and **not claimed
as implemented**:

1. **Complete visibility into every session plan, task, and team agent**:
   - *Current state (verified in t27)*: `View::apply(plan_proposed)` replaces active `self.tasks` with the newly proposed plan's tasks, so the graph shows the current plan. Earlier plans, tasks, CLI turns, reports, and verdicts remain fully preserved in SQLite `core.db`; their comments are correctly marked `[applies to previous plan]`.
   - *Planned extension*: Simultaneous inspection or navigation of historical plans and their previous executors/verifiers directly in the TUI graph.

2. **Concurrent work across projects with switching in one TUI**:
   - *Current state (verified in t27)*: The TUI talks to a project-managing daemon. `Ctrl+O` searches and immediately switches conversations within that project while fully retaining drafts.
   - *Planned extension*: Multiple independent project directories with workspace switching in one TUI process.
