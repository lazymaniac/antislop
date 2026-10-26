# Acceptance scenarios (design §18)

How each scenario of design §18 is met by the lean core. The kind of
evidence differs, and the difference matters:

- **runtime**: the runtime enforces it. A test with fake CLIs shows it
  (`tests/`, unit tests in `src/lean`).
- **prompt**: an agent's instructions or its verifier's numbered
  obligations ask for it (`src/lean/tasks.rs`). The runtime makes sure the
  verifier accounts for each obligation (scenario 30).

Where the lean core departs from the design on purpose (the user's
principles: risks only to paired verifiers, no copies of sources, state that
does not grow with the repository), the scenario says so. Full TUI parity
verification against `main@7ad664d25fbeebd3e8906caf29e9fb9925f20fc8` is in
[parity.md](parity.md).

| # | Scenario | How | Evidence |
| --- | --- | --- | --- |
| 1 | Small bugfix: plan, worker, verification | Orchestrator plan → worker/verifier pair → integration; *testing* checks need assertions that fail on the bug | runtime: `lean_plan::a_plan_runs_worker_verifier_pairs_and_reports_back`, `lean_writer::an_approved_writer_lands_in_the_project_without_a_commit`; prompt: checklist `testing` |
| 2 | Plan misses a dependency → blocked before implementation | Plan verifier reviews every plan before the user sees it; the runtime rejects self or forward dependencies | runtime: `lean_review::a_plan_the_verifier_sends_back_is_revised_before_the_user_sees_it`, `tasks::plans_with_unknown_dependencies_are_rejected` |
| 3 | A test without a meaningful assertion is rejected despite a green run | Testing verifier obligation C1 ("assertions that fail on the bug") | prompt |
| 4 | A duplicated module or a needless layer is named | Implementation/architecture obligations | prompt |
| 5 | Parallel tasks share a contract | Orchestrator orders tasks on the same files or contract; plan verifier checks the order; an integration task checks the whole change; integration lands one change at a time and never with conflict markers | runtime: `git::tests::user_edits_and_conflicts_leave_the_project_untouched`, `git::tests::a_dependent_result_applies_over_an_earlier_uncommitted_one`; prompt: team shapes |
| 6 | The user writes to a working agent; delivery state shown; the orchestrator knows | `@t2 text` → the worker's mailbox (delivered at its next turn, the state shown in the graph and details) and a copy for the orchestrator | runtime: `lean_messages::the_user_writes_to_a_working_agent_and_the_orchestrator_learns_of_it`, `view::messages_to_agents_show_their_delivery` |
| 7 | Supervised stops at the gate and goes on after consent | A plan waits for `accept` naming the plan the user saw; `/mode` switches a conversation for what it does next, never for what waits | runtime: `lean_recovery::accepting_runs_the_plan_the_user_saw_not_a_newer_one`, `lean_review::a_mode_change_applies_to_what_the_conversation_does_next` |
| 8 | Autonomous runs within policy; a missing capability is not bypassed | A checked plan starts by itself, and correction rounds go on, without a count (design §14); a plan that repeats one that ran, or a verifier repeating its findings, waits for the user; a plan whose review failed never runs by itself; without `local_write` a change stays on its branch | runtime: `lean_review::autonomous_mode_runs_a_checked_plan_and_reports_audit_gaps`, `…_plans_go_on_until_one_repeats_a_plan_that_ran`, `…_rounds_go_on_while_the_worker_gets_closer`, `…_verifier_repeating_its_findings_stops_the_rounds`, `…_review_failed_is_shown_but_never_runs_by_itself` |
| 9 | Closing the TUI does not stop the run; reattaching restores graph and chat | The daemon owns the work; frontends replay events | runtime: `lean_core::the_cli_talks_to_one_daemon_that_outlives_it`; the View is rebuilt from events (`view::the_view_follows_a_conversation_and_its_tasks`); TUI responsive layout (breakpoint 85, 12x5 guard), grapheme cluster editing & wrapping, Enter/Ctrl+S submission, history on draft boundaries, Escape draft stashing/detaching, draft recovery on error and across A→B→A switches, inspectors with raw view and query/scroll retention, recipient picker, pinned confirmations with stale_scope validation, adaptive graph layout with corridor bypass routing, compaction on fold/filter, and bounded canvas memory (`tests/lean_tui.rs`, `tests/lean_protocol.rs`, `src/lean/tui/`, [parity.md](parity.md)) |
| 10 | A daemon crash between an action and its record does not repeat it blindly | Turns left running become `interrupted`, tasks `cancelled`; resume continues the agents' sessions; integration notices a change that already landed | runtime: `lean_recovery::tasks_cut_off_by_a_daemon_restart_end_cancelled`, `git::tests::a_change_that_already_landed_is_not_landed_again`, `git::snapshot_tests::committing_paths_leaves_the_users_other_changes_alone` |
| 11 | Research with a non-existent or unsupportive source is not passed | Research obligations (every source exists and supports its claim) | prompt |
| 12 | Analysis with a wrong unit or denominator is corrected after recomputation | Analysis obligations (independent recomputation, units, denominators); an obligation marked `issue` returns to the worker | runtime: `lean_review::an_obligation_marked_issue_sends_its_note_to_the_worker`; prompt |
| 13 | A fix after a positive review is verified again | Every worker round is followed by its verifier; an approved task is not changed afterwards; later changes are new tasks with their own verifiers; the audit checks the end state | runtime: `lean_plan::a_plan_runs_worker_verifier_pairs_and_reports_back` (rejected → fixed → verified again) |
| 14 | A CLI needs a login or hits a limit → explicit waiting | The task becomes `waiting` with the CLI's message; resume continues in the same sessions | runtime: `lean_recovery::after_a_login_resuming_finishes_the_plan_in_the_same_sessions`, `tasks::login_and_limit_errors_wait_for_the_user` |
| 15 | Text from the repo, a page or an agent cannot widen permissions | Access comes from the task's `writes` and the policy, never from messages; a plan's scope outside the project reaches only read-only CLIs; writers work in a worktree (or, in place, one at a time with detection of the user's edits) | runtime: `lean_writer::an_approved_writer_lands_in_the_project_without_a_commit` (scope `../shared` withheld from the writer); prompt: project rules |
| 16 | After a reboot the right sessions resume by id, with pending approvals and roles | Sessions, plans, approvals and roles live in `core.db`; a session is resumed only by the CLI that created it | runtime: `lean_recovery::an_autonomous_plan_cut_off_by_a_restart_resumes_by_itself`, `lean_core::a_question_cut_off_by_a_restart_is_answered_after_it` |
| 17 | A TOML change does not silently change a running run | The daemon keeps the configuration it loaded; an idle one restarts on a change, a busy one keeps its work and the frontend says the change waits | runtime: `lean_core::a_changed_configuration_waits_for_running_work_and_says_so` |
| 18 | In-place keeps the user's changes; a conflict is reported, not overwritten | Snapshot commits before and after each worker turn; touching the user's files ends `needs_user` with the user's version recoverable. Uncommitted paths still matching their latest approved output remain available to subsequent writers; later user edits, unrelated files and rejected results remain protected | runtime: `lean_writer::in_place_a_writer_that_touches_the_users_edits_stops_and_keeps_their_version`, `…_edits_the_project_itself_and_reviewers_see_a_snapshot`, `…_chained_writers_can_extend_approved_uncommitted_results`, `…_ownership_survives_restart_but_later_user_edits_stay_protected`, `…_approval_does_not_claim_unrelated_user_files_in_its_snapshot`, `…_rejected_changes_do_not_become_approved_outputs` |
| 19 | A plan that assumes an intermediate state or order is blocked | Plan verifier: no intermediate state or ordering the system does not guarantee | prompt (`plan_review_prompt`, checklist `planning`) |
| 20 | An unknown replaced by an assumption is not let through | Orchestrator: unknowns are established by an earlier task; plan and context verifiers check it | prompt |
| 21 | An analyzer exits 0 with a warning: not clean | Static-analysis obligation: warnings count whatever the exit code | prompt |
| 22 | A suppression or baseline change hides a finding | Static-analysis obligation: no suppression, rule weakening or baseline change | prompt |
| 23 | A preexisting finding in scope does not pass as preexisting | Static-analysis obligation: findings in the agreed scope count; a scope doubt is named | prompt |
| 24 | Working code that leaves a needed refactor or a needless abstraction | Implementation obligation: no needed refactoring left undone; no needless layer | prompt |
| 25 | A structural problem in adjacent code | Autonomous: the writer may fix a small one with evidence and report it; supervised: it names the problem and the fix, the user decides | prompt (`worker_prompt`) |
| 26 | Well-kept code needs no cosmetic patch | Refactoring obligation: the report says why no change is needed | prompt |
| 27 | 99 of 100 obligations done | An independent auditor lists every obligation of the user's messages; a gap is a high finding and the orchestrator proposes a plan for it | runtime: `lean_review::autonomous_mode_runs_a_checked_plan_and_reports_audit_gaps` |
| 28 | A requirement with several qualifiers, a test checks one | Original source spans and qualifiers remain immutable; every mandatory criterion needs current integrated evidence | runtime: `lean_ledger::long_document_preserves_middle_negative_obligation_without_copying_document_into_sqlite`, `lean_ledger::completion_gate_requires_source_coverage_current_audit_and_integrated_evidence` |
| 29 | Extraction misses a requirement in the middle of the source | Both source-verification sessions receive full sources; a rejected compliance audit closes completion even after an earlier approval | runtime: `lean_ledger::compliance_sessions_verify_integrated_files_and_reopen_after_file_changes`; identifying semantic omissions remains the independent agents' responsibility |
| 30 | A report that skips a checklist item, a risk or an N/A reason is rejected | Risks, the common checks of design §7.6 and the kind's checklist are numbered obligations; a verdict must account for each with a status and a note; a gap is asked for once more, then the task waits for the user; an `issue` or `blocked` never approves; the Kontrole tab shows each obligation and its account | runtime: `lean_review::a_verdict_that_skips_an_obligation_is_asked_again_and_never_approves`, `tasks::a_verdict_counts_only_with_every_obligation_accounted_for` |
| 31 | A change of requirements after a review invalidates what depends on it | Atomic spec revisions invalidate dependent task generations and evidence; affected processes stop while independent work continues | runtime: `lean_ledger::semantic_edit_invalidates_dependency_closure_but_keeps_independent_tasks`, `lean_ledger::live_scope_edit_cancels_only_affected_process_and_rejects_late_task_updates` |
| 32 | One policy duplicated across adapters | Architecture obligation: no policy duplicated across modules | prompt |
| 33 | A pattern without a need | Architecture obligation: no pattern or layer without a present need | prompt |
| 34 | Three parameters valid alone break the contract together | Test-design obligation: interactions of three or more coupled parameters, not only pairs | prompt |
| 35 | A custom parser where a maintained library exists | Dependencies obligation: a maintained library considered first | prompt |
| 36 | Unresolved license terms in the real dependency graph | Dependencies obligation: licenses of the actual graph, vendored files included | prompt |
| 37 | A multi-licensed package with an acceptable option | Dependencies obligation: the chosen option and its obligations stated | prompt |
| 38 | A high-ranked variant misses an obligation | Options obligation: excluded before ranking | prompt |
| 39 | Sham alternatives or unequal benchmark conditions | Options and performance obligations: real alternatives under the same conditions | prompt |
| 40 | An experiment shows a better variant | Report the better variant or broken premise explicitly with evidence; the options selection remains binding unless a departure is explicitly justified. The verifier checks the implementation and departure evidence independently. No silent switch or automatic plan stop | prompt: options checklist and writer/verifier binding instructions; runtime with scripted CLIs: `lean_options::only_verifier_verdict_can_approve_worker_claims_and_departures` |

Requirements, task evidence and the requirements completion gate are
implemented as [versioned records](requirement-ledgers.md). The remaining
models listed below still use the lean obligation approach.

## Task-level options and evidence

Task-level options are separate from the global pre-plan `ensure_options`
comparison. Runtime normalization adds a read-only `WorkKind::Options` dependency
before every `writes=true` task lacking a direct options dependency, after optional
quality gates and even when `policy.required_gates=false`. Shared options are
reused and must cover each dependent writer. Startup configuration validation
always requires `routing.options` and valid executor/verifier profiles. Evidence:
`tasks::options_inherit_writer_context_and_precede_it`,
`tasks::options_reuse_existing_dependency_and_resolve_id_collisions`,
`finalization_adds_options_with_and_without_required_gates`, and
`lean_options::orchestrator_plan_is_completed_before_each_writer_runs`.

The options prompt requires 2–4 realistic alternatives, common criteria, exclusion
of requirement violations before ranking, an evidence-supported selection, and a
structured MCP selection. Only the chosen local approach is delivered to the writer and its verifier. See [execution contracts](lean-core.md).

## Deliberate departures from the design

- The design's ledgers (ContextModel, RiskCoverage, evidence graph,
  InputDomainMatrix, RefactorReport) are not separate records. Their checks
  are verifier obligations, the verdict's `coverage` is the risk coverage,
  and the agents read the project themselves. This keeps state independent
  of the repository's size.
