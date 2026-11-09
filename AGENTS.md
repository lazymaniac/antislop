# AGENTS.md — Development Guidelines for antislop

This document specifies the rules, design constraints, and engineering practices that all AI coding agents must follow when working in this repository.

---

## 1. Toolchain & Rust Standards

- **Rust Edition & Channel**: Edition `2024`, Rust `1.98.1` (`rust-toolchain.toml`).
- **Formatting**: Always format code using standard Rust formatting:
  ```sh
  cargo.sh fmt --check
  ```
- **Linting & Warnings**: Zero tolerance for compiler or clippy errors:
  ```sh
  cargo.sh clippy --all-targets -- -D warnings
  ```
- **Review**: Run `cargo check` and targeted tests before concluding changes:
  ```sh
  cargo.sh test --test lean_core
  cargo.sh test --test lean_tui
  ```

---

## 2. Core Architectural Principles (Lean Core)

1. **Agents Work Directly on Files**:
   - Never copy entire repository trees or full file contents into SQLite database tables, event payloads, or prompt bodies.
   - Agents explore directories independently using their CLI tools.
   - Writers work in isolated Git worktrees; reviewers inspect diffs or read worktrees in read-only mode.

2. **Compact, Scalable State**:
   - Database operations in `src/lean/store.rs` must update small, normalized rows one at a time with an append-only event log.
   - Avoid whole-document hashing or state dumps that scale with the repository size.

3. **No Unactionable Blockers**:
   - When a CLI execution or background step fails, report the raw failure reason directly so the user can retry or adjust.
   - On daemon restart or interruption, mark in-flight turns as interrupted cleanly.

---

## 3. Terminal UI (TUI) & Text Editing Rules

When modifying or adding UI components in `src/lean/tui/`:

1. **Unicode & Grapheme Integrity**:
   - All text slicing, cursor movement, line wrapping, and deletions must operate on **Unicode extended grapheme cluster boundaries** (`unicode_segmentation`), never on raw bytes or character counts.
   - Support zero-width joiners (ZWJ) and combining marks without splitting glyphs or corrupting UTF-8.

2. **Draft & State Preservation**:
   - Network or daemon errors must never discard user text. Unsent drafts must be recovered or restored.
   - Retain drafts independently per conversation during conversation switching (`Ctrl+O`).
   - Modal dialogs (Help, Comments, Confirmation Cards) must cleanly isolate key events and preserve background drafts and search queries.

3. **Terminal Dimensions & Canvas Bounds**:
   - Guard against terminal dimensions smaller than 12 columns x 5 rows.
   - Keep canvas memory allocations bounded by viewport area (`O(viewport_area)`). Never allocate canvas buffers proportional to graph task count.
   - Honor `NO_COLOR=1` (disable ANSI colors, keep dim/reversed) and `ANTISLOP_ASCII=1` (render borders in plain ASCII).

---

## 4. Working Tree & Process Safety

- **Working Directory**: The process working directory is the repository root supplied by the user. Do not hardcode absolute system paths.
- **Runtime State**: Store runtime state, databases, logs, checkpoints and per-session settings outside disposable checkouts: `ANTISLOP_STATE_ROOT`, else `$XDG_STATE_HOME/antislop`, else `~/.local/state/antislop`. Project directories are keyed by the canonical common Git directory (canonical project root without Git), so worktrees share durable state. Do not put runtime files in the source tree. The runtime keeps no directory trust registry; a CLI's trust dialog for a directory the runtime allocated is answered automatically, any other dialog goes through the approval flow.
- **Git Worktree Isolation**: Task workers must only modify files inside their allocated task worktrees. Never mutate the user's primary working tree outside approved merges.
- **External CLI Processes**: Launch every CLI with no permission, sandbox or approval flags: the CLI's own policy applies, and the approval dialogs it shows are recognized on the PTY screen, put to the user (supervised, the default per conversation) or to the orchestrator (autonomous) and answered with the CLI's own keys. Do not impose task, concurrency, execution-time, idle-time, memory, cost or retry budgets. Honor manual pause/cancellation, surface raw failures and terminal tails, and report missing progress without automatically killing work. Preserve roles, dependency ordering, quality review, evidence and user changes. Pass the gateway per launch; never write into a CLI's own configuration files (Agy's one-time bridge registration, `antislop setup agy`, is run by the user).

<!-- hologram:start — generated, do not edit; refreshed by git hooks -->
Hologram project map: exact files, signatures, fields, and retained call paths. Read it before searching source. Line 2 is the legend; omissions are marked. Before adding tests/helpers, check `? tests` and `*` helpers. Address `hologram review` findings before finishing; reuse named originals and consolidate duplicate coverage.

