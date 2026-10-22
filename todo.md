TODO list:

- [x] After submitting a task to antislop, the first step is a user interview, followed by writing a plan. Hide the agent graph at this stage.
- [x] Show the user's task plan as a full-screen document, with a cursor moving between lines and comments on any line. A fixed bottom bar provides Accept / Reject / Comment whole plan buttons.
- [x] Resolve or appropriately address comments added to the plan.
- [x] After addressing every user comment, turn the plan into tasks with dependencies and information about possible parallel execution.
- [x] Let the user inspect the plan at any time.
- [x] Consider linking tasks to specific plan sections and highlighting completed sections appropriately.
- [x] Agents should work in parallel where sensible.
- [x] The --project argument is unnecessary; familiar tools do not use it. The working directory is where antislop starts, usually the project root. No project-root detection is needed: the launch directory is supplied by the user.
- [x] Add first-use working-directory approval and trust, as in tools such as Codex.
- [x] Display an autonomous/supervised mode toggle in the UI.
- [x] Handle /pause /resume /continue /cancel like UI buttons.
- [x] The agent-details session view should emulate the agent process's terminal so users can inspect every operation and message it processes.
- [x] Support agent applications launched through commands supplied by other platforms. For example, use default Codex as orchestrator while launching workers/verifiers through commands such as omlx launch codex or ollama launch codex.
- [x] Translate every UI text into English.
- [x] Keeping effort maps in configuration is unnecessary. Applications have one reasoning-effort scale that should be hardcoded.
- [x] Require some predefined agent roles to ensure the best quality, leave code better than before, and fulfill every requirement.
- [x] Remove export functionality.
- [x] Support mouse/touchpad input in the UI.
- [x] Allow working on several projects at once.
- [x] Add word wrapping to the project-plan window so long lines are readable.
- [x] Mark agents with problems in red.
- [x] Provide a mechanism for unblocking inter-agent steps when a problem prevents progress.
- [x] The agent-details session view omits some sent/received messages, such as reasoning entries, and its terminal does not resemble the original application.
- [x] Show detail tabs appropriate to each agent.
- [x] Color-code agents in the graph so they are easy to distinguish.
- [x] Add icons/loaders to distinguish agent kinds and working, waiting, or problem states. Update orchestrator status in chat too; submitting a message currently gives no feedback that work is happening.
- [x] Avoid frames around agent cards in the graph; distinguish cards from the canvas using background color alone.
- [x] The orchestrator should react to graph-state changes and briefly report progress/completed rounds. Report progress-blocking problems to it too.

Incomplete relative to design.md:

Ledgers and quality gates (§3, §6.4-6.7, §7.6, §10.1, M2Q):

- [x] Runtime RequirementLedger and PlanItemLedger: atomic requirements, statuses (identified..verified/stale/superseded_by_user), evidence, parents/children (§6.6).
- [x] Run spec_revision; versioned requirements/steps, dependent-task/evidence invalidation, process cancellation, and rejection of old results after scope changes (§6.3, scenario 31).
- [ ] Invalidate future ContextModel/SolutionModel and ChecklistDefinition records when changed (§6.3).
- [ ] ContextModel, Unknown register, SolutionModel, SimulationCase, TraceabilityMap records with runtime-enforced context_ready, plan_ready, solution_selected, requirements_complete gates (§6.4-6.5).
- [ ] Structured OptionsAnalysis and SolutionDecision records (options, criteria, rejections, sensitivity, reopening conditions) (§6.7).
- [ ] RiskCoverage: mandatory RISK-Rxxx for every §19 catalog entry, with executor, verifier, and exclusion justification (§7.6).
- [ ] ChecklistDefinition as a versioned catalog with content_hash: shared V01-V08 and extensions for every verifier role (§7.6).
- [ ] Runtime rejection of reports with empty checklist items, missing risks, stale evidence, or unauthorized authors (scenario 30).
- [ ] InputDomainMatrix, ChangeRisk, DependencyDecision, LicenseAssessment, RefactorReport records rather than prompt-only obligations (§10.1, §12.2).
- [x] ComplianceAudit: full sources, both ledgers, and integrated result; independent audit verification and requirements_complete gate (§6.6, scenarios 27-29).
- [ ] Runtime tests rather than prompts alone for acceptance scenarios 11, 19-23, 34-40 (§18).
- [ ] Evidence invalidation graph and QualityGate (required_check_ids, input_hashes, invalidation_dependencies) (§10.1).

TUI (§8):

- [ ] Agent detail tabs: Overview (merged with Session), Artifacts, Findings, Messages, Evidence, Checks. Artifacts, Findings, Evidence are missing as separate tabs (§8.4).
- [x] Requirements and Plan items from chat/palette; sources, criteria, tasks, evidence, history; manual forms, impact preview, diff approval, draft preservation (§6.6, §8.4).
- [ ] Remaining run views: Options, Risks, Quality gates (§8.4).
- [ ] Graph relationship filters (supervises, depends_on, verifies, communicates_with) and communication-edge highlights with time/count (§8.3).
- [ ] All 12 node statuses (queued, starting, running, waiting_agent, waiting_user, verifying, changes_requested, paused, blocked, failed, completed, cancelled), with text/symbols and optional color (§8.3).
- [ ] Stage/module grouping and collapsed completed branches for large teams (§8.3).
- [ ] Comments on every agent-panel element: relationship, session, turn, message, evidence, checklist item, gate (§8.5).
- [ ] Run/stage header and stages (Intake..Delivery) in run state (§6.1, §8.1).
- [ ] Active-agent message delivery choice: native steering, next-turn queue, interrupt/resume; show available options in the UI (§8.4).
- [ ] The `antislop ui` command settled in §8.2 (currently `antislop chat` and launching without arguments).