- Risks go only to the verifier paired with the task, never to the worker.
- The risk catalog of design §19 is shared out among the kinds of work
  (`src/lean/risks/*.tsv`, one file per kind): a verifier gets as `R001`…
  obligations only the entries of its worker's kind, besides the task's own
  risks (`R1`…), the kind checklist (`C`) and the common checks (`V`). A
  verifier may name a risk outside them as a finding.
- A project without git is not turned into a repository by the runtime
  (design §11.1 creates one): read-only work runs without git, writers ask
  the user to run `git init` and commit first, so nothing of the user's is
  committed without them.
- Daemon requests carry no request id or idempotency key (design §9.3):
  a frontend never retries a request by itself, a plan is accepted by its id
  and only once, and pause or mode requests set a value rather than toggle
  it. Several frontends may control one conversation; each sees the others'
  actions as events.
- In supervised mode the first three correction rounds of a task run
  without asking, then the user decides (design §6.2 asks at each round).
  Autonomous mode has no count of rounds or plans, as design §14 says; it
  stops only when work repeats itself.

## Writer branch resume compatibility

New writer branches use the complete validated task ID. Existing worktrees may
retain either that name or the legacy `antislop/<first-eight-ID-bytes>` name;
an unrelated branch is an actionable error, not a candidate for adoption.
Both worktree reuse and destructive cleanup compare the actual branch with the
expected branch before proceeding. The `combine-` task-ID prefix is reserved
for helper branches; generated UUID task IDs are unaffected. A manually stored
ID using that prefix is rejected without changing its worktree or branch.
Regression evidence: `task_branches_validate_ids_and_distinguish_shared_prefixes`
and `resumed_worktrees_keep_legacy_branches_and_cleanup_the_actual_branch`.
This validates task branch names, not protection against concurrent external Git
mutations or proof of a branch creator's identity.