```
# hologram ·.rs
· C/R/I{fields} E{values} · f(args):Ret > calls · -=private · *=helper/fixture · ×0=unused · ~N=lines · p{a,b}s=pas,pbs · :T=supers · Self=own type
cargo.sh
 cargo.sh(C) ×0
packaging
 changelog.sh
  changelog.sh(C) ×0
 homebrew-formula.sh
  homebrew-formula.sh(C) ×0
   sum()
src
 config
  diagnostics
   toml_error(source,error):Error > toml_{location,diagnostic}
   - unsupported_variant,expected_type,syntax_diagnostic
  model
   CommandLine(C) : Deserialize
    - deserialize
   Config(C{schema_version,providers,profiles,orchestrator,routing,policy})
   Effort(E{Low,Medium,High,Xhigh})
   OrchestratorConfig(C{profile})
   Policy(C{local_commits,required_gates,local_options})
   Profile(C{provider,binary,args,command,model,effort})
   ProviderConfig(C{binary,args,command})
   ProviderKind(E{Agy,Codex,Claude,Gemini,Opencode})
    native_effort(effort):Option<&'static str>
   RoleKind(E{Executor,Reviewer,Orchestrator,Planner})
   RoleRouting(C{executor,reviewer})
   WorkKind(E{Orchestration,Context,Requirements,Options,Planning,Architecture,Dependencies,Implementation,Refactoring,StaticAnalysis,TestDesign,Testing,Security,Performance,Integration,Research,Writing,Analysis,Delivery})
   - Raw,default_{local_options,required_gates}×0
  resolution
   resolve_command(provider,profile):Result<(PathBuf,Vec<String>)> > command_and_args,provider_command,merge_args
  validation
   validate(config):Result<()> > require,validate_{providers,profiles,routing}
   - validate_{profile,profile_name,profile_values,model,profile_binary,profile_command,routes,orchestration_route,route_profiles},
     require_{gate_routes,profile},identifier,nonempty_text
 config.rs
  ResolvedProfile(C{profile,provider,binary,args,model,effort,native_effort,work_kind,role})
  Config: load(path):Result<Self>; from_toml(source):Result<Self> > toml_error; validate():Result<()>; resolve(work_kind,role):Result<ResolvedProfile> > validate,profile_name,profile,resolve_command,ResolvedProfile,native_effort
  = SCHEMA_VERSION=4
  - Config: {selected,orchestrator,routed}_profile
 lean
  agent/errors
   clip(text,limit):String
   clip_tail(text,limit):String
   persisted_error(text):String > redact_error,flatten
   redact_error(text,secrets):String > redact_word,clip
   = OUTPUT=2000
  agent.rs
   name(provider):&'static str
   reap(pid,started_ms,provider):bool > is_original_cli,terminate_group
   Outcome(C{session,text})
   Spec(C{provider,binary,args,model,effort,cwd,runtime,add_dirs,resume})
   - elapsed_seconds
  agent_tools
   dependencies
    page_text(text,binding,args):Result<(String,Option<String>)> > require
    DependencyScope(C{tasks,generation,selected_for})
   references
    reference_error(field,requested,reason,available):Error ~50 > edit_distance
  agent_tools.rs
   - revision_request_key
  cli
   chat
    run(args):Result<()> > connect,is_terminal,stream
    - events,send_message,read_input,poll_events,process_input,{input,plan,cancel,pause,send}_request
   presentation
    describe(event):Option<String> > describe_{message,plan},review_lines,describe_{task,status}
    latest_plan(events):Option<String> > as_str
    print(event) > describe
    review_lines(who,review):Vec<String> > as_str,review_findings,coverage_lines
    - value,plan_{tasks,status},describe_mail
  cli.rs
   chat(args):Result<()> > chat.run
   doctor(args):Result<()> > Options.parse,report
   init(args):Result<()>
   resolve_config(project,explicit):Result<PathBuf> > checked_config,config_dir
   setup(args):Result<()> ~43 > status,system_settings_path,settings_text
   stop(args):Result<()> > project_state
   Options(C{project,config,state,conversation,new,message})
    parse(args):Result<Self> > project_state,resolve_config
    prepare():Result<()> > private_directory
    conversation(client):Result<String> > latest
    latest(client):Result<Option<String>> > flatten,Client.call,as_str
   - ParsedArgs,option_value
   - ParsedArgs: parse,apply
  content
   Chunk(C{text,next_offset,done})
   Content(C{directory})
    new(database):Self
    encode(text):Result<String>
    reference(encoded):Result<Reference>
    decode(text):Result<String> > read
    sql_text(row,column):rusqlite::Result<String> > decode
    write(text):Result<Reference> > private_directory,Reference,path
    chunk(reference,offset):Result<Chunk> > require,path,len,Chunk
    read(reference):Result<String> > chunk
    remove_unreferenced(db):Result<usize> ~57
   Reference(C{id,bytes})
   = CHUNK_BYTES
  core
   integration
    conflicts/repair
     Repair(C{core,conversation,task,spec,worktree,onto,change,files,outcome,cancel,base,attempts})
      new(parts):Self > repair_count
      run():Result<()> > {request,stop}_repair,finish_repair_report
      - apply_repair_report,wait_for_repair_input,record_attempt,worker_{report,setup,prompt},persist_repair,queue_review,
        review_prompt
     RepairParts(C{core,conversation,task,spec,worktree,onto,change,files,outcome,cancel})
    land
     flow
      Integration(C{core,conversation,task,spec,project,worktree,config,cancel})
       new(core,conversation,task,spec,project,worktree,config,cancel):Self
       run(lock):Result<()> > finish_no_changes,{create,apply}_change,record_plan_landing,repair_conflict,finish_outcome
       - register_conflict_repair,complete_landing
      - conflict_with_files
     landing
      record(core,conversation,task,spec,outcome,files):Result<Landing> > record_outcome,append_event
      Landing(C{status,text})
      - approved,unresolved
    writer_base
     - PlanBase
   messages
    - dispatch_{to_plan,broadcast,reviewer},direct_task,reject_recipient
   open
    - write_state_gitignore,open_lock_file,acquire_lock,reap_orphaned_agents,spawn_native_reaper,
      restore_{conversations,conversation},wake_pending_work,resume_{cut_off,interrupted},answer_pending_mail
   orchestration
    agent_turn
     turn
      execute
       execute(core,state,cancel,prompt,events):Result<agent::Outcome> > execute_profile
       - PreparedAgent,clone_error,prepare_agent,agent_spec,session_scope,{record_run,run}_context,check_task_inputs,
         tool_handler,protocol_observer,record_rejected_tool,run_cli,{should,announce_startup}_retry,restart_native
      finish
       finish(core,outcome,events,cancel):Result<Option<String>> > finish_{success,failure}
       - record_failed_mail
      prompt
       prepare_prompt(core,state,spec,prompt):Result<String> > inbox_text,{append_task,append_allocation}_context,role_introduction,prompt,append_round_handoff
       - append_{questions,local_checks}
      setup
       begin(core,setup):Result<TurnState<'a>> > TurnStart.new,pending_{inbox,comments},start_turn,attribute_inbox
       wait_until_resumed(core,setup,cancel):Result<()> > as_str,paused
       - TurnStart,SetupParts,profile_provider,resume_session
       - SetupParts: from_setup
       - TurnStart: finish
     turn.rs
      TurnEvents(C{store,conversation,turn,inbox,persistence_error,prompt_bytes})
       announce(kind,payload)
      TurnState(C{session_policy,conversation,role,profile,cwd,add_dirs,provider,resume,inbox,history_mail,comments,turn})
    approvals
     approver(core,conversation,role,turn,spec):Arc<approvals::Approver> > ApprovalSeat,name,settle_approval
     ApprovalSeat(C{conversation,role,turn,provider,directories})
     - {parse_trust,same_directory}_target,decision_name
    comment_helpers
     comment_label(target,plan):String > {plan,check,finding,artifact,addressed_target}_label
     is_plan_target(target):bool
     - {task,diff,diff_location,named_target}_label
    config_snapshot
     - {live,plan}_status
    orchestrator_turn
     context
      answer
       finish(core,context,answer,cancel):Result<()> > TurnResult.decode,submitted_graph,audit_response_error,add_message,submitted_plan,record_answer_progress,finish_interview,handle_plan_answer
      comments
       comment_context(core,conversation,upto,answered,is_comment):Result<String> > append_comment_{instructions,plans,followup}
       - append_{last_orchestrator_reply,latest_plan,original_request,referenced_plans}
      plan_view
       load(core,conversation):Result<PlanView> > workflow,announce_planning,editable_plan,record_revision_view,graph_json,PlanView
       PlanView(C{workflow,graph,graph_json})
     context.rs
      ask(core,context,cancel):Result<Option<String>> > turn_setup,Core.turn,render_prompt
      finish(core,context,answer,cancel):Result<()>
      prepare(core,conversation,upto):Result<OrchestrationContext<'a>> > request_context,config_snapshot,plan_view.load,history_context,audit_response,OrchestrationContext,interviewing,open_user_decision
      AuditResponse(C{approved,findings,error,report})
      OrchestrationContext(C{conversation,upto,answered,asked,is_comment,config,project,open,comment_text,workflow,interviewing,graph,editable_graph,graph_json,history,ledger_context,decision,audit_response})
      - RequestContext,open_comments
     prompt
      orchestrator_prompt_text(context):String > orchestrator_prompt,append_{history,decision,graph},prepend_phase,append_{audit_response,ledger}
      PromptContext(C{project,first,fresh,answered,asked,open,history,comment_context,editable_graph,graph,graph_json,interviewing,workflow,ledger_context,decision,audit_response})
    planner_turn/prompt
     planner_prompt_text(context):String > format_messages,append_ledger
     PlannerPrompt(C{project,first,requirements,asked,replaced,keepable,ledger_context})
    planner_turn.rs
     - PlannerTurnContext,authored_contracts,planner_{setup,prompt},announce_planner_draft,record_planner_result
    planning_context
     append_ledger(context,prompt)
     planning_ledger_context(core,conversation):Result<Option<String>> > load_planning_context,planning_run,ledger_context
    prompt_helpers
     blocked_report(tasks):String > blocked_entries
     format_messages(messages):String
     orchestrator_prompt(project,first,messages,open):String > initial_orchestrator_prompt,format_messages,append_open_comments
     stopped_report(tasks):Option<String> > blocked_entries
     - stopped_for_coordination,format_message
    unanswered
     includes_comment(asked):bool
   plan
    answer
     PlanAnswer(C{conversation,project,config,text,plan,addressed,is_comment,answered,upto})
    helpers
     finalize_plan(specs,required_gates,local_options):Vec<TaskContract> > finalize_team_plan
     finalize_team_plan(specs,required_gates,local_options,team):Vec<TaskContract>
     planning_gaps(specs,team):Option<String>
     writer_base_scratch(state,{plan,spec}_id):PathBuf
    kept_tasks
     - authored,not_keepable
    proposal
     helpers/recording
      append_planning_gaps(prompt,gaps):String
      plan_event_payload(text,plan,specs,review,waiting,revision_token):Result<serde_json::Value>
      plan_review_prompt(project,request,specs,first):String
      record_plan_review_start(store,conversation,plan,config,specs):Result<()> > Store.append_event
     helpers.rs
      PreparedPlan(C{id,tasks,answered})
    proposal.rs
     checked_plan(specs,config):std::result::Result<Vec<TaskContract>,String> > checked_team_plan
     checked_team_plan(specs,config,team):std::result::Result<Vec<TaskContract>,String> > finalize_team_plan,check,message
    report
     - task_{result,findings},apply_audit_gates,invalidate_unearned_approval,unapproved_task_ids,audit_text
    revision
     acceptance
      ensure_accepted_plan(core,conversation,plan):Result<()> > db,unaccepted_plan_error
     review
      affected_tasks(baseline,specs,redo):BTreeSet<String> > changed_tasks,as_str
      ReviewContext(C{conversation,revision,baseline,specs,cancel,project})
      - ApprovedRevisionWork,RevisionBarrier,contract_changed,reworded_only,checkpoint_review_context,previous_specs
      - RevisionBarrier: drop
    revision.rs
     - PreparedRevision,RevisionRegistration,run_revision_worker
     - RevisionRegistration: fresh,existing
    start
     runner/reporting
      has_blocked_tasks(tasks):bool
     runner.rs
      - AuditedPlanSnapshot,format_uncommitted_notice
   review
    agent
     - run_review_queue,next_review_role,review_{mail,role,audit,plan},task_contracts,plan_review_prompt
    change
     - CodeReview,review_comments,execute_code_review,review_{turn_setup,prompt,failure},publish_{code_review,review_message}
   state
    Activity(C{running,plans,reviews,agent_reviews,task_cancels,{task,orchestrator}_restarts,blocked_seen})
     start_orchestrator(conversation):Option<Arc<AtomicBool>>
     cancel_tasks(tasks)
     plan_running(conversation):bool > has
     review_running(conversation):bool > has
     agent_review_running(key):bool > has
     task_running(task):bool > has
    ConfigState(C{current,update,modified})
     new(config):Self
     updating():MutexGuard<'_,()>
     modified():u64
     set_modified(modified) > store
    Locks(C{integrating,graph_updates,turns,session_lifecycle})
     integration():MutexGuard<'_,()>
     turn(conversation,role):Arc<Mutex<()>>
     graph():MutexGuard<'_,()>
    RevisionState(C{revising,jobs,views})
     view(conversation):Option<(String,i64)> > flatten
    WorktreeState(C{cleanup,cleanup_notes,leases,lease_changed})
   task
    checks/scheduler
     run(core,conversation,plan_id,cancel) > Scheduler.new
     - Scheduler,Reservations,task_states,dependencies_approved,{execute,fail}_task,restart_agent
     - Reservations: contains,reserve,release,is_empty
     - Scheduler: run,schedule_{ready,one},is_schedulable,restore_skipped_task,is_runnable,schedule_if_available,spawn_task,
                  receive_completion,release
    execution
     reviewer_setup
      build(conversation,task,role,profile,cwd,add_dirs):TurnSetup<'a> > TurnSetup
     task_execution
      review
       record_checks(core,conversation,task,spec,verdict):Result<()> > record_one
       run(input):Result<Option<Verdict>> > request_review,stop_task,parse_verdict
       save_verdict(input):Result<()> > stamp_verdict,set_feedback,update_task
       ReviewRound(C{core,conversation,task,cancel,spec,workdir,add_dirs,upstream,reviewer,round,findings,report,changes})
       VerdictSave(C{core,task,spec,verdict,report,changes,summary_effect,repairs})
       - ReviewTurn,run_review_turn
      rounds
       - FinishedWorker
      worker/completion
       commit_round(task,spec,worktree,_round,report,summary,commit_message):Result<Option<String>> > validate_change_paths
       complete_report(core,conversation,task,spec,worktree,raw):Result<Option<WorkerCompletion>> > update_task,finish_report
       WorkerCompletion(C{report,effect,commit_message})
       - save_checkpoint,commit_worker_files
      worker.rs
       is_recheck(core,conversation,task,worker,reviewer):Result<bool> > completed_task_output,pending_mail
       run(input):Result<Option<String>> > completed_task_output,run_worker_round
       task_directories(core,task,spec,project,worktree):Result<(PathBuf,Vec<PathBuf>)> > task_worktree
       writer_worktree(core,task,spec,project):Result<Option<PathBuf>> ~60 > has_delivery,require,prepare_artifact_outputs,writer_base,combine_saved_checkpoint,worktree_path,ok
       WorkerRound(C{core,conversation,task,cancel,spec,workdir,project_text,add_dirs,upstream,worker,round,state,recheck})
       - run_worker_turn,worker_{setup,dirs},continuation
     task_execution.rs
      Execution(C{core,conversation,task,cancel,spec,config,project,worktree,_worktree_lease,workdir,project_text,add_dirs,upstream,worker,reviewer,integration_repairs})
       run_round(round,state):Result<bool> > prepare_round,worker_or_stop,finish_worker_report,{review,settle}_round
       new(core,conversation,task,cancel,worktree_lease):Result<Self> > Prepared.load
       run():Result<()> > resume_approved_writer,integrate_approved,round_state,run_round,exhausted
       - worker_report,save_round_verdict,sweep_runtime,incomplete,approve_round,update_progress,load_spec,
         dependents_invalidated,resumed_progress
      - RoundState,Prepared
    feedback_helpers
     event_files(files):Vec<String>
     land(task,outcome,source,detail,commit,on_branch,files) > current_feedback,integration_record
     note(task,text)
     review_json(review):serde_json::Value
     - add_{commit,branch,files}
    recovery
     actions
      apply(core,work):Result<()> > keep_branch,retry,restart,cancel
      - queue_guidance,persist_simple,recovery_payload,{retry,restart}_event
     events
      append(core,work,payload):Result<()>
     integrate
      apply(core,work):Result<()> > require,integration_commits,validate_consent,{integrate,mark_remaining}_changes,finish_integration
      - validate_consent_file,integration_result,persist_integration,integration_event
     manual/finish
      approve(core,work,review,fixed):Result<()> > commit_manual_fixes,approval_notice,land,note,persist_manual
      reject(core,work,review,fixed):Result<()> > append_findings,land,persist_manual
      - manual_event
     manual.rs
      apply(core,work):Result<()> > prepare_review,update_review_state,feedback,approve,reject
      - ManualReview,Group,run_reviewer,reviewer_setup,snapshot_conflict,
        restore_snapshot_{conflicts,conflicts_counted,conflicts_with_budget},snapshot_contains_path,apply_snapshot_paths,
        materialize_snapshot,snapshot_lookup_{fixed_arg_bytes,batches},reviewer_prompt,decode_answer,failed_reviewer,
        awaiting_answer
     prepare
      interrupt_if_active(core,work):Option<TaskRecoveryResponse> > is_running_action,queue_restart,interrupted
      load(core,req,actor):Result<RecoveryWork> > accepted_plan,config_snapshot,Core.task,assert_task_inputs,task_freshness_token,validate_request,RecoveryWork
      - allowed_status
     response
      build(core,work):Result<TaskRecoveryResponse> > task_freshness_token,TaskRecoveryResponse
      interrupted(work):TaskRecoveryResponse > TaskRecoveryResponse
    recovery.rs
     RecoveryWork(C{req,actor,project,config,spec,task,token,integrated_files,preserved_branch,notice})
    runtime
     citations(core,task,report,verdict):Result<String> > flatten,task_evidence_context,evidence_text
     directory(state,task):Result<PathBuf> > task_path_component,private_directory
     prune_settled_generations(state,task):Result<()> > task_path_component
     sources(core,task):Result<Vec<serde_json::Value>> ~51 > task_in_plan_by_id,plan_conversation,task_path_component,private_directory
     sweep_uncited(state,task,citations,cutoff):Result<Sweep> > task_path_component,sweep_directory
     Sweep(C{removed,bytes,errors})
     - Citation,is_path_char,cited,scan,newest_modification
    state
     - stop_records_progress
    unblock_dependents
     - skipped_task_ready,dependency_approved
   worktree
    cleanup
     WorktreeLease,WorktreePin(C{core,path}) : Drop
      WorktreeLease - drop
      WorktreePin - drop
     WorktreeLeaseState(C{leased,pins})
     - CheckoutKind,checkout_kind,eligible
    paths
     branch(task):Result<String>
     release_settled_views(store,state):Result<()> > worktree_directory,{release_settled_view,release_writer}_checkouts,prune_settled_view_branches
     release_view(path):Result<()> > require
     release_views(state,task_id,keep):Result<()> > task_view_paths,release_view
     task_path_component(id):Result<&str> > all
     task_view_paths(state,task_id,keep):Result<Vec<PathBuf>> > task_path_component,worktree_directory
     worktree_branch(state,task):Result<String> > worktree_path,branch,require
     worktree_directory(state):Result<PathBuf> > require
     worktree_path(state,task):Result<PathBuf> > task_path_component
     - view_task,settled,release_writer_checkout,writer_checkout_{id,is_clean}
  daemon
   client/launch
    daemon_exit_error(status,state):Error
    detach_daemon(command)
    spawn_daemon(project,config,state,executable):Result<std::process::Child> > detach_daemon
    wait_for_daemon(client,child,state):Result<()> > Client.call,daemon_exit_error,require
   client.rs
    Client(C{socket,state,notice,origin,identity})
     at(state):Self > socket
     named():Self
     state_dir():&Path
     frontend_binary():Result<PathBuf>
     stop(state,force):Result<bool> > stop_client,send_stop
     connect(project,config,state,executable):Result<Self> > prepare_client,reuse_existing,clear_stopped,{spawn,wait_for}_daemon
     call(request):Result<Value> > call_once,lost
    - daemon_missing,same_{origin,daemon_context},reuse_busy_daemon,busy_notice,wait_unlocked
   config_write
    atomic_config_write(path,text):Result<()> > PreparedConfigWrite.prepare
    cleanup_config_temporaries(path):Result<usize> > config_directory,ConfigDirectoryLease.acquire,config_temporary_prefix,cleanup_temporary,sync_directory_if_changed
    config_temporary_prefix(target):String
    ConfigDirectoryLease(C) : Drop
     - drop
    ConfigUpdateReply(C{toml,path,status,warnings})
    ConfigUpdateStage(E{EventCommit,DirectorySync})
    ConfigUpdateStatus(E{Applied,AppliedWithWarnings})
    ConfigUpdateWarning(C{stage,message})
    PreparedConfigWrite(C{target,temporary,directory,lease}) : Drop
     prepare(path,text):Result<Self> > config_directory,temporary_path,ConfigDirectoryLease.acquire
     publish():Result<()>
     sync_directory():Result<()>
     - create_locked,lock_lease,write_text,drop
    - temporary_name_matches,{owned_unlocked,remove_if_same}_file
   dispatch
    config
     route(context,request):Result<Route> > route_pipeline
     - set_{config_route,config},validate_config_revision,publish_config,{record_publish,record_sync}_warning,config_reply,
       get_{config_route,config}
    control
     route(context,request):Result<Route> > route_pipeline
     - identity,shutdown,force_shutdown,conversations_and_start,runtime_notice
    conversation
     route(context,request):Result<Route> > route_pipeline
     - {comment_status,comments}_route,comments,comment_status_filter,load_comments,decorate_comment_scopes,
       comment_{route,at},validate_round_comment
    history
     route(context,request):Result<Route> > route_pipeline
     - task_{rounds,round},run_context,session_events,history_{content,messages},session_content,events,
       paged_events_and_sessions,history_and_mail,{handle,wait_for}_events,events_result
    ledger
     route(context,request):Result<Route> > route_pipeline
     - list_runs,reject_proposal,get_{proposal_and_requirements,entry,diff,evidence_and_propose},apply_change_and_create_run,
       {add,get}_source
    mod
     handle(core,project,config,request,frontend):Result<Value> > ensure_accepting,scope,handle_request
     Context(C{core,project,config})
     Route(E{Handled,Unmatched})
     - route_{request,all},validate_{project_conversation,conversation_plan,plan_task}
    planning
     actions
      route(context,request):Result<Route> > route_pipeline
      - send,addressed_send,reject_plan,review_recovery_and_resume,resume_plan,restart_and_cancel,cancel_work,pause,approvals
     mod
      route(context,request):Result<Route> > route_pipeline
     tasks
      route(context,request):Result<Route> > route_pipeline
      - plans_and_plan,task,task_{checks,diff},review_task
    terminal
     route(context,request):Result<Route> > route_pipeline
     - terminal_{open,read,input,resize,detach}
   lease
    scope(request):Option<(&str,Access)>
    Access(E{Follow,Change,Take})
    Leases(C{held})
     access(conversation,client,access):Result<bool> > require
    = OBSERVER
   lifecycle
    serve(project,config_path,state):Result<()> > start,spawn_idle_cleanup,accept_connections
    - Runtime,ConnectionResponse,warn_on_config_cleanup,open_core,bind_socket,serve_connection,read_request,handle,
      error_response
   protocol
    convert
     decode_control/all
      decode_group_all(wire):std::result::Result<Request,Box<WireRequest>> > decode_{runtime_notice,identity,get_config,set_config,shutdown,force_shutdown,conversations,start}
     decode_control.rs
      decode_control(wire):std::result::Result<Request,Box<WireRequest>>
     decode_conversation
      comments
       decode_group_comments(wire):std::result::Result<Request,Box<WireRequest>> > decode_{comment,comments,comment_status}
      messages
       decode_group_messages(wire):std::result::Result<Request,Box<WireRequest>> > decode_{send_to_agent,send,reject,resume,review_comments,recover,restart_orchestrator,take_control,cancel,pause,approvals}
      planning
       decode_group_planning(wire):std::result::Result<Request,Box<WireRequest>> > decode_{plans,plan,diff,review,task_checks,task}
     decode_conversation.rs
      decode_conversation(wire):std::result::Result<Request,Box<WireRequest>> > decode_group_{messages,comments,planning}
     decode_history
      content
       decode_group_content(wire):std::result::Result<Request,Box<WireRequest>> > decode_history_{content,messages}
      events
       decode_group_events(wire):std::result::Result<Request,Box<WireRequest>> > decode_{events,history,mail}
      task
       decode_group_task(wire):std::result::Result<Request,Box<WireRequest>> > decode_task_{rounds,round}
     decode_history.rs
      decode_history(wire):std::result::Result<Request,Box<WireRequest>> > decode_group_{task,content,events}
     decode_ledger
      reads
       decode_group_reads(wire):std::result::Result<Request,Box<WireRequest>> > decode_{list_runs,get_ledger_proposal,list_requirements,get_ledger_entry,get_ledger_diff,get_evidence,get_ledger_source}
      writes
       decode_group_writes(wire):std::result::Result<Request,Box<WireRequest>> > decode_{reject_ledger_proposal,propose_ledger_change,apply_ledger_change,create_run,add_ledger_source}
     decode_ledger.rs
      decode_ledger(wire):std::result::Result<Request,Box<WireRequest>> > decode_group_{reads,writes}
     decode_session/all
      decode_group_all(wire):std::result::Result<Request,Box<WireRequest>> > decode_{run_context,session_events,session_content,sessions,delete_session}
     decode_session.rs
      decode_session(wire):std::result::Result<Request,Box<WireRequest>>
     decode_terminal/all
      decode_group_all(wire):std::result::Result<Request,Box<WireRequest>> > decode_terminal_{open,read,input,resize,detach}
     decode_terminal.rs
      decode_terminal(wire):std::result::Result<Request,Box<WireRequest>>
     encode_control/all
      encode_group_all(request):std::result::Result<WireRequest,Box<Request>> > encode_{runtime_notice,identity,get_config,set_config,shutdown,force_shutdown,conversations,start,start_in}
     encode_control.rs
      encode_control(request):WireRequest
     encode_conversation
      comments
       encode_group_comments(request):std::result::Result<WireRequest,Box<Request>> > encode_{comment,comments,comment_status}
      messages
       encode_group_messages(request):std::result::Result<WireRequest,Box<Request>> > encode_{send_to_agent,send,reject,resume,review_comments,recover,restart_orchestrator,take_control,cancel,pause,approvals}
      planning
       encode_group_planning(request):std::result::Result<WireRequest,Box<Request>> > encode_{plans,plan,diff,review,task_checks,task}
     encode_conversation.rs
      encode_conversation(request):WireRequest > encode_group_{messages,comments,planning}
     encode_history
      content
       encode_group_content(request):std::result::Result<WireRequest,Box<Request>> > encode_history_{content,messages}
      events
       encode_group_events(request):std::result::Result<WireRequest,Box<Request>> > encode_{events,events_paged,history,mail}
      task
       encode_group_task(request):std::result::Result<WireRequest,Box<Request>> > encode_task_{rounds,round}
     encode_history.rs
      encode_history(request):WireRequest > encode_group_{task,content,events}
     encode_ledger
      reads
       encode_group_reads(request):std::result::Result<WireRequest,Box<Request>> > encode_{list_runs,get_ledger_proposal,list_requirements,get_ledger_entry,get_ledger_diff,get_evidence,get_ledger_source}
      writes
       encode_group_writes(request):std::result::Result<WireRequest,Box<Request>> > encode_{reject_ledger_proposal,propose_ledger_change,apply_ledger_change,create_run,add_ledger_source}
     encode_ledger.rs
      encode_ledger(request):WireRequest > encode_group_{reads,writes}
     encode_session/all
      encode_group_all(request):std::result::Result<WireRequest,Box<Request>> > encode_{run_context,session_events,session_content,sessions,delete_session}
     encode_session.rs
      encode_session(request):WireRequest
     encode_terminal/all
      encode_group_all(request):std::result::Result<WireRequest,Box<Request>> > encode_terminal_{open,read,input,resize,detach}
     encode_terminal.rs
      encode_terminal(request):WireRequest
     mod
      decode(wire):Request > decode_terminal,decode_session,decode_history.decode_history,decode_ledger,decode_conversation,decode_control
      encode(request):WireRequest > encode_terminal,encode_session,encode_history.encode_history,encode_ledger,encode_conversation,encode_control
    envelope
     RequestEnvelope(C{protocol,request,client})
    request
     control
      ControlRequest(E{Identity,RuntimeNotice,{Get,Set}Config,Shutdown,ForceShutdown,Conversations,Start,StartIn})
     conversation
      ConversationRequest(E{Recover,Comment,SendToAgent,Send,Reject,Resume,TakeControl,RestartOrchestrator,Cancel,Pause,SetApprovalMode,AnswerApproval,ReviewComments,Plans,Plan,Comments,CommentStatus,Diff,Review,TaskChecks,Task})
       operation():Option<(&str,&str,&'static str)> ~40
     history
      HistoryRequest(E{TaskRounds,TaskRound,HistoryContent,HistoryMessages,Events,EventsPaged,History,Mail})
     ledger
      LedgerRequest(E{ListRuns,{Reject,Get}LedgerProposal,ListRequirements,GetLedgerEntry,GetLedgerDiff,GetEvidence,{Propose,Apply}LedgerChange,CreateRun,{Add,Get}LedgerSource})
     mod
      Request(E{Terminal,Session,History,Ledger,Conversation,Control})
       - serialize,deserialize
     session
      SessionRequest(E{RunContext,SessionEvents,SessionContent,Sessions,DeleteSession})
     terminal
      TerminalRequest(E{TerminalOpen,TerminalRead,TerminalInput,TerminalResize,TerminalDetach})
    wire
     WireRequest(E{RuntimeNotice,TerminalOpen,TerminalRead,TerminalInput,TerminalResize,TerminalDetach,RunContext,Comment,SendToAgent,TaskRounds,TaskRound,SessionEvents,HistoryContent,HistoryMessages,SessionContent,ListRuns,{Reject,Get}LedgerProposal,ListRequirements,GetLedgerEntry,GetLedgerDiff,GetEvidence,{Propose,Apply}LedgerChange,CreateRun,{Add,Get}LedgerSource,Identity,{Get,Set}Config,Shutdown,ForceShutdown,Conversations,Start,Send,Reject,Resume,ReviewComments,Recover,TakeControl,RestartOrchestrator,Cancel,Pause,SetApprovalMode,AnswerApproval,Plans,Plan,Comments,CommentStatus,Diff,Review,TaskChecks,Task,Events,Sessions,DeleteSession,History,Mail})
  daemon.rs
   build_identity(executable):Result<String>
   private_directory(path):Result<()> > require
   reply_event_count(events,reply_bytes):usize > len
   serve(project,config_path,state):Result<()>
   socket(state):PathBuf > len
   - Response,handle,modified,build
  diff/parser
   parse(patch):Vec<FileDiff>
   target(key):Option<Target<'_>> > Target
   Target(C{task,file,line})
   - Parser,line,path,hunk_start,update_metadata,consume_hunk,append_hunk_line
   - Parser: consume,start_file
  diff.rs
   parse(patch):Vec<FileDiff>
   DiffLine(C{kind,old,new,text})
   FileDiff(C{path,old_path,status,added,removed,binary,lines})
   ReviewerProfile(C{provider,model,effort})
  doctor
   find_executable(binary):Option<PathBuf>
   init(project,force,out):Result<PathBuf> > find_executable,initial_config
   launch_problems(config,kinds):Vec<String> > err,profile_command
   report(project,config_path,state,out):Result<bool> ~43 > name,profile_command
   - version
  feasibility
   check(specs):Vec<Finding> > descendants,missing_delivery,forbids_reading,finding,waits_for_descendant
   forbids_reading(spec):Option<String>
   message(findings):String
   Finding(C{task,rule,severity,detail})
   Severity(E{Block,Advise})
  git
   artifact_update
    index
     Lease(C{file,path}) : Drop
      acquire(project):Result<Self>
      - drop
    parent
     Parent(C{directory})
      open(path):Result<Self> > read
      at(root,path,create):Result<(Self,String)> ~49 > cstring
      recovery_at(root,path,create):Result<(Self,String)> ~61 > cstring
      read(name):Result<File> > cstring
      matches(name,expected):Result<bool> > read,len
      rename(from,to,name):Result<()> > cstring,syscall
      restore(from,to,name):Result<()> > cstring,syscall
      unlink(name):Result<()> > cstring,syscall
      link(from,to,name):Result<()> > cstring,syscall
   artifact_update.rs
    artifact_bytes(project,commit,path):Result<Vec<u8>> > git
    deliver_owned_artifacts(project,base,change,original,expected,backups):Result<Integration> > Lease.acquire,require_{added_files,owned_outputs},publish
    require_owned_outputs(project,outputs,expected):Result<()> > require_untracked,Parent.at,artifact_bytes,require_new_outputs
    - recover
   artifacts
    artifact_files(directory,outputs,require_all):Result<Vec<String>>
    commit_artifacts(worktree,outputs,purpose,require_all):Result<String> > commit_{artifact_files,subject}
    commit_artifacts_with_message(worktree,outputs,message,require_all):Result<String> > commit_artifact_files
   changes
    changed_files(dir,base,result):Result<Vec<String>> > changed_paths
    diff_stat(dir,base,result):Result<String>
    matching_paths(dir,left,right,paths):Result<Vec<String>> > changed_paths
    patch(dir,base,rev):Result<String> > make_patch
    source_patch(dir,base,rev):Result<String> > make_patch
    squash(worktree,base,purpose):Result<String> > {task,commit}_subject
    - parse_changed_paths
   integration/files
    blobs(dir,rev,files):Result<HashMap<String,String>>
    unchanged_since(project,rev,files):Result<bool> > blobs,all
    write_files(project,rev,files):Result<()> > blobs
   integration.rs
    integrate(project,base,change,commit):Result<Integration> > integrate_scoped
    integrate_scoped(project,base,change,allowed_files,commit):Result<Integration> > changed_files,target_paths,previously_landed,unchanged_since,write_files,integrate_current
    transplant_repair(worktree,base,onto,purpose):Result<String> > commit_all,task_subject,{changed,write}_files,commit_all_with_message
    - MergePlan,{merge,run_merge}_tree,{merge_tree,conflict}_result,{can_cherry,cherry}_pick,unstage_paths,
      write_merge_result,integration_message
   recovery
    recovery_fingerprint(project,scope):Result<String> > is_repository,scope_observations,append_repository_state
    - observe_non_repository_path,append_{head_and_diffs,untracked},raw_git
   snapshots
    base_with_artifacts(dir,base,artifacts):Result<String> ~55 > {absolute,snapshot,with}_index
    commit_paths(dir,files,purpose):Result<String> > commit_{paths_with_message,subject}
    commit_paths_with_message(dir,files,message):Result<String> > commit_staged_paths
    integrated_fingerprint(project):Result<String> > snapshot
    snapshot(dir,purpose):Result<String> > snapshot_with_artifacts
    snapshot_with_artifacts(dir,outputs,purpose):Result<String> > artifact_files,commit_subject,{absolute,snapshot}_index,write_snapshot
    stage_source(dir,index):Result<()> ~78 > command
    uncommitted(dir):Result<Vec<String>> > git
    - staged_paths_clean,commit_only_paths
   untracked
    deliver_untracked(project,base,change):Result<Integration> ~55 > changed_files,all,git,ensure_parents
    require_added_files(project,base,change,expected):Result<()> > changed_files
    require_new_outputs(project,outputs):Result<()>
   worktree
    add_worktree(project,path,branch,base):Result<()> > require_worktree_branch,git
    branches(project,prefix):Result<Vec<String>>
    combine(project,scratch,onto,changes):Result<String> > combine_or_conflict
    combine_or_conflict(project,scratch,onto,changes):Result<CombineResult> > {remove,add}_worktree,apply_changes
    commit_all(worktree,purpose):Result<String> > stage_source,commit_{staged,subject}
    commit_all_with_message(worktree,message):Result<String> > stage_source,commit_staged
    commit_staged(worktree,message):Result<String> > git,head
    common_base(dir,left,right):Result<String>
    delete_branch(project,branch):Result<()>
    head(dir):Result<String>
    is_clean(path):bool
    is_repository(dir):bool > git
    prune_worktrees(project):Result<()>
    remove_clean_view(project,path,branch):Result<()> > require_worktree_branch,is_clean,view_snapshot_ref,head,delete_branch
    remove_clean_writer(project,path,branch,outputs):Result<()> > require_worktree_branch,all
    remove_worktree(project,path,branch):Result<()> > require_worktree_branch,git
    remove_worktree_keep_branch(project,path):Result<()> > git
    rename_branch(project,from,to):Result<()>
    restore_clean_view(project,path):Result<()> > view_snapshot_ref,add_worktree
    session_branch(dir):Result<String> > head
    tree_identity(dir,commit):Result<String>
    worktree_branch(path):Result<String>
    worktree_project(path):Result<PathBuf>
    CombineResult(E{Applied,Conflicted})
    - apply_{change,patch}
  git.rs
   commit_subject(purpose):&'static str > subject
   git(dir,args):Result<std::process::Output>
   ok(dir,args):Result<String> > git
   own_subject(dir,rev):Result<Option<String>> > ok,is_task_subject
   task_subject(dir,base,rev):Result<Option<String>> > ok,is_task_subject
   tree_entry(dir,tree,name):Result<Option<TreeEntry>> ~44 > git,TreeEntry,len
   CommitPurpose(E{TaskResult,Checkpoint,ArtifactCheckpoint,Integration,Repair,ManualRecovery,{Workspace,Conflict,Dependency,ArtifactInput,SourceInput,Requirements}Snapshot,TestFixture})
    subject():&'static str
   Integration(E{Committed,Applied,Conflict,UserEdits})
   TreeEntry(C{object,is_tree,stdout_bytes})
   - is_runtime_subject
  history
   mail/agent
    agent_message_signature(request):Result<String>
    existing_agent_message(tx,request,signature):Result<Option<Value>> > existing_agent_result
    insert_agent_mail(tx,request,signature,task,stored_text,now):Result<i64> > insert_{mail_row,mail_scope_and_operation,orchestrator_message,mail_events}
    update_agent_task(tx,request,task,now):Result<()> > queue_agent_task
    validate_agent_recipient(tx,request):Result<Option<String>> > {validate_task,validate_role}_recipient
    - validate_task_scope,insert_user_message_event
   mail.rs
    - invalidate_{dependent,task_input},record_invalidation
   messages
    - message_row
   rounds
    - round_exists,load_{round_turns,native_sessions}
  history.rs
   AgentMessage(C{conversation,plan,recipient,operation_id,expected_{round,run},text,delivery})
   ContentOwner(E{Round,Mail})
   Delivery(E{Queue,Steer,Interrupt})
    name():&'static str
   HistoryScope(E{Round,Session})
  interview
   Choice(C{label,description}) : From
    - from
   Question(C{id,context,question,options})
    validate():Result<()> > require,len,all
    transcript():String
   - RawChoice
  launch
   common
    runtime_args(args,provider):Vec<String>
    toml_string(text):String
   providers
    agy
     agy(command,spec,gateway):Result<()> > agy_ready,url,token,agy_overrides
     agy_ready():Result<()> > agy_bridge_registered
    claude
     claude(command,spec,gateway):Option<String> > clear_parent_session,append_{gateway_config,provider_options,session_id}
    codex
     codex(command,spec,gateway) > append_codex_settings,token,append_codex_{paths,model}
    gemini
     gemini(command,spec,gateway):Result<()> > system_settings_path,settings_text,url,token,append_gemini_options
     settings_text():String
     system_settings_path():PathBuf
     = URL_ENV="ANTISLOP_MCP_URL"
    mod
     command(spec,gateway):Result<Launch> > base_command,configure
     - configure_{claude,codex,opencode,agy,gemini}
    opencode
     opencode(command,spec,gateway):Result<()> > read_config,install_{agent,gateway},append_resume_model
  launch.rs
   Launch(C{command,session,settings})
  ledger
   mod
    diff_hash(diff):Result<String> > diff_hash_with_after
    matches_proposal_hash(diff,request,expected):Result<bool> > diff_hash
    = MISSING_LEDGER
   model
    Assessment(C{evidence,approved,note})
    ChangeResult(C{run_id,spec_revision,affected_tasks})
    Criterion(C{id,text,mandatory,integration})
    Disposition(E{Active,SupersededByUser})
    EntrySpec(C{id,source,interpretation,qualifiers,parent,depends_on,mandatory,criteria,position,disposition})
    Evidence(C{id,run_id,proposal,reviewer_turn,assessment,approved})
    EvidenceProposal(C{entry_{id,version},{criterion,task}_id,generation,attempt,producer_turn,result,artifact,integrated_fingerprint})
    Extraction(C{run_id,expected_spec_revision,entries})
    LedgerChange(C{conversation,run_id,expected_spec_revision,operation_id,reason,entries})
    LedgerDiff(C{run_id,{from,to}_revision,before,after,affected_tasks})
    LedgerEntry(C{run_id,version,spec_revision,original,spec,status})
    LedgerPage(C{run,entries,total,offset,has_more})
    LedgerQuery(C{conversation,run_id,revision,status,task,missing_coverage,offset,limit})
    Run(C{id,conversation_id,goal,spec_revision,state,start_message})
    Source(C{id,run_id,message,locator,hash})
    SourceSpan(C{source_id,start,end})
     text(source):Result<&'a str> > require
     lines(source):Result<(usize,usize)> > text
    - yes×0
   runtime
    compliance/audit_conversion
     audit_proposal(findings,submitted,entries,scope,producer):Result<Audit> > bind_audit_evidence,Audit,audit_approves
     bind_assessment(assessment,evidence):Result<Assessment> > evidence_id,Assessment
     bind_assessments(submitted,evidence):Result<Vec<Assessment>> > bind_assessment
     bind_audit_evidence(item,entries,tasks,fingerprint,producer):Result<EvidenceProposal> > resolve_evidence
     decode_audit(answer):Result<(Vec<serde_json::Value>,Vec<AuditEvidence>)> > TurnResult.decode
     decode_audit_review(answer):Result<(String,Vec<AuditAssessment>)> > TurnResult.decode
    compliance.rs
     - Review,AuditScope,audit_{prompt,tasks},review_prompt
    requirements
     extraction
      ensure_not_cancelled(cancel):Result<()>
      extracted_change(conversation,run,entries):LedgerChange > LedgerChange
      extraction_prompt(review_feedback,sources):String
     turn
      RequirementsTurnScope(C{conversation,run_id,phase,plan_id})
    requirements.rs
     - requires_user_approval
    submissions
     audit
      audit_approves(findings):bool > as_str
      audited_tasks(tasks):impl DoubleEndedIterator<Item = &crate::lean::store::Task>
      evidence_id(index):String
      plain_findings(value):Result<Vec<serde_json::Value>> > require
      resolve_evidence(item,entries,tasks):Result<EvidenceProposal> ~44 > evidence_{entry,task},as_str,audited_tasks,require,EvidenceProposal
      AuditAssessment(C{evidence,approved,note})
      AuditEvidence(C{{criterion,task}_id,result,artifact})
      - Finding,criterion_belongs,task_matches,validate_assessment,evidence_ids
     extraction
      quoted_span(sources,entry,quote,source):Result<SourceSpan> > locate_quote,SourceSpan,SourceSpan.text
      - SubmittedEntry,yes×0,extraction_entry,validate_source_name,unique_quote_start
   storage
    changes/apply
     carry_unaffected_inputs(tx,change,versions,affected):Result<()> > as_str
     emit_change(tx,change,result,versions):Result<()> > event
     mark_affected_tasks(tx,tasks):Result<()> > retain_work_from_ids
     persist_entries(tx,change,before,state,revision):Result<Vec<serde_json::Value>> > persist_entry
     resume_if_approved(tx,change,user):Result<()> > enqueue_requirements_resume
     save_decision(tx,change,result,author,request_hash):Result<()>
     saved_decision(tx,change,request_hash):Result<Option<ChangeResult>> > require
     - next_entry_version,insert_{entry_version,version_row,entry_criteria,criterion,entry_relations}
    changes.rs
     - {existing,persist}_proposal
    entries
     entries(db,run,revision):Result<Vec<LedgerEntry>> > entry_{rows,from_row}
     EntryRow(C{id,version,revision,source_id,start,end,interpretation,mandatory,position,disposition})
     - EntryRelations,entry_{spec,relations},source_text_for_span,entry_criteria
    evidence
     queries
      audit_missing(store,run,revision,fingerprint):Result<bool> > db
      build_ledger_page(run,entries,offset,requested):LedgerPage > LedgerPage
      entry_gaps(entries):Vec<String>
      extraction_gap(store,conversation,run):Result<Vec<String>> > extraction_current
      filter_ledger_entries(entries,query,linked):Vec<LedgerEntry> > matches_ledger_filter
      linked_entry_ids(store,query):Result<Option<HashSet<String>>> > db
     status
      enforce_dependency_statuses(entries) > dependency_dependents,demote_unreviewed_parents
      set_ledger_statuses(entries,tasks,evidence,fingerprint):Result<()> > LedgerStatusIndex.build
      - LedgerStatusIndex,LinkedTasks,EvidenceApproval,linked_task_index,current_{approved_tasks,evidence_index},evidence_key,
        {disposition,linked,choose_linked,empty_linked}_status
      - LedgerStatusIndex: apply,entry_reviewed
    evidence.rs
     - audit_turns_bound,evidence_task_is_current,active_evidence_entry,validate_evidence_task_links
    reviews/assignments
     - {bind_task,bind_requirement}_inputs
    reviews.rs
     - validate_{review,review_session},review_turns_bound,validate_review_coverage,review_{sources_uncovered,producer},
       insert_review
    sources
     - write_or_check_artifact
    turns/rejections
     - insert_user_correction,queue_rejection_resume
    validation
     changes
      validate_change(db,change,state,user):Result<LedgerDiff> > validate_change_scope,entries,existing_specs,apply_entry_changes,make_diff
      - altered_entries,validate_entry_{change,authority}
     graph
      checked_graph(all):Result<()> > as_str,require,validate_entry
      - validate_{criteria,relations,composite_requirement,entry_cycle}
     impact
      impact(db,run,changed,before,all):Result<Vec<String>> > impacted_entries,reach_of,tasks_for_entries,expand_dependent_tasks
      same_meaning(old,new):bool
      - Reach,task_is_reached,plan_task_rows
   storage.rs
    - invalid,run,relations,{message,document}_source_text,independent,requirements_resume_exists,
      {resume,insert_resume}_message,emit_resume_events
   task_links
    resolve_covers(spec,entries):Result<()> > resolve_cover,push_unique
    validate_criterion_coverage(entries,covered):Result<()> > require
    validate_task_links(spec,entries):Result<()> > require
  mail
   lock_cancellable(lock,cancel):Option<MutexGuard<'a,()>>
  mcp
   dispatch
    call(shared,params):Value ~77 > scoped_tools,dispatch,is_terminal
    dispatch(shared,params):Result<Value> > as_str,current_turn,prepare_dispatch,invoke_handler,finish_dispatch
    error(id,code,message):Value
    result(id,result):Value
    - Prepared,require_open_turn,remember_session,replay_terminal,finish_{answer_wait,terminal}
   gateway
    listener
     spawn_listener(server,worker,halt) > transport.serve
    wait
     poll_wait(state,turn,cancel):Result<Option<Finish>> > take_finish
     take_finish(state):Option<Finish>
   gateway.rs
    is_terminal(name):bool
    receipt_payload(name,args):Value
    terminal_result(name,args):Result<super::super::turn_result::TurnResult>
    Finish(C{text})
    Gateway(C{shared,stop,address}) : Drop
     start():Result<Self> > Shared,spawn_listener
     url():&str
     token():&str
     begin(turn):Result<()>
     set_role(role):Result<()>
     set_task_contract(contract):Result<()>
     tool_names():Vec<&'static str> > scoped_tools
     set_handler(handler):Result<()>
     set_protocol_observer(observer):Result<()>
     connected():bool
     while_pending(turn,action):Result<bool>
     while_open(action):Result<Option<String>>
     session():Option<String>
     wait(turn,cancel,alive):Result<Finish> > wait_loop
     - drop
    Shared(C{token,state,changed})
    State(C{expected,finished,session,listed,handler,protocol_observer,role,contract,receipt,waiting,rejected_finish,protocol_rejected_at})
    = SERVER="antislop",TOOL="finish_turn",SUBMIT_PLAN="submit_plan",ASK="ask",WAIT="wait_for_answer",ASK_USER="ask_user",
      SUBMIT_REQUIREMENTS="submit_requirements",REVIEW="review",{LEDGER,TASK}_RESULTS,TOOLS,CANCELLED="agent turn cancelled",
      BODY
    - agent_exited
   schema
    addressed():Value
    audit():Value > object
    audit_review():Value > object
    ledger_entry():Value > object
    plain_review():Value > object
    planned_task():Value > object
    question():Value > object
    review():Value > object
    revision():Value
    selection():Value > object
   tools
    allowed_tools(role):Vec<&'static str> > {task,orchestrator}_tools,plain_reviewer
    scoped_tool_list(role,contract):Vec<Value> > scoped_tools,tool_list,as_str
    scoped_tools(role,contract):Vec<&'static str> > allowed_tools
    tool_list(role):Vec<Value> > allowed_tools,tool_json
    - {tool,turn,plan,control}_contract
   transport
    bridge():Result<()> > forward
    forward(input,output,url,token,conversation):Result<()> > gateway_client,forward_line
    serve(shared,request):Result<()> > authorized,respond,{read,handle}_message
    - with_session,write_reply,initialize_result,list_tools
  mod
   diff_revision_token(conversation,plan,id,base,head):String
   task_role(key):Option<(&str,bool)>
   task_state_digest(tasks):String
   ConversationSummary(C{id,goal,project,created_ms,status,active_plan,tasks_{total,done},revision_token})
   Core(C{store,native,weak,_lock,opened_ms,config,state,activity,revisions,worktrees,locks}) : Drop
    orchestrator_tool(&Arc<Self>,conversation,role,turn,tool,args):Result<Value> > {answer_agent,decide_approval}_tool,{retry_failed,resume_coordinating}_task
    answer_agent_tool(&Arc<Self>,conversation,args):Result<Value> > defer_question_tool,answer_agent_with_disposition,maybe_continue_plan
    defer_question_tool(conversation,args):Result<Value> > graph,required_text,agent_question,defer_question_to_user,notice
    submit_plan_tool(conversation,role,turn,args):Result<Value> > interviewing,workflow,planned_tasks,as_str,check_submitted_plan,select_team,record_result
    audit_tool(conversation,role,turn,tool,args):Result<Value> > submitted_{extraction,audit,audit_review},record_result
    revision_tool(&Arc<Self>,conversation,role,tool,args):Result<Value> > request_replan,required_text,revise_plan_tool
    revise_plan_tool(&Arc<Self>,conversation,args):Result<Value> > {prepare_plan,submit}_revision
    read_dependency_result(conversation,role,turn,args):Result<Value> ~136 > required_text,dependency_{reader_scope,reports},report_reference_error,referenced_worker_report,result_evidence,{dependency_report,page}_text
    dependency_reader_scope(conversation,role,turn):Result<DependencyScope> ~77 > task_{role,by_id},require,plan_conversation,dependency_inputs,flatten,as_str,DependencyScope,audit_dependency_scope,dependency_plan,plan_revision
    evidence_tool(conversation,role,turn,tool,args):Result<Value> > task_role,required_text,authorized_evidence,{evidence,page}_text
    completion_evidence(conversation,turn,args,result):Result<Vec<store::EvidenceReference>> ~46 > require,{result,authorized}_evidence
    require_open_agent_turn(conversation,role,turn):Result<()> > task_role,require_current_task_generation
    require_current_task_generation(turn,task):Result<()> > db,require
    question_tool(&Arc<Self>,conversation,role,turn,tool,args):Result<Value> > {ask_agent,wait_for_answer}_tool
    ask_agent_tool(&Arc<Self>,conversation,turn,args):Result<Value> > required_text,local_questions,ask_agent_question,wake,await_agent_answer
    wait_for_answer_tool(conversation,role,turn,args):Result<Value> ~63 > open_question_of_turn,local_questions,as_str,record_task_result
    finish_tool(conversation,role,turn,tool,args):Result<Value> > require_{plan_correction,refused_plan},record_result
    interview_tool(conversation,role,turn,tool,args):Result<Value> > require,interviewing,workflow,record_result
    report_reference_error(conversation,requested,history):Error ~46 > flatten,db,reference_error
    evidence_reference_error(conversation,requested,reason):Error > db,reference_error
    record_result(turn,result):Result<Value> > save_turn_result
    record_task_result(conversation,role,turn,result):Result<Value> > record_task_result_with_evidence
    record_task_result_with_evidence(conversation,role,turn,result,evidence):Result<Value> > validate_agent_result,save_turn_result_with_evidence,invalidate_open_questions
    check_submitted_plan(conversation,tasks,keep,team):Result<()> > submitted_graph,planning_run,ledger_entries,checked_team_plan,config_snapshot
    revised_tasks(conversation,plan,tasks):Result<Vec<TaskContract>> > plan_run,ledger_entries
    link_revised_task(task,baseline,entries):Result<TaskContract> > into_contract,restore_requirement_links
    await_agent_answer(conversation,turn,request):Result<Value> > agent_question,require
    validate_agent_result(conversation,role,turn,result):Result<()> > task_{role,by_id},assert_task_inputs,validate_task,agent_question
    agent_tool(&Arc<Self>,conversation,role,turn,tool,args):Result<Value> > require_open_agent_turn,evidence_tool,read_dependency_result,{task_result,finish,interview,orchestrator,submit_plan,audit,revision}_tool
    repair_integration(&Arc<Self>,conversation,task,spec,worktree,onto,change,outcome,files,cancel):Result<()> > Repair.new
    integrate(&Arc<Self>,conversation,task,spec,project,worktree,config,cancel):Result<()> > defer_agent_work
    writer_base(plan_id,spec,project):Result<String> > plan_{base,base_record},writer_{dependency_changes,base_scratch}
    record_plan_landing(plan_id,task,result):Result<()> > {plan,save_plan}_base_record
    submit_addressed(&Arc<Self>,conversation,text,explicit_to):Result<serde_json::Value> > address,require,dispatch_addressed
    open(state,config):Result<Arc<Self>> > state_lock,recover_agents_and_turns,make_core,restore_startup
    turn_inner(setup,cancel,prompt):Result<Option<String>> > turn_lock,lock_cancellable,TurnState.begin,TurnEvents.new,announce_started,execute,finish.finish
    settle_approval(&Arc<Self>,seat,approval,cancel,shown):Result<Verdict> ~64 > automatic_choice,approval_mode,as_str,open_approval,kind_name,ask_orchestrator_to_decide,close_approval,approval_state,shown
    decide_approval_tool(conversation,args):Result<Value> > approval_state,escalate_approval,notice,answer_approval
    answer_approval(conversation,request,choice):Result<()>
    approval_mode(conversation):Result<String>
    set_approval_mode(conversation,mode):Result<()>
    comment_at(&Arc<Self>,conversation,plan_id,target,text,round,run):Result<Comment> > comment_at_with_operation
    comment_at_with_operation(&Arc<Self>,conversation,{operation,plan}_id,target,text,round,run):Result<Comment> ~41 > require,comment_plan,diff_author,contextual_comment_label,persist_comment,CommentDraft,send_to_agent
    review_comments(&Arc<Self>,conversation,plan_id):Result<usize> > send_draft_comments,wake
    plans(conversation,offset,limit):Result<PlansPage> > plans_page
    plan_history(conversation,plan_id):Result<PlanHistory>
    set_comment(&Arc<Self>,conversation,id,status):Result<Comment> > set_comment_status,wake
    pause(conversation,paused):Result<()>
    restart_orchestrator(&Arc<Self>,conversation):Result<()> > add_message,drop,wake
    cancel(conversation)
    is_idle(conversation):bool > is_idle_with_plans
    delete_session(conversation):Result<()> > require,is_idle_with_plans
    busy():bool
    task_freshness_token(conversation,{plan,task}_id):Result<String> > task_unscoped,freshness_for_task
    config_snapshot():Arc<Config>
    replace_config(config)
    send(&Arc<Self>,conversation,text):Result<Message>
    start(project):Result<Conversation> > {record_project,project}_identity,create_conversation
    submit(&Arc<Self>,conversation,text):Result<serde_json::Value> > submit_addressed
    send_to_agent(&Arc<Self>,message):Result<serde_json::Value> > enqueue_and_refresh,steer,resume_after_delivery
    conversation_revision_token(conversation):Result<String> > latest_event_seq,paused,is_idle
    plan_revision_token(_conversation,plan_id):Result<String> > plan_run
    diff_revision_token(conversation,plan,id):Result<String> > task_unscoped,change
    conversations():Result<Vec<ConversationSummary>> > conversation_summary
    sessions(conversation):Result<Vec<store::SessionInfo>>
    history(conversation,after,limit):Result<store::HistoryPage> > history_page
    comment(&Arc<Self>,conversation,target,text):Result<Comment> > comment_scoped
    comment_scoped(&Arc<Self>,conversation,plan_id,target,text):Result<Comment> > comment_at
    orchestrator_turn(&Arc<Self>,conversation,upto,cancel):Result<()> > prepare,ask,context.finish
    planner_turn(&Arc<Self>,conversation,upto,cancel):Result<()> > planning_requirements_ready,planner_turn_context,run_planner_turn,handle_planner_answer
    require_refused_plan(conversation,turn):Result<()> > last_orchestration_answered,asked_since,messages_by
    request_replan(conversation,reason):Result<serde_json::Value> > last_orchestration_answered,messages_by,planning_run,add_message
    unanswered(conversation):Result<Option<u64>> > last_orchestration_{turn,answered},messages_after
    asked_since(conversation,answered,upto):Result<Vec<Message>> > messages_after
    planner_owns_turn(conversation,upto):Result<bool> > last_orchestration_answered,asked_since,includes_comment,awaiting_first_plan,includes_replan
    handle_plan_answer(&Arc<Self>,answer,cancel):Result<()> > propose,PlanProposal,record_invalid_plan,handle_plan_words
    require_plan_correction(conversation,turn):Result<()> > db,has_uncorrected_review_draft,require
    submitted_graph(conversation,tasks,keep):Result<Vec<TaskContract>> > kept_contracts
    keepable_tasks_note(conversation):Result<Option<String>> > approved_task_history,keepable
    review_plan(&Arc<Self>,conversation,project,plan,specs,config,cancel):Result<std::result::Result<Verdict,String>> > record_plan_review_start,team,planning_gaps,{team,dependency}_note,review,append_planning_gaps
    cancel_proposed_plan(plan):Result<()> > set_plan_status
    requeue_rejected_plan(conversation,plan,review):Result<bool> > previous_plan_rejection,open_obligations,notice,record_proposed_plan_rejection
    planning_requirements(&Arc<Self>,conversation,addressed,cancel):Result<Option<Run>> > record_addressed_comments,count_since_requirements_reset,ensure_requirements,record_requirements_failure
    covered_plan(conversation,run,specs):Result<Option<Vec<TaskContract>>> > ledger_entries,persist_proposed_tasks,link_plan_run,set_plan_status,notice,retry_plan_from_ledger
    prepare_plan(conversation,specs,run):Result<Option<PreparedPlan>> > persist_proposed_tasks,link_plan_run,bind_proposed_plan,PreparedPlan,last_orchestration_answered
    publish_plan(&Arc<Self>,conversation,text,addressed,plan,specs,review):Result<()> > resolve_plan_comments,plan_{revision_token,event_payload}
    propose(&Arc<Self>,conversation,project,text,proposal,addressed,cancel):Result<()> > prepare_{proposal,plan},complete_proposal,ProposalPublication
    upstream(plan_id,spec):Result<String> > dependency_inputs
    dependency_tasks(plan_id,spec):Result<Vec<Task>> > collect_dependencies
    plan_report(&Arc<Self>,conversation,plan_id,cancel):Result<String> > task_results,report_{compliance_gaps,intro},append_plan_audit,PlanAuditInput,append_compliance_gaps
    resume_plan(&Arc<Self>,conversation,plan_id):Result<Vec<Task>> ~49 > plan_{belongs_to_conversation,run},extraction_needs_refresh,add_message,wake,all,activate_reviewed_plan
    recover_task(&Arc<Self>,req):Result<TaskRecoveryResponse> > recover_task_as
    review(&Arc<Self>,conversation,role,plan_id,project,cancel,prompt):Result<std::result::Result<Verdict,String>> > config_snapshot,last_answered,messages_by,review_setup,turn,prompt,review_{request,outcome}
    turn(setup,cancel,prompt):Result<Option<String>> > turn_inner
    review_and_apply_revision(&Arc<Self>,context,lease):Result<()> > affected_tasks,approved_work,revision_checkpoints,graph,RevisionBarrier.new,review_revision,ReviewData,require,apply_after_review
    submit_revision(&Arc<Self>,conversation,request_key,revision):Result<serde_json::Value> > reserve_revision,revision_job,process_fresh_revision
    finish_blocked_plan(&Arc<Self>,conversation,tasks,cancel) > blocked_report,add_message,wake
    report_stopped_tasks(&Arc<Self>,conversation,plan_id) > stopped_report,add_message,wake
    plan_runner(&Arc<Self>,conversation,plan_id,cancel,tasks_write) > warn_of_uncommitted,run_plan_cycle,notice,clear_running_plan
    has_runnable_dependents(plan_id):Result<bool> > dependencies_approved
    warn_of_uncommitted(conversation,writes) > uncommitted_notice
    wake(&Arc<Self>,conversation)
    orchestrate(&Arc<Self>,conversation,cancel) > next_orchestration_{upto,turn},stop_orchestrator
    start_plan_with_scope_lock(&Arc<Self>,conversation,plan_id,queue):Result<Vec<Task>> > ensure_plan_can_start,require,validate_plan_inputs,ensure_profiles_launch,db,remove,announce_plan_tasks,drop,spawn_plan_runner
    resume_agent_reviews(&Arc<Self>,conversation):Result<()> > register_agent_review,spawn_agent_review_worker
    review_change(&Arc<Self>,conversation,plan,id):Result<()> > prepare_code_review,remove
    open_plan_comments(conversation,plan_id):Result<Vec<Comment>>
    activate_reviewed_plan(&Arc<Self>,conversation,plan_id):Result<Vec<Task>> > plan_review_approved,open_plan_comments,start_plan
    reject_plan(&Arc<Self>,conversation,plan_id,reason):Result<()> > set_plan_status,require,rejection_reason,announce_rejected_tasks,record_plan_rejection,wake
    artifact_inputs(task,spec):Result<Inputs> > plan_conversation,artifact_receipt,{recover,freeze}_artifact_input
    artifact_commits(task,spec):Result<BTreeMap<String,String>> > artifact_inputs
    prepare_artifact_outputs(task,spec,project):Result<BTreeMap<String,String>> > artifact_commits
    validate_artifact_delta(task,spec,worktree):Result<()> > artifact_commits
    deliver_artifact_change(task,spec,project,base,change):Result<git::Integration> ~45 > artifact_commits,validate_change_paths,record_artifact_delivery,ArtifactReceipt
    record_check(conversation,task,update) > assert_task_inputs,{set,publish}_check
    run_plan(&Arc<Self>,conversation,plan_id,cancel) > scheduler.run
    run_task(&Arc<Self>,conversation,task,cancel):Result<()> > {task_worktree,run_task_with}_lease
    run_task_with_lease(&Arc<Self>,conversation,task,cancel,lease):Result<()> > Execution.new
    freshness_for_task(conversation,plan_id,task):Result<String> > freshness_seed,append_run_revision
    task_unscoped(plan,id):Result<Task> > task_in_plan_by_id
    task(plan,id):Result<Task> > task_unscoped,plan_conversation,freshness_for_task
    task_checks(plan,id):Result<Task> > task_unscoped
    change(conversation,task):Result<(PathBuf,String,Option<String>)> > as_str,worktree_path
    diff(conversation,plan,id):Result<serde_json::Value> > config_snapshot,truncate_patch,diff_revision_token,reviewer_profile
    cancel_changed_task_inputs(before,after) > queue_worktree_cleanup
    invalidate_task_dependents(conversation,task):Result<()> > graph,{assert,cancel_changed}_task_inputs,announce_task
    resume_coordinating_task(&Arc<Self>,conversation,role,turn,name,expected,resolution):Result<serde_json::Value> > resume_request,recover_task_as
    retry_failed_task(&Arc<Self>,conversation,role,turn,name,guidance):Result<serde_json::Value> > retry_request,recover_task_as
    recovery_task(conversation,role,turn,name):Result<(String,Task)> > dependency_plan,require,accepted_plan,as_str
    recover_task_as(&Arc<Self>,req,actor):Result<TaskRecoveryResponse> > graph,prepare.load,interrupt_if_active,actions.apply,response.build
    announce_task(conversation,task,reason) > task_state_digest
    set_status(conversation,task,status):Result<()> > stop
    stop(conversation,task,status,reason):Result<()> > record_stop_reason,update_task,record_stop_progress,queue_worktree_cleanup,announce_task
    unblock_dependent_tasks(conversation,{plan,approved_task}_id):Result<()> > refresh_answer_waits,tasks_to_unblock,{update,announce}_task
    check_comments(conversation,plan,spec_id):String
    notice(conversation,text)
    maybe_continue_plan(&Arc<Self>,conversation,plan_id):Result<()> > refresh_answer_waits,all,start_plan_with_scope_lock
    start_plan(&Arc<Self>,conversation,plan_id,queue):Result<Vec<Task>> > plan_running,start_plan_with_scope_lock
    lease_worktree(path):WorktreeLease<'_> > wait,WorktreeLease
    wait_for_worktree_available(paths,cancel):bool > all
    try_lease_worktree(path):Option<WorktreeLease<'_>> > WorktreeLease
    try_pin_worktree(path):Option<WorktreePin<'_>> > WorktreePin
    try_task_worktree_lease(task):Result<(bool,Option<WorktreeLease<'_>>)> > task_worktree_path,try_lease_worktree
    task_worktree_lease(task):Result<Option<WorktreeLease<'_>>> > task_worktree_path,lease_worktree
    task_worktree_path(task):Result<Option<PathBuf>> > task_in_plan_by_id,worktree_path,has_delivery
    queue_worktree_cleanup(task)
    drain_worktree_cleanup() > cleanup_task_worktrees,queue_worktree_cleanup,cleanup_notice
    release_task_views(id,keep) > release_task_views_with_hook
    compliance_audit(&Arc<Self>,conversation,plan,cancel):Result<Vec<String>> > audit_scope,{produce,review}_audit,AuditTurns,finish_audit
    apply_checked_ledger_change(proposal_id,diff_hash,change):Result<ChangeResult> > check_ledger_proposal,{applied,preview,apply}_ledger_change
    announce_affected_ledger_tasks(change,result):Result<()> > cancel_tasks,queue_worktree_cleanup,task_by_id,announce_task
    requirements_turn(&Arc<Self>,scope,reviewer,prompt,cancel):Result<(String,String)> > require,plan_{conversation,run},publish_requirements_progress,requirements_{role,turn_setup,turn_metadata},ledger_context,run_requirements_agent,{last,bind_requirements}_turn
    refresh_requirements_if_needed(&Arc<Self>,conversation,cancel):Result<()> > planning_run,extraction_needs_refresh,ensure_requirements,accepted_plan,plan_run,as_str,maybe_continue_plan
    ensure_requirements(&Arc<Self>,conversation,cancel):Result<Run> > {prepare,extract,accept}_requirements
    register_missing_sources(conversation,run,user):Result<bool> > ledger_sources,execution_decision_ids,{missing,register_message}_source
    validate_extraction(conversation,run,existing,extraction):Result<LedgerChange> > require,preview_ledger_change
    submitted_audit(conversation,args):Result<(Vec<serde_json::Value>,Vec<AuditEvidence>)> > plain_findings,planning_run,ledger_entries,run_tasks,resolve_evidence
    submitted_audit_review(conversation,args):Result<(String,Vec<AuditAssessment>)> > as_str,proposed_evidence_ids,validate_assessments
    submitted_extraction(conversation,args):Result<Extraction> > planning_run,source_texts,{ledger,extraction}_entries,Extraction,validate_extraction
    ledger_fingerprint(conversation):Result<String>
    ledger_query(query):Result<LedgerPage> > planning_run,ledger_fingerprint,list_ledger
    apply_user_ledger_change(&Arc<Self>,proposal_id,diff_hash,change):Result<ChangeResult> > apply_checked_ledger_change,announce_affected_ledger_tasks,drop,has_pending_requirements_approval,wake
    ledger_context(conversation,run_id):Result<String>
    full_ledger_sources(conversation,run_id):Result<String> > messages_after,ledger_sources,append_{ledger_source,interview_summary}
    validate_ledger_plan(conversation,plan,run_id):Result<()> > extraction_current,{task_has_ledger,assert_task}_inputs
    turn_lock(conversation,role):Arc<Mutex<()>>
    mail_address_task(conversation,plan,address):Result<Option<(Task,String)>> > parse_mail_address,task_by_id,plan_conversation
    new_handoff(id,round,cwd,task):Result<RoundHandoff> > handoff_seed,base_handoff
    add_handoff_requirements(task,spec,handoff):Result<()> > plan_{conversation,run},ledger_entries
    add_handoff_comments(task,handoff):Result<()> > plan_conversation,spec_target
    add_protected_paths(_task,spec,handoff):Result<()> > has_delivery
    round_interruptions(id,round):Result<Vec<RoundInterruption>> > db,RoundInterruption
    recover_previous_rounds(id,round,cwd,handoff):Result<()> > handoff_rounds,merge_round_{result,report}
    matching_round_session(conversation,role,provider,task,round,profile,cwd,add_dirs):Result<Option<String>> > scoped_turn,session_for_context
    session_for_turn(conversation,role,provider,policy,profile,cwd,add_dirs):Result<Option<String>> > {last,matching_round}_session
    round_handoff(id,round,cwd):Result<RoundHandoff> > task_by_id,new_handoff,add_handoff_{requirements,comments},add_protected_paths,round_interruptions,recover_previous_rounds
    fail_scheduled_task(conversation,task,error) > append_report_entry
    stop_task(conversation,task,role,cancel):Result<()> > last_turn,failed_turn_status,record_failed_turn
    defer_agent_work(conversation,task):Result<bool> > flatten,pending_mail,set_status
    wait_for_question(conversation,task,request,reviewer):Result<()> > update_task,park_task_for_question,announce_task
    terminal_open(conversation,role,columns,rows,retry):Result<Screen> > size
    terminal_read(conversation,role,handle,after):Result<Option<Screen>>
    terminal_input(conversation,role,handle,sequence,input):Result<()>
    terminal_resize(conversation,role,handle,columns,rows):Result<()> > size,require
    terminal_detach(conversation,role,handle):Result<()> > remove
    - revision_view,plan_base_path,refresh_delivered_tasks,{conversation,approved_plan}_status,replaced_graph,
      planner_submitted_plan,record_{comment_reply,plan_lint},{run,store}_report_audit,require_revision_review,
      checkpoint_revision_work,{cancel,wait_for}_affected_tasks,{spawn,finish}_revision_worker,
      prepare_{revision,revision_contents},revised_specs,ensure_revision_accepted,prepared_revision,wait_for_revision_jobs,
      finish_{plan_run,audited_plan},audited_plan_{snapshot,changed},publish_plan_report,
      {clear_agent,reserve,run_code}_review,unchanged_recovery_request,cleanup_{view_path,task_worktrees_with_hook},
      check_audit_scope,save_audit_{evidence,review},complete_audit,requirements_{failure,feedback},
      {analyze,apply_extracted,propose_extracted}_requirements,require_extracted_entries,append_interview_question,
      primary_project,session_matches_context,execution_signature
   RecoveryAction(E{IntegrateWithConsent,KeepOnBranch,ReviewManualSnapshot,RetryWithGuidance,RestartAgent,CancelTask})
   TaskRecoveryRequest(C{conversation,plan,task,action,consent_files,freshness_token,message})
   TaskRecoveryResponse(C{task,status,round,integrated_files,preserved_branch,new_freshness_token,notice})
   = COMMENT_ORCHESTRATOR="comment-orchestrator",PLANNER="planner",REPLAN="replan-request",ORCHESTRATION_ROLES,
     COMMENT_PREFIX="[user comment #"
   - SessionPolicy,{record_message_and_wake,wake}_locked
  native
   approvals
    detect(provider,screen):Option<Approval> > claude,codex,opencode,agy,gemini
    Approval(C{kind,summary,choices,fingerprint,server})
     mcp_server():Option<&str>
     choice(id):Option<&Choice>
    Choice(C{id,label,decision,keys})
    Decision(E{Allow,AllowAlways,Deny,Other})
    Key(E{Char,Enter,Escape,Up,Down,Right})
    Kind(E{Command,Mcp,Confirm,Trust,Other})
    Verdict(E{Choose,AnsweredInTerminal})
    - agy_tool,mcp_heading,numbered_choices,number,{marked,navigation}_choices,navigate,split_shortcut,choice,decision,
      finish,{is,is_solid}_rule
   readiness
    Tracker(C{saw_work,reminded})
     observe(provider,screen,approval_open,paused):bool > classify
    - ScreenState,is_{ready_prompt,work_signal},has_braille_spinner,is_status_label
   registry/history_view
    restore_view_directory(core,context):Result<()> > turn_scope,task_by_id,plan_conversation,task_path_component,worktree_directory,queue_worktree_cleanup
   registry.rs
    - recorded_round,validate_profile,restored_agent_spec
   session
    Session: start(core,conversation,spec,signature,role):Result<Self>; identity():Option<String>; touch(); check_alive():Result<()> > diagnostic,screen_stop,approval; run(registry,turn,prompt,cancel):Result<agent::Outcome> > check_alive,require,execute,identity,session,store,touch; steer(text,interrupt):Result<Option<String>> > check_alive,while_open; failed_before_start():bool; set_tool_handler(handler):Result<()> > set_handler; set_approver(approver):Result<()>; set_protocol_observer(observer):Result<()>
    - Session: build,serve_approval,dialog_gone,wait_{ready,for_connection,for_settle},submit_turn,wait_for_result,
               remind_if_ready,describe_result_error,learn_identity,opencode_session
   stops
    screen_stop(provider,screen):Option<StopCause> ~51
    StopCause(E{ProviderLimit,LoginRequired,UnexpectedDialog,ProcessExit,Cancelled})
     marker():&'static str
     diagnostic(detail):crate::Error
  native.rs
   invalid(error):Error
   recover(store,state):Result<()> > reap,kill_leftovers
   Registry(C{sessions})
    reap_idle():Result<()>
    restore(core,conversation,role_key):Result<Arc<Session>> > graph,parse_role_key,recorded_{turn,context},restored_spec,require,review_execution,turn_scope,try_pin_worktree,prepare_scoped
    retire_directory(path):Result<bool> > try_lock
    shutdown():Result<()> > store
    prepare_scoped(core,conversation,role,spec,scope):Result<Arc<Session>> > task_by_id,session_signature
    get(conversation,role):Result<Option<Arc<Session>>>
    claimed():Vec<String>
   Session(C{conversation,core,spec,signature,_settings,gateway,terminal,screen,identity,started_ms,stop,executing,primed,startup_failed,touched,approver})
   = EXECUTION_FAILED,APPROVAL_LINES=40
   - turn_text
  paths
   config_dir():Result<PathBuf> > environment_path
   project_identity(project):Result<String> > git
   project_state(project):Result<PathBuf> > state_{in_root,root}
   state_root():Result<PathBuf> > resolve_state_root,environment_path
  plan_revision
   apply/tasks
    validate_revision_coverage(specs,entries):Result<()>
    - reset_affected_task,update_preserved_task,insert_revision_{task,ledger_links},record_contract_event
   apply.rs
    - RevisionApply
  plan_revision.rs
   PlanRevision(C{plan,base_revision,reason,tasks,retire,redo,replacements})
    merge(baseline):Result<Vec<TaskContract>> ~44 > require_revision_reason,baseline_specs,validate_retire_and_redo,has_delivery,len,remove_retired,merge_updates,order_graph
    - validate_update
   RevisionLease(C{requirement_revision,request})
   - replace_or_append,same_contract
  plan_tests
   - test_tempdir
  process_tree
   kill_descendants(root,marker) > processes,descendants,tagged,kill
   kill_leftovers(state) > processes,kill,tagged
   marker(state):String
   = MARKER="ANTISLOP_AGENT"
   - Process,listing,parse
  progress
   blocks(finding):bool
   fixed_earlier(earlier,verdict):bool > blocks,as_str
   open_obligations(verdict):BTreeSet<String> > is_open,blocks,as_str
   Progress(C{previous})
    record(open):bool
    record_round(open,fixed_earlier):bool > record
   Run(C{basis,rejections,progress,findings})
    resume(record,spec,generation):Self > basis,Run
  projects
   Projects(C{file})
    from_env():Result<Self> > config_dir
    at(dir):Self
    list():Vec<PathBuf> > read
    remember(project):Result<()> > read,truncate
   - Stored
  risks
   for_kind(kind):impl Iterator<Item = (String,String)> > risks
   - Risk
  rounds/handoff
   - HandoffSeed,collect_previous_changes,should_mark_interrupted
  rounds.rs
   execution_signature(profile):Result<String> > execution_signature_fields
   spec_signature(profile,effort,spec):Result<String> > execution_signature_fields
   RoundHandoff(C{round,change_kind,worktree,report,verdict,files,base,result,interrupted,interruptions,requirements,comments,protected_paths,primary})
   RoundInterruption(C{round,role,status,reason})
  run_context
   RunContext(C{turn_id,conversation,role,provider,profile,cwd,add_dirs,provider_{thread_id,session_id}})
   - EncodedContext
  store
   agent_exchange
    answer_question_tx(tx,conversation,request,answer):Result<()> > {question,record_question}_answer
   approvals
    ApprovalRequest(C{turn,role,provider,kind,summary,options,fingerprint,decider})
    = SUPERVISED="supervised",AUTONOMOUS="autonomous"
   artifacts
    ArtifactReceipt(C{task,generation,commit})
   comment_mail_state
    - resolve_comment
   comments_and_mail
    comments
     insert
      append_comment_event(store,tx,conversation,comment):Result<()>
      append_scoped_comment_event(store,tx,conversation,comment,round,run):Result<()> > comment_payload
      insert_comment_message(tx,conversation,text,now,plan_id):Result<u64>
      insert_comment_row(tx,conversation,comment,stored_text,now):Result<u64>
      insert_draft_comment(store,tx,conversation,comment,stored_text):Result<Comment> > now_ms,append_comment_event
      persist_comment(store,tx,conversation,comment,round,run,text):Result<Comment> > now_ms,insert_comment_row,{insert,link}_comment_message,save_comment_metadata
      set_comment_message(tx,comment_id,message):Result<()>
     updates
      append_reopened_message_event(tx,conversation,comment,text):Result<()>
      draft_comments(store,tx,conversation,plan_id):Result<Vec<Comment>> > Store.comment_rows
      draft_message(drafts):String
      ensure_comment_was_sent(comment,status):Result<()>
      insert_draft_message(tx,conversation,plan_id,text,now):Result<u64>
      load_comment(store,tx,id):Result<Comment> > Store.comment_rows
      open_draft_comments(store,tx,conversation,drafts,message,now):Result<Vec<Comment>> > reopen_draft_comment
      reopen_comment(store,tx,conversation,id,comment):Result<()> > update_reopened_comment,append_reopened_message_event
      reopen_draft_comment(store,tx,conversation,comment,message,now):Result<()> > append_comment_event
      update_comment_status(tx,conversation,id,status):Result<()>
      update_reopened_comment(tx,id,message):Result<()>
    mail
     append_mail_event(tx,conversation,mail,stored_text):Result<()>
     append_mail_source(tx,mail_id,conversation,turn):Result<()>
     insert_mail_row(tx,conversation,role,mail,source_turn,stored_text):Result<Mail> > now_ms,Mail,append_mail_source
   dependency_results
    DependencyReports(C{retained,producer,current,reports})
    ReportReference(C{result_ref,round,input_generation,bytes,result_type,producer_task,current_inputs})
   evidence
    EvidenceReference(C{id,label,bytes,sha256,producer_turn})
    = MAX_EVIDENCE_BYTES,MAX_RESULT_EVIDENCE=64
    - evidence_row
   mail_and_events/event_encoding
    - query_{turn_plan,task_plan}
   messages_and_turns
    - new_turn,persist_{turn,turn_scope}
   plan_history
    - plan_history_turn_rows,turn_belongs_to_plan,plan_{history_agents,agent_for_turn},add_{report,verdict}
   plan_tasks
    checks
     - StoredTaskContent,encode_snapshot_field,retain_previous_result,continue_prior_round,update_task_row,insert_task_round
    dependencies
     - task_contracts,is_closing_task,{earlier,collect_earlier}_checks,earlier_check_is_approved,selected_report
   plan_tasks.rs
    - task_generation
   session
    - {read,decode}_turn_row
   sessions_and_plans
    carry
     carry_approved(tx,conversation,plan_id):Result<Option<String>> > source_plan,rows,carry_{plan_tasks,notice}
     - dependencies_are_current,source_changes,same_contract,carry
    recovery
     close_open_approvals(tx):Result<()>
     insert_proposed_tasks(tx,conversation,plan_id,specs,now):Result<Vec<Task>> > insert_proposed_task
     record_turn_interruptions(tx):Result<()> > running_turn_rows,record_turn_interruption,now_ms
     running_orchestrators(tx):Result<Vec<String>>
     unfinished_plans(tx):Result<Vec<(String,String)>>
     - interrupted_report,proposed_task
   setup_and_conversations/schema
    initialize_base_tables(transaction):Result<()>
    initialize_plan_tables(transaction):Result<()>
   setup_and_conversations.rs
    - insert_message_event,restart_completed_workflow,{create,insert}_requirements_run,link_planning_run,
      append_{requirements_events,run_event}
   turn_status
    TurnStatus(E{Running,Done,Failed,Cancelled,Interrupted})
     as_str():&'static str
     parse(text):Result<Self>
  store.rs
   now_ms():i64
   Comment(C{id,plan_id,target,label,text,status,message})
   Conversation(C{id,project,created_ms})
   Event(C{seq,{turn,plan}_id,kind,payload,created_ms})
   HistoryPage(C{events,after,next_after,has_more,total,{oldest,latest}_seq,history_unavailable})
   Mail(C{id,sender,recipient,kind,text,created_ms,delivered_{turn,ms},state})
   MailTarget(C{role,newest,{task,plan}_id})
   Message(C{seq,author,text})
   PlanAgentInfo(C{role,provider,model,task_{id,label},turns_count,status})
   PlanHistory(C{run_id,id,conversation_id,created_ms,status,is_active,tasks,agents,turns,reports,verdicts})
   PlansPage(C{plans,offset,limit,total,has_more,active_plan_id})
   SessionInfo(C{{turn,plan}_id,role,provider,session_id,status,error,upto_message,{started,finished}_ms,pid})
   Store(C{connection,content,content_write,operations})
    history_content(conversation,owner,offset):Result<crate::lean::content::Chunk> > chunk,history_reference
    enqueue_agent_message(request):Result<Value> > agent_message_signature,db,existing_agent_message,validate_agent_recipient,now_ms,insert_agent_mail,update_agent_task
    invalidate_task_dependents(conversation,task):Result<()> > db,invalidate_dependents,now_ms
    mark_steered(conversation,id,turn):Result<bool> > db,now_ms
    requeue_steered(turn):Result<Vec<u64>> > db
    attribute_mail(conversation,ids,turn):Result<()> > db
    assert_comment_run(conversation,plan,target,run):Result<()> > db
    comment_scope(conversation,id):Result<Value> > db
    history_messages(conversation,scope,after,limit):Result<Value> > validate_history_scope,history_message_rows,shape_history_messages
    session_events(conversation,turn,after,limit):Result<Value> > session_events_with_messages
    session_events_with_messages(conversation,turn,after,limit,include_messages):Result<Value> > require_session,db,load_session_event_rows,shape_session_events,session_messages
    task_rounds(task):Result<Vec<Value>> > db,{stored,add_historical}_rounds,add_current_round
    task_round(task,round):Result<Value> > task_round_with_references
    task_round_with_references(task,round,references):Result<Value> > round_record_or_current,hydrate_round_fields,attach_round_execution
    handoff_rounds(task,through):Result<Vec<(u32,Value)>> > load_handoff_round_rows,db,shape_handoff_round
    completed_worker_report(task,round):Result<Option<(u32,String)>> > db
    handoff_field(value,json):Result<Value>
    initialize_history(connection):Result<()>
    agent_mail_user_message(conversation,mail):Result<Option<u64>> > db
    ledger_entries(conversation,run_id,revision):Result<Vec<LedgerEntry>> > run,require,entries
    preview_ledger_change(change,state):Result<LedgerDiff> > db
    propose_ledger_change(change,state):Result<(String,LedgerDiff)> > propose_ledger_change_as
    propose_agent_ledger_change(change,state,turn):Result<(String,LedgerDiff)> > check_analyst_author,propose_ledger_change_as
    apply_ledger_change(change,state,user):Result<ChangeResult> > require,apply_ledger_change_as
    apply_analyst_ledger_change(change,state,turn):Result<ChangeResult> > check_analyst_author,apply_ledger_change_as
    ledger_diff(conversation,run_id,from,to):Result<LedgerDiff> > ledger_entries,changed_entry_ids,impact,ledger_diff_result
    ledger_status_tasks(run_id):Result<Vec<LedgerStatusTask>> > db
    add_ledger_evidence(conversation,run_id,reviewer_turn,assessment):Result<Evidence> > run,validate_evidence_{content,sessions,task,criterion},Evidence,insert_evidence
    ledger_evidence(conversation,run_id,entry):Result<Vec<Evidence>> > run,Evidence,EvidenceProposal
    ledger_statuses(conversation,run_id,revision,fingerprint):Result<Vec<LedgerEntry>> > ledger_{entries,evidence,status_tasks},{set_ledger,enforce_dependency}_statuses
    list_ledger(query,fingerprint):Result<LedgerPage> > linked_entry_ids,ledger_statuses,build_ledger_page,filter_ledger_entries
    requirements_gaps(conversation,run_id,fingerprint):Result<Vec<String>> > extraction_gap,ledger_statuses,entry_gaps,audit_missing
    bind_task_ledger(conversation,run_id,plan):Result<()> > ledger_entries,run_tasks,prior_covered_criteria,bind_current_tasks
    rejected_requirements_feedback(conversation,run_id):Result<Option<serde_json::Value>> > run,decode_content
    requirements_review_findings(conversation,run_id):Result<String> > run,review_verdict
    task_generation(task):Result<i64> > db
    assert_task_inputs(task):Result<()> > require,task_generation
    record_ledger_review(conversation,run_id,revision,kind,producer,reviewer,fingerprint,approved,note):Result<()> > db,LedgerReview,persist_review
    accept_extraction(conversation,run_id,revision,producer):Result<()> > db,validate_extraction_revision,record_extraction_acceptance
    extraction_needs_refresh(conversation,run_id):Result<bool> > extraction_current
    extraction_current(conversation,run_id):Result<bool> > run
    create_run(conversation,goal):Result<Run> > conversation,require,run_start_message,event
    run(conversation,id):Result<Run> > db
    planning_run(conversation):Result<Option<Run>> > run
    select_planning_run(conversation,run_id):Result<()> > run
    approved_requirements_producer(conversation,run_id,revision):Result<Option<String>> > run
    has_pending_requirements_approval(conversation):Result<bool> > db
    recover_approved_requirements():Result<()> > db,enqueue_requirements_resume
    latest_run(conversation):Result<Option<Run>> > run
    plan_run(plan):Result<Option<String>> > db
    link_plan_run(conversation,plan,run_id):Result<()> > run,require,all,plan,task_in_plan
    execution_decision_ids(conversation):Result<Vec<String>> > db,flatten
    register_message_source(conversation,run_id,message):Result<Source> > run,{message,save}_source
    register_document_source(conversation,run_id,text,state):Result<Source> > run,write_source_artifact,Source,save_source
    ledger_sources(conversation,run_id):Result<Vec<Source>> > run,Source
    ledger_source_text(conversation,run_id,source,state):Result<String> > run,source_text
    reject_ledger_proposal(conversation,run_id,id,hash,comment):Result<()> > require,db,load_rejection,review_rejection_retry,validate_proposal_state,persist_rejection
    has_pending_ledger_proposal(run_id):Result<bool> > db
    has_rejected_ledger_proposal(run_id):Result<bool> > db
    check_ledger_proposal(id,hash,change):Result<()> > db,saved_proposal_diff,require
    ledger_entry_history(conversation,run_id,id):Result<Vec<LedgerEntry>> > run,entries
    bind_ledger_turn(conversation,run_id,turn,revision,phase,sources):Result<()> > db,validate_turn_{revision,owner},record_source_coverage
    task_has_ledger_inputs(task):Result<bool> > db
    set_run_completion(conversation,run_id,revision,complete):Result<()> > run,require
    applied_ledger_change(change):Result<Option<ChangeResult>> > db,require
    list_runs(conversation):Result<Vec<Run>> > conversation,run
    latest_ledger_proposal(conversation,run_id):Result<serde_json::Value> > run,saved_proposal_diff,require
    check_analyst_author(change,turn):Result<()> > run,require
    run_tasks(conversation,run_id):Result<Vec<crate::lean::store::Task>> > run,plan
    ledger_entry_tasks(conversation,run_id,entry):Result<Vec<serde_json::Value>> > run
    plan_revision(plan):Result<i64> > db
    apply_plan_revision(conversation,revision,baseline,specs,affected,lease):Result<()> > revision_apply_context,db
    initialize_run_context(connection):Result<()>
    record_run_context(context):Result<()> > encode_context,db,require_running_turn,insert_context
    run_context(conversation,turn):Result<Option<RunContext>> > db
    ask_agent_question(conversation,turn,key,question):Result<String> > db,previous_question,ensure_question_capacity,insert_agent_question
    open_question_of_turn(turn):Result<Option<String>> > db
    agent_question(conversation,request,turn):Result<Value> > db
    answer_agent_question(conversation,request,answer):Result<Value> > db,answer_question_tx
    save_turn_result(turn,result):Result<()> > save_turn_result_with_evidence
    save_turn_result_with_evidence(turn,result,evidence):Result<()> > db
    latest_role_result(conversation,role):Result<Option<super::super::turn_result::TurnResult>> > db
    local_questions(turn):Result<Vec<Value>> > db
    invalidate_open_questions(turn):Result<usize> > db
    defer_question_to_user(conversation,request,explanation):Result<Value> > db
    park_task_for_question(task,request):Result<String> > db
    answer_agent_with_disposition(conversation,request,answer,disposition,target):Result<Value> ~57 > answer_agent_question,refresh_answer_waits,agent_question
    refresh_answer_waits(conversation):Result<()> > db
    approval_mode(conversation):Result<String> > db
    set_approval_mode(conversation,mode):Result<()> > conversation,append_event
    team(conversation):Result<crate::lean::tasks::Team> > db
    team_note(conversation):Result<String> > db
    select_team(conversation,team,reason):Result<()> > append_event
    open_approval(conversation,request):Result<String> > append_event
    approval_state(id):Result<(String,Option<String>,String)> > db
    answer_approval(conversation,id,choice,by):Result<()> ~66 > flatten,append_event
    escalate_approval(conversation,id):Result<()> > append_event
    close_approval(conversation,id,reason):Result<()> > append_event
    artifact_output_history(task):Result<Vec<String>> > db,flatten
    artifact_receipt(conversation,path):Result<Option<ArtifactReceipt>> > db,ArtifactReceipt
    record_artifact_delivery(conversation,path,receipt):Result<()> > db
    artifact_input(task,path):Result<Option<Option<ArtifactReceipt>>> > db,ArtifactReceipt
    recover_artifact_input(task,path,receipt):Result<Option<ArtifactReceipt>> > artifact_input
    freeze_artifact_input(task,path,source):Result<Option<ArtifactReceipt>> > artifact_input
    unfinished_checkpoints(conversation,plan):Result<Vec<Value>> > db,checkpoint_source
    revision_checkpoints(conversation,revision):Result<Vec<Value>> > task_in_plan_by_id,checkpoint_source
    inherit_revision_checkpoint(tx,conversation,plan,source,target):Result<()> > checkpoint_source
    resolve_plan_comments(conversation,upto):Result<()> > resolve_listed_plan_comments
    resolve_listed_plan_comments(conversation,upto,only):Result<()> > db,comment_rows,resolve_comments
    pending_plan_comment_ids(conversation):Result<Option<Vec<u64>>> > db
    plan_document(conversation,plan):Result<Option<String>> > flatten,db
    set_paused(conversation,paused):Result<()> > db
    paused(conversation):Result<bool> > db
    comments(conversation,status):Result<Vec<Comment>> > comment_rows,db
    comment_payload(comment):Result<Value>
    comment_rows(db,filter,values):Result<Vec<Comment>> > sql_text
    pending_mail(conversation,role):Result<Vec<Mail>> > db,Mail,sql_text
    pending_mail_conversations():Result<Vec<String>> > db
    pending_mail_targets(conversation):Result<Vec<MailTarget>> ×0 > db,MailTarget
    approved_writer_results(project):Result<Vec<(String,String)>> > db
    add_mail(conversation,role,mail):Result<Mail> > add_mail_from
    add_mail_from(conversation,role,mail,source_turn):Result<Mail> > db,append_mail_event
    add_comment(conversation,comment,text):Result<Comment> > add_comment_at
    add_comment_at(conversation,comment,round,run,text):Result<Comment> > add_comment_at_with_operation
    add_comment_at_with_operation(conversation,operation_id,comment,round,run,text):Result<Comment> > db
    add_draft_comment(conversation,comment):Result<Comment> > db,insert_draft_comment
    send_draft_comments(conversation,plan_id):Result<Vec<Comment>> > db,draft_{comments,message},now_ms,insert_draft_message,open_draft_comments
    set_comment_status(conversation,id,status):Result<Comment> > db,update_comment_status,load_comment,ensure_comment_was_sent,reopen_comment,append_comment_event
    retain_work(tx,source,target,generation,reason):Result<()>
    retain_work_from_ids(tx,source,source_generation,target,generation,reason):Result<()> > flatten
    retained_result_ref(task):Result<Option<String>> > flatten,db
    dependency_plan(conversation,role,turn):Result<String> > db
    audit_dependency_scope(conversation,role,turn):Result<(Vec<Task>,i64)> > run_tasks
    dependency_reports(task):Result<DependencyReports> ~59 > completed_task_result,as_str,worker_report_references,DependencyReports
    referenced_worker_report(reference):Result<Value> > db
    record_evidence(turn,label,text):Result<EvidenceReference> ~51 > db,EvidenceReference
    evidence(conversation,id):Result<EvidenceReference> > db
    evidence_text(evidence):Result<String>
    task_evidence_context(task,generation):Result<Vec<EvidenceReference>> > db
    result_evidence(turn):Result<Vec<EvidenceReference>> > db
    plan_page_rows(conversation,offset,limit):Result<(usize,Vec<String>,Option<String>)> > db,plan_{count,ids},active_plan
    load_plan_histories(conversation,plan_ids):Result<Vec<PlanHistory>> > plan_history
    turn_final_answer(conversation,turn):Result<Option<String>> ×0 > db,hydrate_event_payload
    hydrate_event_payload(kind,encoded):Result<Value> > as_str
    session_content(conversation,event,field,offset):Result<crate::lean::content::Chunk> > db,chunk
    task_in_plan_by_id({plan,task}_id):Result<Option<Task>> > task_by_id
    task_by_id(task_id):Result<Option<Task>> > db,decode_task_row
    plan_belongs_to_conversation(conversation,plan_id):Result<bool> > db
    plan_conversation(plan_id):Result<String> > db
    task_in_plan({plan,task}_id):Result<bool> > db
    comments_all(conversation,status):Result<Vec<Comment>> > comment_rows,db
    comments_by_plan(conversation,plan_id,status):Result<Vec<Comment>> > comment_rows,db
    plans_page(conversation,offset,limit):Result<PlansPage> > plan_page_rows,load_plan_histories,PlansPage
    encode_event_payload(kind,payload):Result<Value> > is_agent_event,encode_session_fields
    resolve_event_plan(db,turn,plan_id,payload):Result<Option<String>> > infer_event_plan
    pending_mail_plans():Result<Vec<(String,String)>> ×0 > db
    mail(conversation):Result<Vec<Mail>> > db,Mail,sql_text
    mark_delivered(conversation,ids,turn):Result<()> > db,now_ms
    sessions(conversation):Result<Vec<SessionInfo>> > conversation,SessionInfo
    latest_event_seq(conversation):Result<u64> > db
    event_bounds(conversation):Result<(u64,u64,usize)> > db
    history_page(conversation,after,limit):Result<HistoryPage> > conversation,event_bounds,events_since_budget,HistoryPage
    publish_config_with_event(path,publish):Result<()> > db
    append_event(conversation,turn,plan_id,kind,payload):Result<u64> > encode_event_payload,db,resolve_event_plan
    events_since(conversation,after,limit):Result<Vec<Event>> > events_since_budget
    latest_audit_event(conversation):Result<Option<Event>> > db,decode_event_row
    events_since_budget(conversation,after,limit,budget):Result<Vec<Event>> > db,collect_events
    messages_after(conversation,after):Result<Vec<Message>> > db,Message,sql_text
    recent_messages_through(conversation,through):Result<Vec<Message>> > db,Message,sql_text
    messages_by(conversation,author,after):Result<Vec<Message>> > db,Message,sql_text
    count_since_user(conversation,author):Result<usize> > db
    count_since_requirements_reset(conversation,author):Result<usize> > db
    start_turn(conversation,role,provider,upto_message,plan_id):Result<Turn> > db
    set_session(turn,session):Result<()> > db
    set_turn_pid(turn,pid):Result<()> > db
    running_processes():Result<Vec<(u32,i64,String)>> > db
    finish_turn(turn,status,error):Result<()> > db
    last_turn(conversation,role):Result<Option<Turn>> > last_turn_for_roles
    last_orchestration_turn(conversation):Result<Option<Turn>> > last_turn_for_roles
    idempotent(conversation,operation_id,kind,run):Result<Value> > run
    plan_history(conversation,plan_id):Result<PlanHistory> > conversation,plan,plan_history_{timing,status,sessions,outcomes,run},PlanHistory
    previous_plan_rejection(conversation):Result<Option<crate::lean::tasks::Verdict>> > db
    set_check(task,round,check,state,status,note):Result<bool> > {previous,merge}_check,db
    update_task(task):Result<()> > StoredTaskContent.encode,task_snapshot,save_task_round
    dependency_inputs(plan,spec):Result<Vec<crate::lean::tasks::DependencyReport>> > {direct_dependency,earlier_check,selected_approach}_reports
    last_plan_that_ran(conversation,except):Result<Option<Vec<Task>>> ×0 > plan
    last_plan_with_approvals(conversation,except):Result<Vec<Task>> ×0 > plan
    approved_task_history(conversation):Result<Vec<Task>> > task_by_id
    plan_specs(plan):Result<Vec<Value>> > db
    task_summary(task):Result<String> > completed_task_output,as_str
    completed_task_output(task):Result<Value> > completed_task_result
    completed_task_result(task):Result<(String,Value)> > plan_conversation,retained_result_ref,task_by_id
    plan(plan_id):Result<Vec<Task>> > db,plan_task_rows,decode_plan_task
    checks(task,round):Result<Vec<Value>> > db
    reserve_revision_job(conversation,key,revision):Result<(String,bool)> > db,resume_revision_job,require_no_review,insert_revision_job
    revision_job(id):Result<Value> > db
    finish_revision_job(id,result):Result<()> > db
    rejected_revisions_in_a_row(plan):Result<usize> > db
    interrupt_revision_jobs():Result<()> > db,now_ms
    validate_session_identity(tx,turn,session):Result<()>
    update_session_identity(tx,turn,session):Result<()>
    last_turn_for_roles(conversation,roles):Result<Option<Turn>> > db
    mark_tasks_interrupted(tx):Result<()> > unfinished_task_rows,mark_task_interrupted
    last_answered(conversation,role):Result<u64> > db
    last_orchestration_answered(conversation):Result<u64> > last_answered
    forget_orchestrator_sessions(conversation):Result<()> > db
    forget_task_sessions(task):Result<()> > db
    session_last_used_ms(conversation,session,current):Result<Option<i64>> > db
    last_session(conversation,role,provider):Result<Option<String>> > db
    interrupt_running_turns():Result<Vec<String>> > db,running_orchestrators,record_turn_interruptions,close_open_approvals
    interrupt_unfinished_tasks():Result<Vec<(String,String)>> > db,unfinished_plans,mark_tasks_interrupted
    requeue_plan(conversation,plan_id):Result<usize> > refresh_answer_waits
    requeue_after_restart(conversation,plan_id):Result<usize> > db
    accepted_plan(conversation):Result<Option<String>> > db
    plan_review_approved(conversation,plan_id):Result<bool> > db
    queue_plan(conversation,plan_id):Result<usize> > db,proposed_task_count,carry_approved,record_plan_activation,advance_workflow
    set_plan_status(plan_id,from,to):Result<usize> > db
    add_plan(conversation,specs):Result<Vec<Task>> > db,now_ms,insert_proposed_tasks
    latest_plan(conversation):Result<Vec<Task>> > plan
    open(path):Result<Self> > validate_database
    check_runtime_repository(path,identity):Result<()> > validate_database
    initialize(connection):Result<()> > initialize_{base_tables,plan_tables}
    decode_content(text):Result<String>
    db():std::sync::MutexGuard<'_,Connection>
    record_project_identity(project,identity):Result<()> > db
    repository_id(project):Result<Option<String>> > db
    create_conversation(project):Result<Conversation> > Conversation,now_ms,db,begin_interview
    conversation(id):Result<Conversation> > db,Conversation
    conversations():Result<Vec<Conversation>> > db,Conversation
    conversation_goal(conversation):Result<String> > db,sql_text
    delete_session(conversation):Result<()> ~133 > db,remove_unreferenced
    add_message(conversation,author,text):Result<Message> > add_message_with_content_lock
    add_message_with_content_lock(conversation,author,text):Result<Message> > db,insert_message,maybe_start_requirements_checkpoint,record_scope_confirmation_answer,Message
    task_progress(conversation,task):Result<Option<Value>> > db
    record_task_progress(conversation,task,run):Result<()> > append_event
    confirm_scope_for_test(conversation) > events_since,add_message
    queue_requirements_format_repair(conversation):Result<bool> > db
    workflow(conversation):Result<Workflow> > db,Workflow
    ask_user_decision(conversation,question):Result<()> > db
    write_user_decision(tx,conversation,question):Result<()>
    open_user_decision(conversation):Result<Option<Option<crate::lean::interview::Question>>> > messages_by
    awaiting_first_plan(conversation):Result<bool> > db
    ask_interview_question(conversation,question):Result<()> > db,require
    interview_summary(conversation,summary,upto):Result<()> > require,db
    finish_interview(conversation):Result<bool> ~40 > db,current_interview,scope_is_confirmed,promote_current_interview,queue_planning_message,{latest,request}_scope_confirmation
    record_scope_confirmation_answer(tx,conversation,text,seq):Result<()> ~41 > latest_scope_confirmation,current_interview
    - round_reference,{decode,current,unavailable}_round_record,load_round_messages,read_run_context,task_input_generation,
      decode_content_json,resolve_turn_plan,insert_turn,dependency_report,summarize_earlier_checks,option_task_ids,
      selected_approach_report,decode_optional_{text,json}
   Task(C{input_generation,id,plan_id,spec,status,round,report,verdict,base,result,feedback,freshness_token,checks})
   TaskReportInfo(C{task_{id,label},round,report})
   TaskVerdictInfo(C{task_{id,label},round,verdict})
   Turn(C{id,role,status,session_id,error,upto_message,plan_id})
   - default_mail_state×0
  task_failure
   append_integration_report_entry(task,category,text) > append_report
   append_report_entry(task,category,text) > append_report
   - append_failed_turn
  tasks
   agents/gates
    GatePlan(C{prerequisites,deferred,prior_gates,refactoring,ids,{requirement,criterion}_ids,ordered})
     new(tasks,ordered):Self > upstream_tasks,{writer,union}_ids
     next():Option<TaskContract> > remove,reserve_task_id,dependencies,gate_{description,task}
    - GateDescription,gate_summaries,is_audit,gate_goal
   agents.rs
    missing_gates(tasks,gates):Vec<&'static PredefinedAgent> > is_gate_kind
    missing_quality_agents(tasks):Vec<&'static PredefinedAgent> > missing_gates
    refresh_existing_options(tasks):Vec<TaskContract> > options_{task,goal}
    resolve_gate_dependencies(tasks) > expand_gate_dependencies
    with_options(tasks):Vec<TaskContract> > options_{task,goal}
    with_required_agents(tasks):Vec<TaskContract> > with_team_gates
    with_team_gates(tasks,team_gates):Vec<TaskContract> > resolve_gate_dependencies,missing_gates,refresh_existing_options,GatePlan.new,order_graph
    without_options(tasks):Vec<TaskContract>
    PredefinedAgent(C{id,kind,title,focus,required_for_code_changes})
    = PREDEFINED_AGENT_POOL,GATE_KINDS
   artifact_outputs
    validate_artifact_output(output):Result<()> ~58 > all,json_report_path
   bounds
    bound(text,limit):String > bound_cut
    bound_cut(text,limit):(String,bool) > grapheme_prefix_end,overflow_cluster
    = EFFECT_LIMIT=240,DETAIL_LIMIT=600,FILE_LIMIT=20,UNKNOWN_ID_LIMIT=8,UNKNOWN_ID_BYTES=32,CUT_MARK='…'
   effect
    effect(stat,files):(String,bool) > change_summary,effect_candidate,bound_cut
    effect_keys(effect):(Value,Value)
    report_effect(report):(String,bool) > bound_cut
   feedback
    counted_status(entry):&'static str > as_str
    coverage(verdict,obligations):Value > obligation_counts,unknown_ids,bound,coverage_value
    feedback_is_current(feedback,round):bool
    recovery_outcome(action):Option<&'static str>
    resets_card(action):bool
    Outcome(E{Committed,Applied,LeftOnBranch,NoChanges,Conflict,UserEdits,NotApplicable,Cancelled})
     name():&'static str
     label():&'static str
     named(name):Option<Self>
    = OUTCOMES,INTEGRATE_WITH_CONSENT="integrate_with_consent",KEEP_ON_BRANCH="keep_on_branch",
      REVIEW_MANUAL_SNAPSHOT="review_manual_snapshot",RETRY_WITH_GUIDANCE="retry_with_guidance",
      RESTART_AGENT="restart_agent",CANCEL_TASK="cancel_task",UNBLOCK="unblock"
   graph
    depends_transitively(tasks,from,target):bool
    parallel_peers(tasks):std::collections::HashMap<String,Vec<String>> > PeerNode,peers_of
    peers_of(nodes):std::collections::HashMap<String,Vec<String>> > peer_index,dependency_reachability,peer_ids
    render_plan_document(markdown,tasks):String > parallel_peers,plan_document_start,append_plan_task
    work_kind_name(kind):&'static str
    PeerNode(C{id,depends_on})
    - reachable_nodes,peers_can_run,has_section_headers,append_task_{heading,details,relationships},{dependency,peer}_text
   messages
    inbox_text(mail):String > inbox_text_from
    inbox_text_from(mail):String
   plan
    order_graph(remaining):Result<Vec<TaskContract>> > graph_failure,validate
    planned_graph(tasks):Result<Vec<TaskContract>> > planned_graph_with
    planned_graph_with(tasks,kept):Result<Vec<TaskContract>> > resolve_gate_dependencies,order_authored_graph
    validate(tasks):Result<()> > validate_{task_id,change,dependencies},as_str
    validate_task_id(id):Result<()> > all
    PlannedTask(C{id,title,goal,kind,depends_on,deferred,change_kind,outputs,delivery,covers,section,summary})
     into_contract():TaskContract > TaskContract,ReviewerSpec,reviewer_summary
    - known_authored_dependency
   prompts
    instructions
     orchestrator_instructions():String > kind_names,agent_descriptions
     planner_instructions():String > kind_names,agent_descriptions
     = FILESYSTEM_GUIDANCE
    plan_review
     audit_followup(request,results):String
     audit_prompt(project,request,results):String
     plan_rejected(findings):String
     plan_review_followup(request,plan):String
     plan_review_prompt(project,request,plan):String
     plan_revision_review_prompt(project,request,plan,baseline,approved,reason,initial):String > plan_review_{prompt,followup}
     ApprovedWork(C{kept,redone})
     - plan_{text,entry},listed
    requirements
     findings_key(findings):String > first_sentence
     findings_text(findings):String > as_str
     round_handoff_prompt(handoff,reviewer):String > handoff_sections
     = REVIEW_FORMAT,PROJECT_RULES,OUTSIDE,REVIEW_RULES,COVERAGE_FORMAT
     - requirements_note,handoff_protection,latest_report,handoff_tail
    review
     code_review_prompt(dir,task,base,result,comments):String > review_{range,comments}
     reviewer_followup(task,report,changes):String > reviewer_binding
     reviewer_prompt(project,task,report,changes):String > reviewer_task_context,obligations_text,obligations,reviewer_binding,risks_note
     reviewer_turn_prompt(project,task,report,changes,fresh,upstream,earlier):String > reviewer_{prompt,followup},earlier_findings,report_recheck
     - change_note,format_earlier_finding
    worker
     changes_text(base,result,stat):String
     dependency_metadata(text):String
     dependency_reports_note(reports):String > as_str,upstream_note
     needs_user_action(error):bool
     upstream_note(done):String > dependency_{marker,header}
     worker_followup(findings):String
     worker_prompt(project,task):String > worker_prompt_parts,has_delivery
     worker_resume(findings):String
     worker_turn_prompt(project,task,continuation,upstream):String > worker_prompt,change_kind
     DependencyReport(C{id,title,report})
     - worker_{prompt_opening,prompt_closing,checks,change_rules},has_options_prerequisite
   validation
    check_comments_text(comments):String
    obligations(task):Vec<(String,String)> > for_kind
    obligations_text(obligations):String
    risks_note(task):String
    settle(verdict,obligations,read_only) > unaccounted,append_{missing_finding,unsettled_findings}
    unaccounted(verdict,obligations):Vec<&'a (String,String)> > counted_status
    CheckUpdate(C{id,status,note})
    = GOAL_CHECK="goal"
    - {append_unsettled,has_severe_check}_finding
  tasks.rs
   ChangeKind(E{None,Artifact,Code})
    has_delivery():bool
   DeliveryMode(E{Untracked})
   ReviewerSpec(C{summary})
   TaskContract(C{quality_gate,options_for,id,{requirement,criterion}_ids,title,goal,summary,section,deferred,change_kind,outputs,delivery,depends_on,kind,covers,reviewer})
    require_not_deferred():Result<()>
    change_kind():ChangeKind
    changes_code():bool > change_kind
    validate_change_paths(paths):Result<()> > change_kind
   Verdict(C{approved,findings,coverage,resolved})
   = STALE="stale",PROPOSED="proposed",QUEUED="queued",WORKING="working",REVIEWING="reviewing",APPROVED="approved",
     COORDINATING="coordinating",NEEDS_USER="needs_user",FAILED="failed",SKIPPED="skipped",CANCELLED="cancelled",
     REVISED="revised",WAITING="waiting"
  terminal
   controls
    Filter(C{mode,sequence,osc})
     feed(bytes):Vec<u8> > feed_byte
     - finish_string,ground,escape,begin_string,csi,string,append_osc,string_escape
   emulator/cluster
    written_position(term,text,width,before,after):Option<(usize,i64)> > get_cell
    ClusterContinuation(E{Updated,Write})
    Tail(C{text,position,width})
   emulator.rs
    GraphemeStream(C{mode,utf8,tail,dimensions})
     continue_cluster(term,ch):Result<ClusterContinuation> > require,reprint_cluster
     feed(term,bytes):Result<()> > sync_dimensions,feed_next
     - replace_cluster,relocate_tail,advance_control_sequence,feed_{control,ascii},join_ascii_prefix,write_ascii_suffix,
       consume_utf8,character,write_character
   session
    handles
     quiet_for(state):std::time::Duration
     tail(state,count):String > screen_text
     Session: attached():bool; submit(state,text):Result<()> > require; interrupt(state):Result<()>; press(state,keys):Result<()> > require; press_for_approval(state,provider,fingerprint,key):Result<bool> > with_matching_fingerprint,screen_text,detect,require; shared():Arc<Mutex<State>>; exited():Option<String> > flatten
     - Session: attach,resize
    lifecycle
     Session: snapshot(handle,after):Result<Option<Screen>> > render_screen; input(handle,sequence,input):Result<()> ~42 > require,apply_input
     - render_{cells,cell},resolve_color,cell_modifiers,apply_{paste,key},terminal_key,apply_mouse,mouse_event,modifiers,
       mouse_button
     - Session: drop
    spawn
     Session: spawn(command,core,provider,columns,rows):Result<Self> > size,spawn_child,new_state
     - PtyChild,process_marker,register_child,start_{writer,reader_thread},read_{terminal,chunk},{update,disconnect}_terminal
  terminal.rs
   Cell(C{x,y,text,width,foreground,background,modifiers})
   Input(E{Key,Paste,Mouse,Scroll})
   Screen(C{handle,revision,columns,rows,cursor,cells,error})
   Session(C{master,child,state,handles,registration,marker})
   State(C{term,revision,error,output})
   - Config,Handle,Writer,pty_size
   - Config: scrollback_size×0,color_palette×0,unicode_version×0
   - Writer: write,flush
  tui
   app
    commands/recovery
     - unblock_result
    compose/send
     comment_text(editor,view):String > build_lines
     is_rejection_target(target):bool
     AgentMessageSend(C{client,updates,target,operation,text})
      spawn() > send
     AgentMessageTarget(C{conversation,recipient,addressed,plan,expected_{run,round},delivery})
     CommentSend(C{client,updates,conversation,operation_id,plan,generation,request_id,editor,round,run,text})
      spawn() > complete
      - execute,execute_{unblock,rejection,comment}
    conversation/poller
     Poller(C{client,updates,conversation,generation,stop})
      spawn() > run
      - poll_once,{make,event}_update
    picker
     - conversation_title
    pointer/header
     token_at(text,target_col):Option<String>
    projects
     - {project,remembered_project}_item
    requests/fetch
     - {Task,Run}Fetch,mark_run_fetch,task_fetch_update
   approval
    approval_panel_height(summary,width,options,max_summary_lines,chrome_rows,available):u16 > card_summary_rows
    card_summary_rows(text,width,max_lines):usize > len,bounded_summary
    ApprovalCard(C{approval})
     new(approval):Self
     key(key):Option<Outcome>
     draw(frame,area) ~92 > Theme.from_env,bounded_clean,truncate,style,len,bounded_summary,title,borders
    Outcome(E{Later,Choose})
    super::App: sync_approval() > ApprovalCard.new; route_approval(key):Option<bool>; set_approval_mode(mode); show_pending_approvals()
   buffer
    InputBuffer(C{text,cursor,preferred_col})
     edit_key(key) > insert_char,delete_{word_back,to_start,to_end},move_{home,end},backspace,delete,move_{word_left,word_right,left,right,buffer_start,buffer_end}
     new():Self
     with_text(text):Self
     clear()
     is_empty():bool
     len():usize
     clamp_cursor() > len
     insert_char(ch) > insert_str
     insert_str(s) > clamp_cursor
     backspace():bool > clamp_cursor
     delete():bool > clamp_cursor
     delete_word_back() > clamp_cursor,all
     delete_to_start() > clamp_cursor
     delete_to_end() > clamp_cursor
     move_up(width) > {clamp,visual}_cursor
     move_down(width) > {clamp,visual}_cursor
     visual_cursor(width):(u16,usize) > {compute_visual,visual_cursor_from}_lines
     visual_cursor_from_lines(lines):(u16,usize) > len
     visual_line_count(width):usize > len,compute_visual_lines
     on_first_row(width):bool > visual_cursor
     on_last_row(width):bool > visual_{cursor,line_count}
     move_left() > clamp_cursor
     move_right() > clamp_cursor
     move_word_left() > clamp_cursor,skip_trailing_whitespace,trailing_word_start
     move_word_right() > clamp_cursor,all
     move_home() > clamp_cursor
     move_end() > clamp_cursor
     move_buffer_start()
     move_buffer_end() > len
     - find_byte_for_col,cursor_{on_line,column},inside_grapheme
   clipboard
    clipboard_sequence(text):String
    copy_text(writer,text):io::Result<CopyMethod> > copy_with_fallback
    CopyMethod(E{Osc52,Fallback})
    = MAX_CLIPBOARD_BYTES
    - BrokenWriter,validate_size,write_osc52,native_copy,feed_stdin,drain_stderr,read_diagnostic,wait_for_exit,finish_copy
    - BrokenWriter: write,flush
   config
    app
     - config_reply
    draft
     - {locate,write}_field
    model
     kind_names():Result<Vec<String>,String>
     toml_value(value):Result<toml_edit::Value,String>
     ConfigEditor(C{fields,selected,path,status,pending,visible,conflict,reload_requested,base_source,baseline,adding})
      source():Option<String> > {parse,validate,emit}_draft
      paste(text) > insert_str,sanitize_single_line_paste
      key(key) > {addition,structural}_key,edit_selected
      new(source,path):crate::Result<Self>
      flatten(path,value):Result<(),String> > flatten_table,push_field
      inputs():BTreeMap<Vec<String>,String>
      is_dirty():bool > inputs
      from_reply(value):Result<Self,String>
      refresh(fresh) > is_dirty
      applied(source) > inputs
      parse_draft():Option<Value> > len
      validate_draft(value):bool > {validation,mark_validation}_error
      draw(frame) > draw_with_theme,Theme.from_env,config_dir
      draw_with_theme(frame,theme,global) > field_lines,{selected,status}_line,cursor_column
      - remove_missing_entries,patch_field,show_addition,first_entry_name,add_kind,begin_addition,addition_section,
        offer_addition,add_profile,{append,delete_selected}_entry,edit_field,removes_orchestration,visible_fields,field_line
     Field(C{path,input,error,choices})
      belongs_to_entry(section,name):bool
      value():Result<Value,String> > as_str
     - KindNameError,Reply,strings,field_text
     - KindNameError: custom,unknown_variant×0
   contracts
    ConversationState(C{input,editor,sending,pending_send,send_request_id,failed_send,{sending,pending}_comment,comment_request_id,failed_comment,comment_{drafts,draft_cursors},selected,relations,offset,details,diff,{plan,ledger}_view,scroll,{task,diff}_request_id,focus,graph_{scope,visibility_exception,collapsed,grouping},details_{scroll_memory,query_memory},graph_viewport})
     capture(app):Self
     restore(app) > restore_{composer,navigation,overlays,requests}
     - capture_{composer,navigation,overlays,requests}
    Editor(C{target,input})
    Focus(E{Chat,Graph})
    Update(E{NativeInputRejected,PlanHistory,InterviewResult,AgentMessage,Ledger,Config,Events,Conversation,Notice,ProjectReady,ProjectStatus,Diff,Task,TaskFailed,{Send,Comment}Result})
   dashboard
    hero/identity
     - identity_{id,summary,kind}
    sections
     history
      comments_and_agents(this,view,theme,width,keys):Vec<Section> > comments,agents
      - comment_lines,agent_{lines,identity,usage,activity}
     review
      - check_{section,body,rank,lines},pending_line,finding_{section,body,lines,rank}
     scope
      lines(details,view,theme,width,row):Vec<Line<'static>> > dependencies,metadata
      - append_{section,requirements,resolution,parallel}
    sections.rs
     - append_{primary_sections,goal_sections,risk_scope},review_sections,{text,risks}_section
   dashboard.rs
    Fold(E{Preview,Full,Folded})
    Section(C{id,title,meta,body,preview,right})
    = TWO_COLUMNS=118,GAP=3,WHOLE
    - gauge,para,item,{first,continuation,empty}_item_line,chip,join_chips
   details
    checks
     check_rows(stored):Vec<CheckRow> > obligations,make_check_row
     CheckRow(C{id,kind,text,mark,style,state,status,note})
     - {check,live_check}_state,check_mark
    model
     Composer(C{input,sending,operation,error,sent_text,mail,delivery,delivery_{receipts,failure},address,mode})
     DetailRecord(C{lines,search_text})
     Details(C{native,native_focus,resume_round,native_areas,text_area,selected_round,available_rounds,latest_observed,active_panel,split,{dash,panel}_scroll,panel_expanded,documents,panel_layout,fetched_events,fetch_in_flight,task_uuid,conversation,drafts,editing,input_width,key,tab,scroll,task,fetched_{round,generation,plan},comment,check,reveal_check,fetched_checks,fetch_{failures,retry_after,error},{scroll,query}_memory,query,searching,sessions,history_unavailable,follow,show_info,dash_{fold,ids,whole,focus,reveal},pending_g,layout_cache})
      hero_key():String > session_keys
      identity_line(view,theme,key):Line<'static> > row,identity_{status,title}
      provider_chip(view,theme,key):Vec<Vec<Span<'static>>>
      check_chips(theme):Vec<Vec<Span<'static>>> > check_rows
      activity_chips(view,theme):Vec<Vec<Span<'static>>> > obligation_chip,{comment,usage}_chips
      hero(view,theme):Vec<Line<'static>> > hero_key,provider_chip,{check,activity}_chips
      review_sections(view,theme,width,rows,keys):Vec<Section> > check_findings,comments_and_agents
      sections(view,theme,width):Vec<Section> > row,session_keys,SectionContext,{append_task,filter_tab}_sections
      dashboard_visible():bool > overview
      fold_of(id):Fold
      dash_move(delta) > len
      dash_jump(last) > len
      dash_cycle() > fold_of
      dash_expand_all() > all,fold_of
      dash_scroll(delta)
      lines(view):Vec<(Style,String)> > records
      summary(view,theme):Vec<Line<'static>> > hero
      role_keys():Vec<String>
      overview():bool > is_overview
      terminal_visible():bool > overview
      session_keys():Vec<String>
      neighbour_tab(steps):usize > tabs_for
      nth_tab(n):Option<usize> > tabs_for
      switch_tab(next) > tabs_for,{save,restore}_current_tab
      switch_key(new_key) > save_current_tab,tabs_for,restore_current_tab
      switch_role() > save_current_tab,tabs_for,restore_current_tab
      composer_recipient(key):String
      composer_key():(String,String) > session_keys,composer_recipient
      change_round(view,direction) > task_id
      save_current_tab()
      restore_current_tab() > overview
      records(view):Vec<Rc<DetailRecord>> > overview,comments.comments,checks,results
      fetch_failed(error):bool
      load_task(task) > as_str
      new(key):Self > Details,tabs_for
      comments(view):Vec<&'a super::view::Comment> > as_str
      selected_comment(view):Option<u64> > flatten,comments
      selected_check():Option<String> > check_rows
      check_count():usize > len,check_rows
      task_id():Option<&str>
      draw_dashboard(frame,view,area,theme) > dashboard_{geometry,rows},reveal_dashboard_focus,dashboard_areas,paint_{dashboard,dashboard_scrollbar}
      draw(frame,view) > normalize_tab,Theme.from_env,summary,detail_areas,overview,draw_{header,round_header,secondary_screen,detail_content}
      draw_overview(frame,view,body,theme) > session_{keys,areas},draw_session_panel
      - check_summary,token_chip,findings_section,
        draw_{overview_screen,split_overview,single_overview,dashboard_screen,footer_hint,detail_block,document,document_result,document_page,records,native_panel,composer_panel,composer_status,composer_input}
     LayoutCache(C{records,query,width,lines})
      update(records,query,width):&[Line<'static>] > len,all,render_records
    presentation
     details_footer_hint(tab,width):&'static str > is_overview,{overview,checks,comments,content}_hint
     details_footer_hint_for(key,tab,width):String > details_footer_hint,shown
     format_tabs(agent_key,active_tab,max_width):Line<'a> > TabStrip.new
     - TabStrip
     - TabStrip: name,full_width,agent_span,full,narrow,window,extend_{right,left},tab_span
    records
     checks
      checks(details,view,records) > append_checks
      - summary_record,append_not_started,issue_suffix,check_record,comment_line,evidence_lines,label
     comments
      comments(details,view,records) > empty_comments,comment_record
      - comment_{style,header}
     overview
      overview(details,view,records) > conversation_record,append_agent_records
      - conversation_status,{cli,task}_record,task_summary_lines,append_{task_status,list},activity_record,plain,label
     plan
      plan(_details,view,records)
     results
      results(details,view,records) > plan,agent_results,reviewer_result,worker_results
      - append_findings,finding_line,artifact_record,short_hash,report_record,plain,label
     review
      review(details,view,records)
    records.rs
     - record,plain
   details.rs
    is_overview(tab):bool
    tab_name(key,tab):&'static str
    tabs_for(key):Vec<usize>
    = TABS,RESULT=1,CHECKS=2,COMMENTS=3,REVIEW=4,REVIEWER=5,PLAN=6
   diffview
    header
     diff_footer_hint(width):&'static str
     diff_header(task,range,file_count,added,removed,truncated,width):Line<'static> > diff_header_with_reviewer
     diff_header_with_reviewer(task,range,file_count,added,removed,truncated,reviewer,width):Line<'static> > render_diff_header,HeaderSummary
     - {range,count}_detail,truncation,reviewer_label
    render
     files
      draw_files(frame,area,files,selected,focused,compact) > title,borders,file_rows
      - file_{row,marker}
     lines
      diff_rows(input):(Vec<Line<'static>>,usize) > diff_file_prefix,append_diff_lines
      file_comments(view,target):Vec<&'a Comment>
      scroll_to_cursor(scroll,cursor,row_count,height):usize
      DiffRows(C{file,comments,task,file_{index,count},compact,selected_line,on_files,width})
      - file_title,append_comments,comment_{line,target},diff_line,wrap_graphemes,diff_{style,text,number}
    render.rs
     - draw_placeholder,render_{diff_error,body_message}
   diffview.rs
    DiffView(C{task,files,range,truncated,file,line,on_files,files_hidden,scroll,reviewer})
     draw(frame,view) > paint_at
     paint_at(frame,area,view) > draw_{header_footer,body}
     new(task):Self > DiffView
     load(reply) > as_str,short,len,files,lines_in_file
     target():Option<String> > files,as_str
     half_page(down) > len,files,lines_in_file
     key(code) > move_{file,selection}
     - draw_{loaded_body,lines}
   document
    builder
     private_file(path):Result<File>
     Builder(C{files,text,ranges,offset})
      new(state):Result<Self> > private_directory,Files,private_file
      append(header,source):Result<()> > source
      finish():Result<Document> > Document
     Document(C{files,ranges,owner,alive,layouts}) : PartialEq
      viewport(width,height,scroll,follow):Result<Option<Viewport>> > viewport_filtered
      viewport_filtered(width,height,scroll,follow,query):Result<Option<Viewport>> > LayoutKey,resolve_index,read_viewport
      - eq×0,{schedule,finish,build}_index
     Files(C) : Drop
      - drop
     LayoutIndex(C{path,lines,text}) : Drop
      - drop
     LayoutKey(C{width,query})
     LayoutState(E{Building,Ready,Failed})
     Viewport(C{lines,total,scroll})
    chunk
     read_utf8_chunk(reader,remaining):Result<String>
    layout
     - make_room,read_{rows,row}
    search
     range_matches(path,start,end,query,alive):Result<bool> > prefix_table,{read_utf8,scan}_chunk
    wrap
     wrap_range(path,output,start,end,width,dim,alive):Result<usize> > WrapState.new,LineIndex,consume_file,emit
     - WrapState,Cursor,ready_length
     - Cursor: start
     - LineIndex: consume,wrap_grapheme
     - WrapState: consume_ready
   document.rs
    = CHUNK,INDEX_BYTES=17
   events
    graph
     - visible_graph_key
    keyboard
     - embedded_diff_graph_key
   graph
    cache
     CachedGraph(C{generation,revision,counts,idle,viewport,expanded,expanded_tab,step,expires,graph})
     LayoutCache(C{entry})
      get(view,generation,scope,collapsed,grouping,viewport):std::rc::Rc<Graph> > get_with_expanded,LayoutIdentity
      get_with_expanded(view,scope,collapsed,grouping,viewport,identity):std::rc::Rc<Graph> > CacheRequest,matches,rebuild
     LayoutIdentity(C{generation,expanded,tab,step})
     - counts
    canvas
     Canvas(C{origin_{x,y},width,height,cells})
      new(width,height):Self
      new_window(origin_{x,y},width,height):Self > Canvas
      cell(x,y):Option<&mut Cell>
      put(x,y,ch,style) > cell
      text(x,y,text,style) > put
      path(points,style) > segment
      glyph(cell):&str
      show(buffer,area,left,top) > get_cell
      lines():Vec<String>
      - horizontal,vertical,{row,column}_visible
     Cell(C{lines,ch,style})
    card_content
     height(view,key,width,limit,folded):u16 > Content.new
     pair_finished(task,role):bool
     reviewer_text(view,key):Option<String> > task_card,pair_finished
     Content(C{rows})
      new(view,key,width,limit,folded):Self ~55 > reviewer_text,branch,activity_line,card_meta,effect_lines,is_admin,card_lines
      height():u16 > len
     Tone(E{Text,Dim,Status,Activity})
    filters
     Relations(C{depends,reviews,mail,supervises}) : Default
      all():Self
      only(index):Self
      toggle(index) > flag
      shown(index):bool
      name():String > shown
      shows(edge,selected):bool
      - default
     Scope(C{query,states,providers,visibility_exception})
      is_filtered():bool
     = RELATIONS
    health/cards
     activity_line(view,key):Option<String> > task_{card,activity}
     card_lines(view,key,width,max):Vec<String> > card_text,node_name,fit_lines
     effect_lines(view,key,width,max):Vec<String> > flatten,fit_lines
     fit_lines(text,width,max):Vec<String>
     - task_{card_text,fallback}
    health.rs
     describe(view,key):String > task_card,{worker,reviewer}_status,describe_agent
     health(view,key):Health > task_card,{worker,reviewer,task,admin}_health
     mark(status):(&'static str,Color)
     provider_of(view,key):Option<&'a str> > task_card,card_members,current_round
     providers(view):Vec<String>
     providers_of(view,key):Vec<&'a str> > provider_of,as_str
     Health(E{Idle,Working,Waiting,Problem,Done})
      name():&'static str
     = ALL
     - agent_flags,task_{problem,running},{activity,requirements}_label
    layout
     builder
      Builder(C{view,graph,size,index,admin,extra_agents,viewport,step,lanes})
       new(view,scope,collapsed,grouping,viewport,expanded,step):Self > animated,Size.fit,empty_graph,task_index,{extra,admin}_agents
       build():Graph > place,add_edges,set_graph_bounds
       matches_query(key):bool > scope_matches
      - task_{matches,fields_match},query_matches
     edges
      add_edges(view,graph,extra_agents) > {add_role,add_mail}_edges,as_str
      - aggregate_mail,push_edge
     units
      build
       card_height(builder,key,folded):u16 > height,width,layout_height
       layers(builder,units):Vec<u16> > {status,dependency}_layers
       measure(builder,units,rows):Vec<Placed> > unit_width,card_height,Placed
       member_keys(view,unit):Vec<String> > is_pair
       rows(builder,units,layers):Vec<Vec<usize>> > ordered_rows,isolate_expanded_unit
       shown_members(builder,unit):Vec<(String,bool)> > member_keys
       unit_width(builder,unit):u16 > shown_members,width
       units(builder):Vec<Unit> > options_of,unit_skeleton,populate_units
       - member_tasks,folded,append_layer_rows,sort_by_parent_position,wrap_layer,push_wrapped_unit
      links
       buses(links,placed):Vec<Bus> ~56 > spots,Bus,Lean.new,is_short,pack,Lane
       crossed(buses,row):bool
       gap_height(lines,tight):u16
       gap_lines(links,buses,placed,rows,entries):Vec<Vec<usize>> > spots,is_short
       is_short(link,spot):bool
       lane_counts(buses):(u16,u16)
       lane_reserve(buses):u16 > lane_counts
       links(units,placed):Vec<Link> > spots,implied,model.rs:Link
       place_lanes(buses,placed) > lane_counts,crossed
       spots(placed):HashMap<usize,&Placed>
       - Lean
       - Lean: offset
      model
       Bus(C{to,lane})
       Lane(C{side,index,span,x})
        crosses(row):bool
       Link(C{from,to})
       Placed(C{unit,row,x,width,height})
       Side(E{Left,Right})
       Tops(C{admin,units,gaps,bottom})
       Unit(C{task,options,parents,visible,folded})
      placement
       admin_rows(builder):Vec<Vec<&'a str>> > width,is_final_review
       admin_width(size,row):u16 > width
       center_rows(placed,width)
       row_tops(builder,admin,placed,gaps,rows):Tops > is_final_review,admin_row_tops,row_heights,content_tops,Tops
      render/link_render
       entry_columns(builder,units,placed,buses):HashMap<usize,u16> > spots,title_end
       push_links(builder,units,placed,wiring) ~40 > spots,exit_columns,gap_line,is_short,Edge
       Wiring(C{links,buses,gaps,tops,entries})
      render.rs
       push_admin(builder,rows,tops,width) > admin_width,Node,height,layout_height
       push_hidden(builder,units,placed) > member_keys,Node,width
       push_unit(builder,unit,spot,top) > Frame,push_cards
       - push_{card,internal_edge}
     units.rs
      place(builder) > units,settle,admin_rows,layout_width,center_rows,place_lanes,entry_columns,gap_lines,row_tops,push_{admin,unit,hidden},Wiring,push_links
      - Arrangement,arrange,resize,fit_expanded_card,expanded_row
    model
     Node(C{key,x,y,width,height,visible,group,collapsed}) : Default
      new(key,x,y):Self
      mid_y():u16
      right():u16
      - default
     Size(C{w,h,admin_w,expanded_{w,h,key},full_{w,h},step})
      fit(viewport,expanded_key):Self
      animated(step):Self > grow
      reserve_columns(viewport,columns) > grow
      limit_expanded_width(width) > grow
      width(key,admin):u16
      layout_height(key,_admin):u16
     = FULL_STEP=16
    nodes
     card
      body
       - nonempty
      content
       card_title(name,inner,collapsed):String
       status_style(theme,health):(String,Style) > status,glyphs,Theme.error,warning,Theme.dim
     card.rs
      draw(view,canvas,node,selected,expanded_tab,theme,tick) > outside_viewport,paint,Card.new
      - Card
      - Card: paint_tabs,status_description,pair_finished,paint_{contents,background,progress}
    nodes.rs
     draw_node(view,canvas,node,selected,expanded_tab,theme,tick) > draw
    plan
     PlanGraph(C{nodes,width,height})
      new(tasks,viewport_width):Self ~80 > as_str
      render(tasks,canvas,selected,theme) > as_str,accent,Theme.dim,draw_card
   graph.rs
    agent_heading(view,key):String
    card_key(view,key):String > requirements_round,task_card
    expanded_card_areas(viewport,(i32,i32),(u16,u16)):[Rect; 3] > clip
    expanded_tab_names(view,key):&'static [&'static str] > has_delivery
    Frame(C{key,task,x,y,width,height,visible})
    Graph(C{nodes,frames,edges,width,height,scope,collapsed,grouping,expanded,expanded_tab})
     new(view):Self
     new_with_options(view,filter_query,collapsed,grouping):Self > Scope
     new_with_viewport(view,scope,collapsed,grouping,viewport):Self
     new_with_expanded(view,scope,collapsed,grouping,viewport,expanded,expanded_tab):Self
     new_animated(view,scope,collapsed,grouping,viewport,(Option<String>,usize),step):Self
     render(view,canvas,relations,selected) > render_with,Theme.new
     render_with(view,canvas,relations,selected,theme,tick) > as_str,card_key,render_{frames,edges,nodes}
     node(key):Option<&Node>
     frame(group):Option<&Frame>
     edge_path(edge,nodes):Option<Vec<(u16,u16)>> ~54 > frame,shown,as_str
     neighbour(key,dx,dy):Option<String> > node
     route(from,to):Vec<(u16,u16)>
     edge_routes(relations):Vec<EdgeRoute<'_>> > shows,edge_path
     route_intersects_card(path,node):bool
     - arrow,entry_arrow,draw_frame,draw_frame_{border,title},render_{edge,edge_label},
       route_{same_column,same_row,between_rows}
    GroupingMode(E{Dependency,Status})
     next():Self
     name():&'static str
    = WIDTH=32,HEIGHT=14
    - Kind,Role,workflow_stage
   graph_filters
    FilterMenu(C{cursor,rows})
     key(key,providers,relations,scope):Outcome > rows,apply_key
     click(column,row,providers,relations,scope):Outcome > rows,toggle
     draw(frame,providers,relations,scope) > rows,menu_area,title,borders,draw_rows
    Outcome(E{Stay,Close})
    - Row,checked,append_heading,heading,row_line
   history
    append(path,text):Result<()> > open
    load(path):Result<Vec<String>> > open
    = LIMIT=1000
   input
    - restored_buffer
   inspection
    history
     task_snapshot(client,conversation,plan,id,round,checks_only):Result<Snapshot> > {round,task_id}_snapshot
     - task_request,selected_round,round_documents,result_document,append_{history_notice,round_fields,field_content},
       read_content
    session
     append_answers(client,conversation,turn,builder):Result<bool> > session_page,append_final_events,next_event_cursor
     run_snapshot(client,conversation,role,selected):Result<Snapshot> > session_runs,selected_run_index,session_metadata,selected_run_snapshot
     - append_{final_event,session_content,run_status}
   inspection.rs
    Snapshot(C{task,documents})
   interview
    keyboard
     Form: key(key):Option<String> > key_global,len,navigate_choice,submit_or_insert_newline,edit_key,insert_character
     - Form: scroll_page
    render
     ScopeCheckpoint(C{offset,sanitizer,wrapper})
     Form: draw(frame,screen,theme) ~50 > interview_box,rows,row_count,content_box,visual_cursor,visible_{scroll,rows},ScopeRows,render_interview_rows,place_cursor
     - ScopeWrapState,ScopePass,ScopeWrapper,question_rows,wrapped_{scope,scope_from},record_checkpoint,
       append_{answer_rows,interview_footer,wrapped},option_rows,append_{option,option_label,input_rows}
     - ScopeWrapper: new,from_state,emit_{row,borrowed},consume,finish
   interview.rs
    Form(C{event,question,scope_{summary,revision,source_revision,cache},selected,input,visible,sending,sent,error,scroll,scroll_step,manual_scroll})
     new(event,question):Self
     paste(text) > insert_str,sanitize_paste
     set_scope_summary(summary) > clear_layout
     sync_scope_summary(summary,source_revision) > set_scope_summary
    - ScopeCache,ScopeCacheKey,ScopeWindow
    - ScopeCache: prepare,window
   layout/lines
    compute_visual_lines(text,width):Vec<VisualLineInfo> > paragraph_end,wrap_paragraph,empty_line
    visual_text_lines(text,width):Vec<String> > compute_visual_lines,visual_text_lines_from_layout
    visual_text_lines_from_layout(text,lines):Vec<String>
    VisualLineInfo(C{{start,end}_byte,chars,width})
    - LineBuilder
    - LineBuilder: new,append_{word,grapheme},finish
   ledger
    app/execute
     spawn_ledger_work(client,updates,work,request_id) > execute_work,UpdateSender.send
     LedgerWork(C{conversation,kind,action,request})
     - prepare_change,register_source,empty_run_page
    app.rs
     - route_{navigation,create,apply,reject,record,detail,source},proposal_request,pending_run,capture_prepared_draft
    comparison/matching
     changed_graphemes(before,after):(Vec<bool>,Vec<bool>) > token_offsets,TokenAlignment,GraphemeChanges,mark_common_tokens
     lcs_pairs(a,b,ai,bi,out) > append_lcs_edges,lcs_split
     - lcs_{row,row_step},mark_word_{pairs,pair},mark_token_gap
    comparison.rs
     list_diff_lines(before,after,theme,width):Vec<Line<'static>> > append_{changed_items,added_items}
     panel(theme,title):Block<'a> > title,borders,separator
     readable(value):String > sanitize,readable_object
     text_diff_lines(before,after,theme,width):Vec<Line<'static>> > changed_graphemes,render_versions
     - DiffStyle,DiffLine,same_list_item,append_{text_version,grapheme}
    detail_render/content
     append_coverage(lines,entry,tasks,evidence,theme,width) > title,Theme.dim,append_task,evidence_for,push_indented,evidence_summary
     append_criteria_with_evidence(lines,entry,evidence,theme,width) > evidence_for,approved,title,Theme.dim,append_criterion
     history_entries(history,theme,width):Vec<Line<'static>> > append_history_entry
     overview_lines(entry,theme,width):Vec<Line<'static>> > push_section,qualifiers_text,append_dependencies
     push_section(lines,title,text,theme,width) > title,sanitize
     section_lines(title,text,theme,width):Vec<Line<'static>> > push_section
     - badge_line,history_entry_body
    form
     Form(C{base,revision,fields,field,operation_id})
      new(spec,revision):Self > form_values,with_text
      spec():crate::Result<EntrySpec> > update_{identity_and_text,links,policy}
      - field_{text,ids}
     FormField(E{Interpretation,Qualifiers,Criteria,Parent,Dependencies,Disposition,Reason,Source,Mandatory})
      next(backwards):Self > len
     = ALL
     - criterion_text,parse_{criteria,criterion}
    interaction/edit
     - toggle_enum_field
    model
     Action(E{None,Fetch,Detail,Source,Prepare,Apply,Reject,Create,Runs,Proposal,Review})
     LedgerView(C{visible,page,selected,query,searching,task,missing_only,status,status_edit,task_{filter,filter_edit},tab,scroll,drafts,editing,preview,proposed_change,detail,source,error,pending,request_id,area,{list,detail}_area,list_start,confirm_new_run,notice,workflow_notice,runs,edit_scroll,pending_kind,review_run,approval_event,stale,scope_epoch,pending_{scope_epoch,run,read},rejecting,rejection,rejection_drafts,prepared_draft,run_filters}) : Default
      detail_lines(theme,width):Vec<Line<'static>> > selected_entry,{section,spec,history}_lines
      form_area():Rect
      draw_form(frame,area) > form_area,draw_form_{heading,labels,input,status}
      browse_key(key):Action > toggle_missing,move_record,{change,select}_tab,edit,change_page,new_entry
      edit_key(key,ctrl,conversation):Action > prepare_edit,move_edit_cursor
      pending_action(key):bool
      pending_page_edit(key):bool
      edit_filter_key(key):bool
      finish_filter_key(key):Action > edit_filter_text,commit_task_filter
      preview_key(key,ctrl):Action > {rejection,review_display}_key
      key(key,conversation):Action > confirm_run_key,cancel_pending_read,preview_key,pending_{action,page_edit},{edit,finish}_filter_key,browse_key
      save_rejection_draft() > as_str
      mouse(mouse):Action > {scroll,edit,browse}_mouse
      preview_lines(theme,width):Vec<Line<'static>> > preview_heading,append_{reason,entry,affected_tasks}
      reply(kind,reply) > page_reply,as_str,{preview,rejected,applied,runs,created}_reply
      draw_body(frame,area,theme) > Theme.panel,draw_{header,review,browse}
      draw_detail_pane(frame,area,theme) > draw_{tabs,detail_content}
      draw_new_run_confirmation(frame,area,theme) > style,Theme.text
      draw_header(frame,rows,theme) > style,selected,heading,title,sanitize,filter_label,Theme.dim,draw_workflow_notice
      draw_browse_footer(frame,actions,status_area,theme) > style,status_{label,style}
      draw_record_list(frame,area,theme) > filtered,record_lines
      draw(frame,area) > draw_form,Theme.from_env,draw_{new_run_confirmation,body}
      draw_review(frame,rows,theme) > draw_{rejection,preview},style,selected,Theme.{error,dim},sanitize
      new():Self
      request(conversation):Request > LedgerQuery
      filtered():Vec<&LedgerEntry> > all
      selected_entry():Option<&LedgerEntry> > filtered
      form_mut():Option<&mut Form>
      paste(text) > insert_str,form_mut
      edit() > selected_entry,form.Form.new
      preserve_filter() > RunFilter
      restore_filter() > RunFilter,InputBuffer.new,len
      invalidate_read()
      - refresh_operation_id,{reject,apply}_action,default,{edit,label}_click,next_form_field,place_form_cursor,edit_scroll,
        {tab,record}_click,select_record,scroll_key,preview_is_for_other_run,{restore_rejection,update_saved}_draft,
        next_run_index,{select,install_new}_run,entry_{lines,title,status},empty_lines,draw_rejection_input
     RunFilter(C{query,status,missing,task,selected,tab,scroll})
     = LABELS,TABS
    preview_render
     - {append,is_skipped}_field
   mod
    in_ascii(buffer) > ascii
    nearest_indexed(r,g,b):u8
    to_indexed_colors(buffer) > nearest_indexed
    truecolor_terminal():bool
    without_color(buffer)
    - padded,ascii_terminal,recover,recover_{orchestrator,task},recovery_token
   native_view
    app
     - WantedView,native_{panels,identity,session_id,round}
    render
     render(frame,area,screen,focused) > draw_{cells,cursor}
     - cell_{visible,style}
    session/worker
     Worker(C{client,conversation,role,dimensions,updates,output,stopped,received,screen,sequence,retry,blocked})
      new(start):Self
      run() > open_if_needed,receive,retry_session,apply_command,{read,publish}_screen,detach
      - should_open,reject_{blocked,not_ready},send_{command,command_to_screen},request,record_result,set_error,reject_input
     WorkerStart(C{client,conversation,role,dimensions,updates,output,stopped,received})
    session.rs
     call_input(client,request):Result<Value> > Client.call
     View: start(client,conversation,role,columns,rows,updates,identity):Self > Launch; send(command):Result<()>; resize(columns,rows) > send
     - View: launch,drop
   native_view.rs
    Command(E{Input,Resize,Retry})
    Snapshot(C{screen,error})
    View(C{state,commands,stop,dimensions,identity})
   palette
    catalog
     Action(E{Run,Select,Show,SwitchConversation,Address,OpenProject})
     Command(E{InterviewQuestion,Requirements,Config,Reject,Plan,Plans,Resume,Cancel,Pause,Continue,PauseOrContinue,{New,Switch}Conversation,Find,Search,Details,Diff,Comment,Filter,Help,Quit,AddressMessage,ConversationDetails,Projects,TaskActions,ApprovalsSupervised,ApprovalsAutonomous,PendingApprovals,TakeControl})
     ConversationItem(C{id,project,title})
     Kind(E{Commands,Agents,History,Conversations,Recipients,Projects})
     Picker(C{kind,query,selected,conversations,projects,confirm_delete})
      new(kind):Self > Picker
      items(view):Vec<(String,Action)> > {unfiltered,matching}_items
      draw(frame,view) > draw_with_cursor
      draw_with_cursor(frame,view,show_cursor) > items,PickerLayout.new,draw_lines,picker_title,draw_picker,place_cursor
     ProjectItem(C{path,title})
     = SELECTION_HELP,COMMANDS
    help
     Help(E{Chat,Graph,Details,Diff,Comment,Picker})
      lines_for_width(inner):Vec<String> > lines,help_line
      max_scroll(area):usize > lines_for_width
      draw(frame) > draw_scroll
      draw_scroll(frame,scroll) > {help,lines_for}_width,draw_help_box
     - help_{box_area,title},{compact,aligned}_help_line,wrap_help_description,append_help_word,
       help_{chat,graph,details,diff,comment,picker}
    picker
     - {command,agent,history,conversation,project,recipient}_items,append_task_recipients,empty_message,append_visible_items
    picker_layout
     PickerLayout(C{box_area,width,inner,visible_rows,start})
      new(area,item_count,selected):Option<Self>
   paste
    sanitize_paste(raw):String
    sanitize_single_line_paste(raw):String > sanitize_paste
   planhistory
    Browser(C{offset,total,loading,error,plan,view,selected,scroll,report_scroll})
     new():Self
     loaded(page)
     key(key):bool > page_{older,newer},move_selection
     draw(frame) > borders,frame_block,sanitize,draw_{graph,report}
     - report_text
   planview
    actions
     PlanView: key(key,view):PlanAction > description_lines,prepare_key,{control,regular}_action; mouse_click(col,row,view):PlanAction > action_state,click_button,right,click_document
     - KeyState,{line,whole_plan}_comment
     - PlanView: move_{cursor,button},selected_comments,{delete,toggle}_comment,activate
    document/build
     PlanView: build_lines(view):Vec<DocLine> > raw_document,DocumentBuilder.new; description_lines(view):Vec<DocLine> > build_lines; selected_line_number(view):Option<usize> > selected_task_line,description_lines; line_comments(view,line_no):Vec<&'a Comment>
     - DocumentBuilder
     - DocumentBuilder: build,add_line,update_section,add_empty_document
    document.rs
     section_state(ids,tasks):SectionState > as_str,all
     section_tasks(heading,tasks):Vec<String>
     synthesize_document(tasks):String > append_task_section
     - append_task_{heading,facts},{dependency,parallel}_label
    graph
     panel(title,focused,theme):Block<'static> > title,borders,accent,Theme.dim
     PlanView: draw_task_graph(frame,area,view,theme) ~66 > fit_hint,panel,len,PlanGraph.new,reveal,new_window,draw_task_details; selected_task_line(view):Option<usize> > build_lines; graph_key(key,view):bool ~52 > len
    render
     buttons
      PlanView: draw_buttons(frame,bar_area,view,theme) > button_areas,len,ButtonState,draw_button_group
      - PlanView: draw_button,button_{label_tone,style}
     rows
      PlanView: layout_rows(view,doc_lines,width,theme):(Vec<VisualRow>,(usize,usize)) > {whole_plan,append_document}_rows
      - Piece,append_text_pieces,text_piece_row,piece_spans,{gutter,document}_style,gutter,{append_heading,heading}_badge,
        append_line_comments,comment_{lines,style}
    render.rs
     PlanView: draw(frame,area,view,theme) ~45 > frame_layout,draw_{banner,document},description_lines,draw_{task_graph,buttons}
     - PlanView: comment_{banner,notes},task_banner,keep_cursor_visible
   planview.rs
    BottomButton(E{Accept,Reject,CommentWholePlan,Review})
     next():Self
     prev():Self
    DocLine(C{number,text,section,task_id,is_heading,state})
    PlanAction(E{None,Close,Accept,Reject,CommentLine,CommentWholePlan,SubmitReview,ToggleResolveComment})
    PlanView(C{plan_id,cursor,scroll,button,button_rects,doc_area,row_docs,tick,graph_{focused,selected,offset,area},task_rects,graph_reveal,task_scroll})
     new(plan_id):Self
     whole_plan_comments(view):Vec<&'a Comment> > as_str
    SectionState(E{Open,Working,Problem,Done})
    = GUTTER=7
    - VisualRow
   render
    chat
     - paint_input_{background,lines}
    ledger
     draw_history_or_ledger(app,frame):bool
    overlays
     editor
      draw_editor(app,frame):bool ~42 > visual_text_lines,comments_on,visual_cursor,editor_box,draw_editor_cursor
      - editor_title
     modal
      draw_graph_filters(app,frame)
      draw_unblock(app,frame)
    overlays.rs
     draw_remaining(app,frame) > draw_{selection,editor,interview,picker,help,unblock,graph_filters}
    specialized_dashboard
     - DashboardRows,dashboard_order,append_dashboard_section
    specialized_dashboard_cards
     dashboard_card_lines(details,section,focused,theme):Vec<Line<'static>> > fold_of,{visible,dashboard}_card_body,add_dashboard_card_tail
     - dashboard_card_header,card_marker,{hidden,folded}_card_line
    specialized_details_layout
     clean_header_spans(spans,theme) > Theme.clean
     dashboard_hint(searching,query,width):String > fit_hint
     detail_areas(area,overview,summary_len):DetailAreas > DetailAreas
     detail_block(details,body,theme):Block<'static> > Details.overview,borders,title,style,faint
     document_key(key,tab):Option<&'static str>
     draw_document_scrollbar(frame,area,total,scroll,theme) > draw_scrollbar
     draw_record_scrollbar(frame,area,total,scroll,theme) > draw_scrollbar
     draw_scrollbar(frame,area,total,scroll,theme) > glyphs,faint,Theme.dim
     latest_round(details,view):u64 > task_id
     overview_hint(searching,query,width):String > fit_hint
     overview_split(body):Option<[Rect; 2]>
     overview_split_hint(searching,query,width):String > fit_hint
     position_record_scroll(lines,tab,show_info,follow,scroll,reveal,height) > reveal_check_scroll,is_overview
     records_hint(details,width):String > details_footer_hint_for
     reveal_check_scroll(lines,scroll,height)
     round_label(details,selected):String
     DetailAreas(C{header,summary,body,footer})
    specialized_overview
     - composer_status_area,session_{block,title},ascii_border_set,session_round_label,
       {native,draw_session,composer,primary_composer}_status,mode_label,place_session_cursor
    specialized_screens/graph
     - render_card_title,round_label,render_card_{round_data,history_message},graph_vertical_markers,expanded_offset_x,
       graph_{position_marker_glyph,title,progress_areas},reveal_{axis,tall_axis}
    specialized_screens.rs
     draw_secondary_or_main(app,frame) > draw_{plan,diff,details}
     - button_rows
    transcript
     - {Transcript,Viewport}Rows,visible_rows,transcript_comment,workflow_card_text
    transcript_cache
     fingerprint(card,comment):u64
     TranscriptLayout(C{context,entries,cached,order,retained_rows,stores})
      clear()
      rows(index,fingerprint,found):Option<&[Row]>
      store(index,fingerprint,found,rows) > remove,len,CachedRows
      retained_rows():usize
      cached_entries():usize > len
     TranscriptLayoutEntry(C{fingerprint,found,height,end})
     - HashWriter,RowSlice
     - HashWriter: write_str
    transcript_cards
     comment_status(app,id,status):(String,Option<String>) > comment_{state,error}
     - CommentCardRef,card_fill,message_{rows,card_props,heading,right},comment_card,{plan,system}_rows,tone_style,
       comment_body
   result_text/agent
    agent_result(raw):String ~52 > section
    - append_value
   result_text.rs
    worker_report(raw):String > append_selection
    ReportStream(C{pending,structured,selection})
     push(chunk,sink):crate::Result<()> > len
     finish(sink):crate::Result<()> > finish_with
     finish_with(format,sink):crate::Result<()>
   sanitize
    sanitize(input):String
    sanitize_bounded(input,byte_limit):String > append_visible
    sanitize_line(input):String > sanitize
    Sanitizer(C{state})
     at_text_boundary():bool
     can_borrow_visible(text):bool > at_text_boundary,all,is_bidi_control
     push(input):String > append_chunk
     append_chunk(input,output) > append_visible,transition
    - State,Visible,visible_character
    - State: from_{text,escape,csi,string}
   selection
    TextSelection(C{area,context,cells,anchor,end,dragging,last_click,origin,pinned})
     invalidate_context(context)
     draw_scrolled(buf,area,context,origin) > update_snapshot,highlight
     mouse(event) > hit_test,mouse_down
     text():Option<String> > range,len,sanitize
     - capture,{pin,clear_active}_selection,select_word,row_{start,text},select_word_range
    - VisibleGrapheme,PinnedSelection,is_double_click
    - VisibleGrapheme: document_position,contains
   shell
    run(options,client):Result<()> > initialize,from_environment,enable_capture,run_loop,store,disable_capture
    - Rendering,apply_pending,{advance,draw}_frame,copy_clipboard,handle_events
   state
    update_delivery
     - clear_comment_failure,preserve_background_comment,receipt_is_delivered,pending_delivery_error
    update_events
     - {delivery,failure}_matches,save_delivery_failure,ledger_read_is_stale,is_ledger_refresh_event,ledger_scope_changed
    update_records
     - clear_background_ledger_review,{save,apply_saved}_config
   state.rs
    App(C{native_views,plan_history,graph_layout_cache,transcript_layout,text_{selection,area,origin},pending_clipboard,config_{editor,drafts},agent_drafts,client,project,conversation,generation,stop_poller,tick,view,interview_forms,input,input_{shown,width},editor_width,scroll,found,notice,updates,focus,selected,relations,offset,details,diff,{plan,ledger}_view,editor,picker,help,help_scroll,terminal_area,history,history_{path,index,draft,draft_cursor,draft_preferred_col,item_cursors},sending,sending_comment,pending_{send,comment},{send,comment,diff,task}_request_id,failed_{send,comment},{conversation,comment}_drafts,{comment,send}_operations,observer,comment_draft_cursors,conversation_states,graph_{scope,visibility_exception,revealed,collapsed,grouping,expanded},animate_graph,graph_{anim,anim_anchor,tab,card_scroll,card_area},details_{scroll_memory,query_memory},{graph,now}_area,now_click_targets,phase_action_{area,command},chat_area,graph_viewport,unblock,approval,approval_hidden,graph_filters,projects,active_project})
     open_unblock(named) > targets_orchestrator,orchestrator,unblock_task,Unblock.new
     unblock_choice(state,choice) > open_unblock_note,start_unblock_recovery
     run_workflow(command) > open_{ledger,interview_answer,plan_history,current_plan},reject_plan,resume_plan,request_cancel,set_paused,open_{config,unblock},show_pending_approvals,take_control,new_conversation,{open_conversation,open_projects}_picker
     accept_plan() > plan_proposed,send_command
     send_command(notice,request) > request
     submit() > {reject_duplicate,begin_chat}_send
     submit_agent_message() > agent_message_target,prepare_agent_message
     submit_comment() > reject_duplicate_comment,is_rejection_target,begin_comment_send
     return_to_main() > hide_ledger,remember_details,close_overlays,{preserve_comment,hide_config}_editor
     apply_update(update) > apply_update_inner,return_to_main,{invalidate,selection}_context
     help_max_scroll():usize
     open(conversation) > {save_active,activate,restore}_conversation,reset_history_recall,start_poller
     new_conversation() > as_str
     open_diff(key) > DiffView.new,fetch_diff
     fetch_diff()
     diff_key(key) ~42 > show_help,get_comment_draft,Editor,fetch_diff,request
     graph_scope_with_exception():graph::Scope
     open_graph_filters()
     pan_graph(graph,forward,amount,sideways)
     select_node(key) > card_key
     toggle_selected_collapse() > remove
     set_graph_filter(query) > current_graph,visible_selection
     sync_interview()
     interview_visible():bool
     send_interview_answer(text)
     open_conversation_picker() > {daemon,append_project,append_saved}_conversations,ensure_active_conversation,Picker.new
     set_comment(id,status) > request
     selection_context():String > hash
     selection_blocked():bool > interview_visible
     mouse_click(col,row) > click_plan_view,dialog_open,click_{phase_action,help,now,graph},{on,click}_header,click_panel
     click_panel(col,area)
     activate_header_token(token,controls) > run_workflow
     bar_height(area):u16
     main_areas(area):[Rect; 3]
     mouse_scroll(down) > scroll_{help,details,graph,chat}
     mouse(mouse):bool > handle_native_mouse,sync_interview,handle_{modal_mouse,content_mouse}
     register_project(conn):usize > Projects.from_env,spawn_project_poller,len
     switch_project(index,conversation) > latest_or_new,restore_project_history,switch_config_workspace,open
     open_project(path) > expand_home,switch_project,frontend_binary,workspace.connect
     open_projects_picker() > open_project_items,append_remembered_projects,Picker.new
     fetch_task() > prepare_task_fetch,fetch_run,run_task_fetch
     fetch_run() > {prepare,run}_run_fetch
     open_details(key) > restore_details,{resolve_details,fetch}_task
     request(request,done)
     task_row_round(key):Option<u64>
     refetch_stale_details() > details_are_stale,fetch_task
     handle_plan_action(action) > accept_plan,open_plan_{rejection,comment},submit_plan_review,set_comment
     header_line(width):Line<'static> > header_{project_label,status},controls,prepend_header_notice
     header_status():String > len,flatten,short,total,stage,glyphs,Theme.from_env
     key(key):bool > key_inner,invalidate_context,selection_{context,blocked}
     config_key(key):bool > {reload,save}_config
     open_config() > config_request
     details_dashboard_key(key,plain):bool
     details_tab_key(key):bool
     details_content_key(key,plain):bool > details_{check_comment_key,round_key,control_scroll_key,scroll_key}
     details_primary_key(key,plain):bool ~46 > details_close_or_search,run_workflow,details_overview_key,show_help,details_{open_comment,comment_status}
     details_key(key) > details_{edit_key,search_key,screen_key}
     graph_key(key):bool ~48 > card_key,fetch_task,open_graph_diff,expanded_card_key,graph_{for_key,scroll_key,direction},move_graph_selection,graph_action_key
     scroll_expanded_card(lines) > graph_console_active,scroll_console
     graph_console_active():bool
     scroll_console(lines) > native_send
     graph_for_key():Graph > new_with_expanded
     toggle_focus()
     route_graph(key):bool > graph_{command,key}
     key_inner(key):bool > route_native_or_history,sync_{interview,approval},route_{ledger,selection_and_config,exit,overlays,screen,base_layout}
     paste(text) > paste_native_or_overlay,sync_interview,paste_{form,details,picker_editor_or_chat}
     comment_key(conversation,plan,target):String
     get_comment_draft(target):InputBuffer > take_comment_draft
     save_comment_draft(target,input)
     remove_comment_draft(target) > remove_draft
     recall_history(older) > save_history_cursor,history_target,restore_history_input
     load_history(state) > history.load
     record_history(text) > len,remove,history.append
     chat_key(key):bool > chat_escape,move_home,chat_history_or_resume,run_workflow,chat_{buffer_end,up,down}
     editor_key(key) > save_comment_draft,submit_comment,move_{up,down}
     open_ledger() > ledger_action
     ledger_action(action) > route_action,mark_pending,LedgerWork,spawn_ledger_work
     sync_native_views() > {wanted,retain,attach}_native_views
     native_focused():bool > graph_native_key,native_key_role
     native_send(input) > native_key_role,recover_native_input
     recover_native_input(conversation,role,input,error) > recovered_input_text,save_native_draft,restore_native_composer
     native_retry() > native_key_role
     help_here():Help
     show_help(help)
     help_key(key):bool > help_max_scroll
     picker_key(key):bool > delete_selected_conversation,pick_selected
     run(command):bool > run_command
     load_plan_history(offset)
     draw_chat(frame,area,theme) > chat_layout,draw_{phase_action,sending,transcript,chat_input}
     draw(frame) > sync_interview,draw_min_term_guard,sync_{native_views,approval},draw_{history_or_ledger,secondary_or_main,remaining}
     draw_graph(frame,area) ~57 > card_key,graph_{areas,expansion,scope_with_exception},get_with_expanded,keep_graph_selection_visible,center_animated_card,draw_graph_position_markers,new_window,Theme.from_env,draw_{expanded_card_panels,progress}
     graph_expansion():(Option<String>,u8)
     draw_main(frame) ~47 > draw_{chat,graph},approval_panel_height,draw_footer
     draw_transcript(frame,area,theme) > prepare_transcript_rows,add_workflow_card,{add_plain,add_empty}_transcript,draw_transcript_viewport
     transcript_visible_rows(start,end,history_height,extra,theme):Vec<Row> > append_visible_history
     transcript_card_rows(entry,found,width,theme):Vec<Row> > comment_{card_ref,rows},standard_card_rows
     apply_update_inner(update) > apply_{native_input_rejected,plan_history,agent_message,ledger,config,events,conversation,project_ready,project_status,notice,task_failed,task,diff,interview_result,send_result,comment_result}
     apply_native_input_rejected(update) > recover_native_input
     apply_agent_message(update) > apply_agent_receipt
     apply_interview_result(update)
     apply_send_result(update) > remove,finish_{active_send,background_send}
     apply_comment_result(update) > {finish_active,finish_background}_comment
     apply_task_failed(update) > {fail_active,fail_background}_task
     apply_task(update) > apply_task_snapshot,TaskSnapshotReply
     apply_diff(update)
     apply_events(update) > apply_{delivery_events,active_event_batch}
     apply_plan_history(update)
     apply_ledger(update) > ledger_reply_view,finish_ledger_decision,ledger_action
     apply_config(update) > state_dir,apply_config_save,refresh_config
     apply_conversation(update) > open
     apply_project_ready(update) > {register,switch}_project
     apply_project_status(update)
     apply_notice(update)
     bootstrap(client,updates):App
     test_app():App > Client.at
     test_app_with_client(client):(App,super::UpdateReceiver)
     test_app_with_client_and_updates(client,updates):App
     - named_unblock_task,reset_input_cursor,launch_chat_send,comment_operation_id,prepare_comment_send,comment_context,
       restore_conversation_draft,initialize_new_conversation,clear_conversation_overlays,
       reset_conversation_{navigation,requests,delivery},daemon_conversation,over_graph,update_selection_context,
       modal_blocks_mouse,click_{graph_filters,unblock},task_fetch_identity,mark_task_fetch,latest_round_for_key,
       {checks_are,session_is}_stale,latest_round,open_comment_editor,config_save_guard,
       details_{jump_key,resume_round,retry_native,start_composer,toggle_info,native_focus_key,route_action,fetch_current_task},
       remove_search_grapheme,graph_tab_count,set_graph_tab,select_visible_graph_node,open_graph_comment,route_native_key,
       is_editing,route_{graph_filters,unblock},embedded_diff_graph_active,route_{interview,plan_view,global_shortcut},
       chat_{paste_enabled,down_with_text},attach_native_view,erase_picker_query,picker_action,select_picker_node,
       show_picker_entry,close_picker_views,switch_conversation,address_recipient,open_comment,phase_action,
       chat_input_window,paint_input,place_chat_cursor,draw_{card_details,graph_frame},repair_graph_selection,
       fit_graph_offset,reveal_selected_node,sync_transcript_context,update_transcript_entry,transcript_card_height,
       position_transcript_scroll,{paint_transcript,append_card}_rows,restore_active_{send,comment},clear_task_fetch,
       task_{round_moved,reply_matches_current_round},{load_active,load_background}_task,
       apply_{mail_delivered,mail_delivery_failed,view_events},update_ledger_approval,refresh_visible_ledger,
       open_pending_plan,install_fresh_config,record_config_error
   taskinfo/effects
    coverage_full(row):String > coverage_count,settled_from,append_unknown_ids
    coverage_short(row):String > settled_total
    effect_front(row):Option<String> > effect_shape
    effect_line(row):Option<String> > effect_shape
    integration_detail(row):String > as_str
    integration_label(row):String > as_str
    integration_line(row):String
    settled_total(row):Option<(u64,u64)> > settled_from
    - effect_pieces,append_unknown_summary
   taskinfo.rs
    card_meta(role,view,row,width):String > coverage_short,integration_label,waiting_on
    cause(row):Option<String>
    doing(view,key):Option<String>
    next_step(view,row):String > waiting_on
    reviewer_activity(view,row):Option<String> > doing,as_str,cause
    reviewer_state(row):String > reviewer_status,cause
    reviewer_status(row):String > round_shown,as_str
    round_shown(round):Option<String>
    task_activity(view,row):Option<String> > {reviewer,worker}_activity
    waiting_on(view,row):Vec<&'a str> > as_str
    with_provider(state,provider):String
    worker_activity(view,row):Option<String> > doing,waiting_on,cause
    worker_state(view,row):String > worker_status,waiting_on,cause
    worker_status(row):String > round_shown,as_str
    - one_line
   theme
    ascii
     ascii_character(ch):char > screen_ascii_symbol
     ascii_grapheme(grapheme):String > ascii_symbol
     - ascii_{basic_grapheme,polish,borders,marks}
    rows
     card(theme,props):Vec<Row> > card_body,gap
     draw_rows(buf,area,rows)
     scrollbar(buf,theme,area,total,top,view) > glyphs,Theme.dim,faint
     section(theme,title,meta,focused,width):Row > accent,title,split_row,Theme.{fit,clean,dim}
     split_row(fill,left,right,width):Row > width,Row.new
     title_row(theme,title,width):Row > overlay,split_row,Theme.dim
     CardProps(C{fill,accent,head,right,body,max_body,width})
     Row(C{fill,line})
      gap():Self
      new(fill,spans):Self
     - card_heading,body_row
    tokens
     tone(status):Tone
     Glyphs(C{logo,bar,vertical,horizontal,dashed_{vertical,horizontal},arrow_{right,left},to,dot,active,queued,done,failed,paused,ellipsis,thumb,track,pointer,waiting,problem,orchestrator,plan_reviewer,auditor,worker,reviewer,spinner,junctions})
     Theme(C{color,ascii})
      new(color,ascii):Self
      from_env():Self > ok
      glyphs():&'static Glyphs
      spinner(tick):&'static str > glyphs
      role_icon(key):&'static str > glyphs
      role_accent(key,kind):Style > pick,fg
      text():Style > pick,fg
      progress_marker():Style > pick,fg
      dim():Style > pick,fg
      faint():Style > pick,fg
      accent():Style > pick,fg
      title():Style > pick,fg
      bar():Style > pick,card_style
      overlay():Style > pick,card_style
      card():Style > pick,card_style
      panel():Style > pick,card_style
      raised():Style > pick,card_style
      selected():Style > pick,card_style
      button():Style > pick,card_style
      separator(focused):Style > pick,fg
      status(status):Style > tone,pick,fg
      status_glyph(status):&'static str > glyphs,tone
      checkbox(status):String ×0 > tone
      person(name):Style > pick,fg
      edge(kind):Style > pick,fg
      ok():Style > pick,fg
      info():Style > pick,fg
      warn():Style ×0 > pick,fg
      err():Style > pick,fg
      purple():Style ×0 > pick,fg
      warning():Style > pick,fg
      error():Style > pick,fg
      clean(text):String > sanitize
      fit(text,width):String > glyphs
     Tone(E{Active,Queued,Done,Failed,Waiting})
     = UNICODE,ASCII,BG,PANEL,LINE,RAISED,SELECTION,FG,TITLE,DIM,FAINT,BLUE,GREEN,YELLOW,ORANGE,RED,PURPLE,CYAN,GOLD,PINK
   theme.rs
    wrap(text,width):Vec<String> > sanitize,hanging_indent
   unblock
    choices_for(task):Vec<Choice> > has_delivery,written_recovery_choices,restart_choice
    is_blocked(status):bool
    Choice(C{action,label,asks_text})
    Outcome(E{None,Close,Choose})
    Unblock(C{plan,task,title,status,reason,choices,selected,rects})
     orchestrator():Self > TaskRow
     new(plan,task):Self > choices_for
     key(key):Outcome > len
     click(col,row):Outcome
     draw(frame) > centered_box,len,title,borders,lines
    - choice,retry_choice,title_lines,choice_{rects,lines},append_footer
   update_queue/queue
    update_channel(capacity):(UpdateSender,UpdateReceiver) > Shared,State,UpdateSender,UpdateReceiver
    QueueClosed,QueueEmpty,UpdateSender(C)
     UpdateSender: send(update):Result<(),QueueClosed> > len,wait
    UpdateReceiver(C) : Drop
     try_recv():Result<Update,QueueEmpty>
     recv_timeout(timeout):Result<Update,QueueEmpty>
     - drop
   view
    comments
     View: check_comments(task,check):Vec<&Comment>; comments_on(key):Vec<&Comment>; stale(comment):bool; review(key,review) > review_word,agent_name; progress():Vec<String> > now_click_rows; now_click_rows(limit):Vec<ProgressEntry> > {initial,task,agent}_progress,ProgressEntry,waiting_progress
    conversation/progress
     is_running(status):bool
     progress_line(view,id,title,was,now,reason):Option<(String,crate::lean::tui::theme::Tone)> > progress_status,len
    conversation.rs
     View: comment_state(id):&str > as_str; comment_error(id):Option<&str>; comment_activity():Option<String>; orchestrator_activity():Option<String> > comment_activity; push(style,text) > Entry; plan_proposed():bool > all; stage():Stage > workflow_stage,plan_proposed,is_running,stage_after_activity; draft_plan_comments():Vec<&Comment> > open_plan_comments; open_plan_comments():Vec<&Comment>; plan_resumable():bool > all; agent_key(role):String; mail_key(name):String; report_progress(id,title,was,now,reason) > {progress,append_progress}_line; system_card(title,text,tone) > push; advance_messages(from,to); row_of(key):Option<&mut TaskRow>
     - View: has_started
    header/footer
     main_footer_hint(focus,plan_{proposed,resumable},notice,width):String > truncate,plan_hint,{chat,graph}_footer
     transcript_empty_state(width):Vec<Line<'static>>
    header.rs
     render_header(project,conversation,status,focus,width):Line<'static> > render_header_with,Controls.default
     render_header_with(project,conversation,status,focus,width,controls):Line<'static> ~74 > Theme.from_env,active_style,compact_header,append_panels,identity_parts,append_status_and_actions
     Controls(C{pause,resume_{paused,plan},cancel}) : Default
     View: controls():Controls > Controls,plan_resumable
     = PAUSE="[ ‖ Pause ]",CONTINUE="[ ▶ Continue ]",RESUME="[ ↻ Resume ]",CANCEL="[ ✕ Cancel ]",PLAN="[Plan]",
       NEW="[ + New ]",PAUSE_SHORT="[Pause]",CONTINUE_SHORT="[Continue]",RESUME_SHORT="[Resume]",CANCEL_SHORT="[Cancel]",
       NEW_SHORT="[+ New]",PAUSE_ICON="[‖]",CONTINUE_ICON="[▶]",RESUME_ICON="[↻]",CANCEL_ICON="[✕]",NEW_ICON="[+]"
     - button,{append_full,append_short}_actions
    model
     compact(n):String
     Agent(C{branch,provider,model,effort,running,pid,activity,turns,reported,{busy,last,started}_ms,tokens_{in,out},estimated,estimated_{in,out}})
      usage():Usage
      tokens():String > usage
     Comment(C{id,plan_id,target,text,status})
     CommentProgress(C{state,error})
     Link(C{count,last})
     Stage(E{Fresh,Interview,Planning,Review,Executing,Blocked,Audit,Done})
      step():usize
      strip(glyphs):String > step,label
      label():&'static str
     Usage(E{Reported,Estimated,Unavailable})
      total(agents):Self
      short():Option<String>
     = STEPS
    presentation
     agent_name(key):String
     bounded_coverage(coverage):Value > bound,clean_line
     bounded_landing(integration):Value > bound,clean_line
     clean(text):String > sanitize
     clean_line(text):String > sanitize_line
     draw_min_term_guard(frame,area):bool > style,wrap,fg
     duration(ms):String
     fit_hint(parts,width,keep_last):String > as_str
     status_label(status):&str
     target_name(target):String > {structured_target,agent}_name
     truncate(text,width):String > Theme.new
     wrap(text,width):Vec<String>
     - {plan,compound}_target_name
    projection
     agents
      on_agent_final(view,event,payload,key,turn) > as_str,TurnResult.decode
      on_agent_tool_rejected(view,event,payload,key,turn) > as_str,fg
      on_turn_finished(view,event,payload,key,turn) > as_str,finish_orchestrator,report_planner_failure,finish_agent_turn
      on_turn_started(view,event,payload,key,turn) > as_str,start_role_activity,record_agent_start
      - finish_comment_turn,report_orchestrator_failure,turn_outcome
     approvals
      on_approval_escalated(view,_event,payload,_key,_turn) > as_str
      on_approval_mode_changed(view,_event,payload,_key,_turn)
      on_approval_requested(view,_event,payload,_key,_turn) > as_str,flatten,PendingApproval
      on_approval_resolved(view,_event,payload,_key,_turn) > as_str
     conversation
      on_interview_question(view,event,payload,key,turn) > as_str,len
      on_ledger_approval_required(view,event,payload,key,turn) > as_str
      on_ledger_changed(view,event,payload,key,turn) > as_str
      on_message(_view,event,payload,key,turn) > as_str
      on_message_2(view,event,payload,key,turn) > as_str,message_text,{record_user,record_runtime,record_agent}_message
      on_planning_progress(view,event,payload,key,turn) > as_str
      on_scope_confirmation_requested(view,_event,payload,_key,_turn) > as_str
      on_session_started(view,event,payload,key,turn) > as_str
      on_user_decision_requested(view,event,payload,key,turn) > as_str
      on_workflow(view,event,payload,key,turn) > as_str
      - update_user_message_state,append_user_message
     graph
      on_graph_task(view,payload) > as_str,graph_task_row,insert_graph_task,refresh_parallel_tasks
      on_graph_updated(view,payload) > as_str,apply_graph_order,report_graph_amendment
     mail
      on_comment(view,payload) > store_comment_scope,mark_comment_queued,target_name,comment_event_text,fg
      on_comment_replied(view,event,payload,key,turn) > as_str,flatten
      on_mail(view,payload) > event_text,mail_{sender,message_entry,line},fg,record_mail_link
      on_mail_delivered(view,event,payload,key,turn) > as_str,flatten,remove
      on_mail_rejected(view,payload) > event_text,mark_rejected_messages,fg
     mod
      apply(view,event) > as_str,event_is_visible
      - graph_{task,updated},task,integration,task_recovery,mail_{message,rejected},comment
     plan
      on_plan_invalid(view,event,payload,key,turn) > as_str,report_plan_invalid
      on_plan_proposed(view,_event,payload,_key,_turn) > as_str,proposed_tasks,fill_parallel_fallback,reset_plan_agents,append_plan_review,publish_plan_summary
      on_plan_rejected(view,event,payload,key,turn) > as_str,fg
      on_plan_review(view,event,payload,key,turn) > as_str
      - proposed_{task,activity},strings
     reviews
      on_audit(view,event,payload,key,turn) > as_str,fg
      on_code_review(view,event,payload,key,turn) > as_str,fg
      on_notice(view,event,payload,key,turn) > as_str,fg
      on_paused(view,event,payload,key,turn) > as_str,fg
      on_requirements_failed(view,event,payload,key,turn) > as_str,fg
     tasks
      on_check(view,event,payload,key,turn) > as_str
      on_integration(view,payload) > clean_line,bound,event_text,report_integration,update_integration_row
      on_task(view,payload) > clean_line,as_str,update_task_row,report_task_reason
      on_task_recovery(view,payload) > clean_line,event_text,bound,update_recovery_row
      - TaskProgress,update_task_identity,clear_finished_activity,update_task_feedback,apply_recovery_outcome
   view.rs
    dim():Style > fg
    Entry(C{style,text})
    MessageEntry(C{id,sender,target,text,state,kind})
    ModalKind(E{None,Help,Picker,CommentEditor,Diff,Details})
    PanelFocus(E{Chat,Graph})
    PendingApproval(C{request,role,provider,summary,orchestrator,options})
    ProgressEntry(C{text,agent})
    TaskRow(C{id,uuid,title,summary,reviewer_summary,section,kind,goal,depends_on,parallel_with,change_kind,focus,risks,status,round,activity,reason,revision_token,effect,effect_cut,obligations,integration})
    TranscriptCard(E{Message,Plan,System,Comment})
    View(C{workflow_phase,session_branch,interview_question,scope_confirmation_summary,user_decision,requirements_{summary,approval_run,approval_event,progress},latest_user_message,comment_scopes,layout_revision,{check,task_check}_events,transcript,tasks,plan,plan_{doc,preview},activity,idle,after,agents,links,mailbox,comments,comment_progress,reviews,paused,autonomous,approvals,messages,cards,{plan,conversation}_revision_token,roles,comment_turns,mail,task_rows})
     apply(event)
    = MIN_TERM_WIDTH=12,MIN_TERM_HEIGHT=5,SIDE_BY_SIDE_BREAKPOINT=85,ORCHESTRATOR="orchestrator",
      PLAN_REVIEWER="plan-reviewer",AUDITOR="auditor"
   workspace
    badges(conversations):(usize,usize) > as_str
    connect(path,executable):Result<ProjectConn,String> > resolve_config,project_state,private_directory,ProjectConn.new
    latest_or_new(conn):Result<String,String> > flatten,as_str
    name_of(path):String
    others_summary(projects,active):Option<String>
    ProjectConn(C{path,name,client,branch,conversations})
     new(path,client):Self > name_of
     badges():(usize,usize)
  turn_result
   review
    task_tool_result(tool,args):Result<TurnResult> > optional_text,selection_tool,required_text,require,review
    - Finding,resolved_items,review_checks,validate_check,{review,record}_findings,record_finding,unique_finding_id,
      review_approved
   tool_results
    report_with_deliverables(report,selection):String
    required_text(args,field):Result<String> > as_str
    tool_result(tool,args):Result<TurnResult> > {report,plan,question}_tool,task_tool_result
    - Addressed,addressed,kept
   types
    Alternative(C{id,approach,tradeoffs})
    Selection(C{alternatives,selected,rationale})
     validate():Result<()> > require,len
     chosen():Option<&Alternative>
    TurnResult(E{Report,Plan,Question,Requirements,Completed,Review,Blocked,NeedsInput,Ledger,Audit,AuditReview})
     encode():Result<String>
     decode(text):Self
     plan_answer(text):(String,Option<SubmittedPlan>)
     review_verdict(text):Option<Verdict>
     addressed():Option<&[u64]>
     validate_task(spec,reviewer):Result<()> > require,len,validate_task_shape
     text():String
   validation
    validate_worker_commit_subject(message):Result<()> ~47 > all
    - validate_{completion,review,coverage,findings,finding},required_finding_id,validate_{finding_fields,waiting},
      invalid_task_result,validate_{complete_coverage,verdict_consistency}
  workflow/repair
   queue_requirements_format_repair(tx,conversation):Result<bool> > latest_missing_ledger_failure,repair_already_handled,record_requirements_retry
  workflow.rs
   begin_interview(tx,conversation):Result<()> > record_event
   Workflow(C{phase,summary,upto_message})
    interviewing():bool
    planning():bool
 lib
  hash(bytes):String
  id():String
  require(condition,message):Result<()>
  Error(E{Invalid,Io,Json,Sql})
  - sql_message×0
 main
  - main,run
 transport
  frame
   read(reader):Result<Option<T>> > read_{frame,chunks}
   write(writer,value):Result<()> > write_chunks
   = MAX_FRAME
   - chunk_end,validate_chunk,chunk_{text,done}
  json
   decode(bytes):Result<T>
   - Unique,UniqueJson
   - Unique: deserialize
   - UniqueJson: expecting×0,visit_{bool,i64,u64,f64,str,unit,seq,map}×0
? tests
 src
  config
   tests
  lean
   plan_tests
   reliability_tests
   agent_tools
    tests:*core
    tests
     blockers
     dependencies:*authorize
     result_evidence:*fixture,start
   cli
    tests
   core
    integration/land
     tests
    orchestration/approvals
     tests:*dialog,setup,settle
    task
     artifacts
      tests:*fixture
     recovery
      test_support
    worktree/cleanup
     tests:*fixture,checkout
   daemon/protocol
    tests
   git/artifact_update
    tests
   history
    tests
   launch
    tests
   ledger/runtime
    tests
   mcp
    tests
    dispatch
     tests:*shared,data
   native
    tests:*core
   rounds
    tests:*fixture,execute
   run_context
    tests:*context
   store
    tests
    tests
     agent_exchange:*fixture
     events_and_recovery
     plan_revision:*fixture,fixture_for
     rows_and_feedback
     schema_and_comments
     plan_revision
      continuations:*apply
   tasks
    tests:*verdict_with,submitted
    tests
     artifact_paths:*plan
     card_outcomes
     coverage:*verdict_with
     options_and_gates
     parsing_and_validation
     plans_and_messages
     requirements_and_graph
     verdicts_and_bounds:*verdict
   tui
    tests
    config
     tests
    details
     tests:*screen
    document
     tests:*page
    editor
     tests
    graph
     tests:*task,view
    palette
     app_tests
     tests
    planview
     tests:*screen,proposed,states,comment
    selection
     tests
    taskinfo
     tests:*row
   turn_result
    tests
   workflow
    tests
  transport
   tests
 tests
  lean_approvals:*fake_codex,config,wait_idle
  lean_core:*fake_codex,config,wait_idle,chat
  lean_ledger:*Fixture
  lean_ledger_tui:*key,screen
  lean_messages:*open,prompts,events,run,accept_reviewed_plan,ReleaseOnDrop
  lean_options:*writer,run
  lean_options_delivery:*task
  lean_options_probe:*git,idle,accept_reviewed_plan
  lean_plan:*fake_clis,config,wait_idle,calls
  lean_plan_history:*fake_clis,DaemonGuard
  lean_projects:*project,Daemon,pump,conversations
  lean_protocol:*fake_codex,DaemonGuard
  lean_recovery:*fake_clis,open,git,wait_idle,statuses
  lean_review:*open,wait_idle,events,ask
  lean_tui:*TestTempDir,Daemon,scoped,open_cards
  lean_workflow:*idle,plan_proposed
  lean_writer:*fake_clis,config,git,project,wait_idle,card
  common
   mod:*tempdir,assert_test_socket,runtime_environment,install_probe_cli
   options_cli.py
   record_options_checks.py:*digest,inputs,execute,main
  support
   fixture_cli:*interactive
   ledger_cli.py:*_ledger_{emit,block}
   mod:*install_ledger_protocol,fixture_conversation,confirm_scope_store,install_task_options_protocol
   pty_shim.py:*ask,without,planned_task,call,finish,dialog,main
· 134,934 LOC · input 1,200,706 · output 45,427 tokens · state 9ec949fb9697 · targets AGENTS.md
```
<!-- hologram:end -->