Runtime, daemon, adapters (§7, §9, §10, §13, §14):

- [x] Ledger-operation idempotency, expected_spec_revision, and exact-diff approval in daemon API (§9.3).
- [ ] request_id and idempotency keys for remaining state changes in daemon protocol (§9.3).
- [ ] One active controlling client plus observer clients; control conflicts cannot use last-write-wins (§9.3).
- [ ] Adapter contract: probe with capabilities, send with delivery_mode/receipt, interrupt with receipts distinguishing requests from confirmed termination (§9.4).
- [ ] Codex app-server (JSON-RPC, steering) instead of exec --json (§9.5).
- [ ] Run/verify the gemini adapter against a real binary and complete the M0 version matrix (§16).
- [ ] Permission profiles read-only, workspace-write, restricted-execution, extended-access; block sessions when restrictions cannot be enforced (§13).
- [ ] Secret redaction in logs/artifacts, file permissions, retention policy (§13).
- [ ] Token usage marked reported / estimated / unavailable (currently reported or absent only) (§14).
- [ ] Native CLI subagents disabled or registered as managed nodes; agy/gemini have no switch (§9.5).
- [ ] Team templates (small-change, feature, refactoring, security-fix, greenfield, research, document, analysis) as versioned code definitions, not just orchestrator instructions (§7.3).
- [ ] Versioned built-in roles/templates persisted at run start (§9.6).
- [ ] Task-level resume and retries for waiting tasks (§14).
- [ ] delivery_unknown state and session reconciliation for uncertain message delivery (§10.4).
- [ ] Events/justification for every team-composition change (§7.3).
- [ ] Validate every necessary profile before team launch; missing CLIs produce explicit replacement proposals or blockage (§7.4).

Workspace (§11):

- [ ] Projects without Git: runtime-created initial snapshot/repository (design §11.1; writers currently request git init).
- [ ] File leases and separate resources (ports, databases, caches) for parallel worktrees (§11.1).
- [ ] Research/documents/analyses without Git: versioned input/output directories and hashed artifacts (§11.2).

Approvals and modes (§6.2):

- [ ] Supervised: request approval for every revision round (currently the first 3 rounds proceed without asking).
- [ ] No automatic revision rounds after rejection in supervised mode; approval per stage and round (§6.2).

Other:

- [ ] Run history measuring time to accepted results, user effort, round/blockage/conflict counts, and scope changes (§15).
- [ ] Architect licensing/distribution decision (§20, item 4).

- [x] Merge worker and verifier into one graph node.
- [x] Merge Overview and Session tabs.
- [x] Global configuration fallback when project antislop.toml is absent (t2).
- [x] TUI editor for every configuration option, applied live in the daemon (t3/t7).
- [x] Mouse selection/copying of session text (t8).
- [x] Refresh open Checks after round changes and check events.
- [x] T14: global trust check before opening daemon state; no trust migration from project databases; safe registry temporary files.
- [x] T14: bounded persisted turn errors and known-environment-secret redaction (not the entire transcript).
- [x] T13: local Checks invalidation; single-task reads without decoding whole plans or rereading for freshness tokens.
- [x] T13: transcript record/wrapping caches; integrated-code review.
- [x] T13: lightweight task_checks endpoint without Git fingerprinting; graph-layout caching and viewport route clipping.
- [x] T14: integrate chain b6356f1 with main base d050f2c, review new code, and full gate: check, fmt, clippy -D warnings, 473 tests (none skipped).
- [x] T14: local routing test removed following the user's change; antislop.toml unchanged.
- [x] T14: SetConfig compares revisions under a lock; 64 KiB clipboard limit; process-interruption tests before/after configuration publication.
- [x] T14: contract/schema 5→6 review; new-verifier-profile regression in the same round; raw protocol rejects missing/stale revisions.

- [x] Mail delivery to every role, bounded retries, reply-loop protection; mail does not block plan schedulers.
- [x] Safe orphaned-configuration-file cleanup protecting active writes.

- [x] t8: compared saved t4/t5/t6/t7 results; resolved details.rs conflict preserving helpers and both behaviors, without final src/tests changes. Six-requirement matrix and manual terminal checks: docs/lean-core.md, “t8 integration gate” section.
- [x] t8: repeated final gate: fmt, clippy -D warnings, check, full cargo test --no-fail-fast — exit 0; 496/496, none skipped. All 31 earlier failures now pass; log /tmp/antislop-t8-final-verification.log. Evidence for six requirements in docs/lean-core.md.
- [ ] t8: manual real-terminal checks: OSC 52 selection/copy, /config live saving, single task node and Overview, Checks across several rounds.
