# Session & Plan Model

This document defines the identity contract, storage model, lifecycle, and scope validation for multi-plan conversations in the lean core runtime.

---

## 1. Identity Contract & Hierarchy

Every runtime entity belongs to a strict, unambiguous hierarchy:

```
Project (canonical filesystem path)
  └── Endpoint / Config / State (`core.sock`, `antislop.toml`, `<state>/core.db`)
        └── Conversation (`conversation_id`, UUID)
              ├── Plan (`plan_id`, UUID)
              │     ├── Task (`task_uuid`, UUID)  [Local label: `t1`, `t2`, ...]
              │     │     ├── Worker Role (`worker:<task_uuid>`)
              │     │     └── Verifier Role (`verifier:<task_uuid>`)
              │     └── Review Role (`code-review:<task_uuid>`)
              ├── Conversation-level Roles
              │     ├── Orchestrator (`orchestrator`)
              │     ├── Plan Verifier (`plan_verifier`)
              │     └── Auditor (`auditor`)
              └── Turns (`turn_id`, UUID) & Events (`seq`, u64)
```

### Components of the Contract

1. **Project**: The absolute, canonical path to the repository directory (e.g., `/Users/.../project`). A daemon serves exactly one project; a screen may be connected to several daemons, one per project (see `docs/lean-core.md`).
2. **Endpoint / Config / State**:
   - Unix domain socket: `<state>/core.sock` (or `/tmp/antislop-<uid>/<hash>.sock` when state path is long).
   - Configuration file: `antislop.toml` with loaded timestamp for hot reload detection.
   - Persistence directory: `<state>/core.db` (SQLite in WAL mode).
3. **Conversation (`conversation_id`)**: A unique UUID identifying a collaborative user-agent thread.
4. **Plan (`plan_id`)**: A unique UUID generated upon plan proposal (`tasks[0].plan_id`). All tasks proposed together share this `plan_id`.
5. **Task UUID vs. Local Label**:
   - **Task UUID (`task.id`)**: A globally unique UUID (e.g., `4a72e811-30db-4bca-9541-b841a54101e2`).
   - **Short Label (`spec["id"]`, e.g. `t1`, `t2`)**: Plan-local identifier assigned by the orchestrator for readability and dependency wiring.
   - **Contract Rule**: Short labels `t1` are valid **only** when scoped to a specific `plan_id`. They **cannot** identify historical objects globally because different plans in the same conversation routinely reuse `t1`, `t2`. Any global or cross-plan reference MUST use the Task UUID or provide `(plan_id, label)`.
6. **Agent Role**:
   - Singletons: `orchestrator`, `plan_verifier`, `auditor`.
   - Task-bound: `worker:<task_uuid>`, `verifier:<task_uuid>`, `code-review:<task_uuid>`. Notice task-bound roles encode the unique Task UUID, never the ambiguous short label `t1`.
7. **Turn (`turn_id`)**: A unique UUID for every execution of an agent CLI process. Tracks `conversation_id`, `plan_id` (optional for global roles, populated for task roles), `role`, `provider`, `session_id`, `status`, `error`, `upto_message`, `started_ms`, `finished_ms`, and `pid`.

---

## 2. Plan Lifecycle and History Invariance

### Preserving Full History
- Adding a new plan (`Core::propose` / `Store::add_plan`) never deletes, clears, or resets previous plans or database state.
- Both active and historical plans are persisted in the `plans` table:
  ```sql
  CREATE TABLE plans(
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    created_ms INTEGER NOT NULL
  );
  ```
- All tasks from every plan remain recorded in the `tasks` table with their `plan_id`, position, status, rounds, reports, verdicts, git bases, and results.

### Read-Only Active Plan Invariance
- Querying historical plans, inspecting past tasks, reading past diffs, or retrieving past comments is strictly read-only.
- Inspecting history **never** alters the active plan execution queue or current conversation state.
- Frontends can inspect older plans while a current plan is executing without causing state invalidation or race conditions.

---

## 3. Paginated Historical Access API

The daemon provides backward-compatible, paginated endpoints to inspect all stored plans, tasks, agents, turns, reports, and verdicts.

### Request: `Plans`
Retrieves a paginated list of all stored plans for a conversation:
```json
{
  "op": "plans",
  "conversation": "<conversation_id>",
  "offset": 0,
  "limit": 20
}
```
**Response**:
```json
{
  "plans": [
    {
      "id": "<plan_id>",
      "conversation_id": "<conversation_id>",
      "created_ms": 1729250000000,
      "status": "approved",
      "is_active": false,
      "tasks": [...],
      "agents": [
        {
          "role": "worker:<task_uuid>",
          "task_id": "<task_uuid>",
          "task_label": "t1",
          "provider": "codex",
          "session_id": "sess-123"
        }
      ],
      "turns": [...],
      "reports": [
        {
          "task_id": "<task_uuid>",
          "task_label": "t1",
          "round": 1,
          "report": "Worker report text..."
        }
      ],
      "verdicts": [
        {
          "task_id": "<task_uuid>",
          "task_label": "t1",
          "round": 1,
          "verdict": { "approved": true, "findings": [] }
        }
      ]
    }
  ],
  "offset": 0,
  "limit": 20,
  "total": 3,
  "has_more": false,
  "active_plan_id": "<latest_plan_id>"
}
```

### Request: `Plan`
Retrieves detailed history for a single specific plan:
```json
{
  "op": "plan",
  "conversation": "<conversation_id>",
  "plan": "<plan_id>"
}
```

---

## 4. Event and Turn Plan Association

Schema 11 creates plan associations and their indexes directly. Events, turns
and comments carry `plan_id`; global events may have a NULL conversation ID.
History is recorded when work occurs, without startup backfills. Incompatible
schemas require manual archival of `.antislop/` and a fresh state, as described
in [the requirements contract](requirement-ledgers.md#database-compatibility).

---

## 5. Comments, Diffs, and Scoped Actions

### Historical Comments
- Comments support plan scoping:
  ```json
  {
    "op": "comment",
    "conversation": "<conversation_id>",
    "plan": "<plan_id>",
    "target": "task:t1",
    "text": "Please verify edge cases."
  }
  ```
- Reading comments:
  ```json
  {
    "op": "comments",
    "conversation": "<conversation_id>",
    "plan": "<plan_id>",
    "status": "open"
  }
  ```
  Passing `"status": "all"` returns comments regardless of status. Passing `plan: null` returns conversation-wide comments.

### Historical Diffs
- Diffs are queried with plan and task identifier:
  ```json
  {
    "op": "diff",
    "conversation": "<conversation_id>",
    "plan": "<plan_id>",
    "id": "t1"
  }
  ```
  The `id` parameter accepts either the short label (`t1`) or the Task UUID.

---

## 6. Daemon-Side Scope Validation

To prevent foreign-scope access, corruption, or unintended cross-project modifications, the daemon enforces three validation tiers on all plan and task operations:

1. **Project Scope (`validate_project_conversation`)**:
   - Verifies that `conversation.project` matches the canonical project path of the daemon.
   - Rejects requests with: `conversation <id> does not belong to project <path>`.
2. **Conversation Scope (`validate_conversation_plan`)**:
   - Verifies that `plan` belongs to `conversation`.
   - Rejects requests with: `plan <plan_id> does not belong to conversation <id>`.
3. **Plan Scope (`validate_plan_task`)**:
   - Verifies that task `id` (matched by UUID or `spec["id"]`) belongs to `plan`.
   - Rejects requests with: `task <id> does not belong to plan <plan_id>`.

Optimistic concurrency control via `scope_hash` continues to be supported; when a client provides a stale hash on a scoped operation (`AcceptScoped`, `ResumeScoped`, `ReviewScoped`, `CancelScoped`), the daemon rejects with `"stale_scope"`.


## Durable round history (schema 8)

`turn_scopes` records a task UUID and round when a turn starts. Historical turns
without explicit attribution remain unassigned. `task_rounds` retains each round's
report, verdict, Git references and status in immutable session artifacts; updating
the current task does not overwrite an older round. Comments may carry a round or
run scope; comments without scope remain shared.

Full agent text, prompts, final parser answers, mailbox input, and worker reports
are stored under `<state>/session-content/`. SQLite holds references. Existing
untagged text rows remain readable. Previously clipped history cannot be recovered.
The diagnostic budget still applies to reasoning and tool output, never agent text.

The protocol adds `task_rounds`, `task_round`, paginated `session_events` and
`session_content` (a UTF-8 chunk addressed by event, field and byte offset). Task
history requires the task UUID and validates project, conversation and plan.
`send_to_agent` takes an explicit recipient, an operation UUID, and the expected
current round or run. Replaying an operation returns its receipt instead of
sending twice. Legacy send operations retain their existing behavior.

A JSON message larger than one transport frame is transferred in ordered
`antislop_chunk` envelopes. Each frame remains bounded; there is no aggregate
message-length limit. Old small JSON-line requests and responses are unchanged.
External CLI argument limits are reported as actual process errors; prompts are
never silently shortened to fit them.

The inspector opens the newest round/run on each opening. Selecting history stops
following; selecting the latest restores following. Worker and verifier panels are
side by side at 80 columns or wider; Ctrl+W chooses the full-width panel below that.
Prompt and final answer are outside the collapsible session trace. Each recipient's
composer keeps its own draft across closing the inspector and conversation changes.

Inspection uses `task_round` with `references: true`, `session_events` with
`include_messages: false`, paged `history_messages`, and `history_content` for
round reports, verdicts and mail. Omitting the new optional flags preserves the
existing full-content responses. Content access validates the owning conversation;
round selection also validates the project and plan. Message and event cursors are
independent, so the first event page never needs to load the entire mailbox.

The TUI spools inspection text into private `<state>/inspection-cache/` artifacts.
Small SQLite rows index blocks; source and raw presentation text stay in files.
Background jobs build disk indexes for wrapping and searching. The drawing thread
reads only viewport rows, and at most four layouts per document are cached.
Closing/replacing a document removes its temporary artifacts. Cache sizes do not
limit accessible content. Escape parsing and grapheme wrapping carry state across
chunks, including Unicode clusters and terminal escape sequences.

A direct message is attributed to the consuming turn before its CLI starts and
acknowledged after successful completion. Failed direct task messages remain
queued for work, never for receipt-only dispatch. A successful verifier-only
recheck preserves dependent results; a rejected recheck invalidates the dependent
chain before worker corrections begin. Starting a turn and recording its round
are atomic. Missing attribution is never inferred at startup; unavailable
historical results remain unavailable.


## Guided session workflow

New conversations start in **interview**, including self-contained requests. The
orchestrator clarifies the goal, scope, constraints, exclusions and observable
acceptance criteria, then emits a fenced `requirements` Markdown summary. The
chat displays the requirements summary and automatically starts **planning**.
Pending answers must finish first; newer user input invalidates an older summary.
The transition and its runtime wake-up message are committed together so restart
can resume planning. No requirements approval command is needed.

In **planning**, the orchestrator writes the plan and
an independent verifier checks it. **Ctrl+L** opens the plan document for review
and comments. Submit Review to request revisions and resolve outstanding plan
comments before accepting. **Ctrl+A / `/accept`** approves that plan and moves
the session to **implementation**, where the task graph shows execution and
verification. Draft text is preserved when using shortcuts.

Plan approval is required for new sessions even in autonomous mode. Autonomous
worker progression after plan acceptance continues as configured. Existing
conversations without a workflow row retain their earlier behavior.

The small `session_workflow` row stores the phase, requirements summary and its
message boundary. Transitions append `workflow` events in the same transaction;
replaying events restores the UI after restart or conversation switching.


### Requirements review and solution selection

`/requirements` opens a workspace with a record list,
separate details, search and coverage filters. Wide terminals use two columns;
narrow terminals stack the list and details. Live progress and failures remain
visible while the workspace is open. Arrow keys select records and detail tabs;
`j`/`k` or the mouse wheel scroll details. Drafts survive view changes and errors.

`p` opens a semantic before/after review of a proposed scope change. **Enter**
approves that exact diff and resumes verification and planning; **Esc** returns
without applying it. Approving requirements does not approve implementation.
Approval, the active planning run and its continuation message are committed
atomically. Retrying an approval does not enqueue duplicate work. Startup also
recovers accepted analyst changes saved by older releases without a continuation.
The next stage verifies the approved revision rather than asking an analyst to
rewrite it again. `N` asks before creating an empty draft set; a draft does not
replace the requirements used by current planning. `g` browses sets independently.

Before publishing a plan, the runtime always runs `routing.options`: its executor
compares two to four real implementation alternatives under the same criteria.
Its verifier supplies independent review notes. These notes go to the planner and
the final plan reviewer; they do not create another standalone approval gate.
A measurement-first strategy is valid when evidence needed to choose a concrete
optimization must be collected during execution. The complete plan must resolve
the review notes, including measurement protocols, decision criteria and task
dependencies. The final plan review remains the blocking review before execution.

Comparison-format issues get one correction attempt independently of substantive
plan review. Malformed task-plan JSON is sent back automatically for at most two
serialization corrections, preserving requirements and the cached comparison.
The correction request and its progress event are committed together. Startup
recovers an unfinished format correction, including plans rejected by older
releases; a newer user message takes precedence and repeated failures stop with
the raw error and retry guidance. Both plain-text and structured textual rationales are accepted. The
runtime binds the plan to its active requirements and comparison; agents do not
have to echo run or choice identifiers to advance. Changed requirements or source
coverage invalidate the cached comparison. Previously completed comparisons that
stopped at the old options gate are recovered with their review notes at startup.
This work kind remains mandatory even when optional worker gates are disabled.


### Options before each writing task

The global pre-plan comparison above (`ensure_options`) supplies planning input;
it does not replace task-level `WorkKind::Options` dependencies. When finalizing a
plan, the runtime first adds optional quality gates, then runs `with_options`.
Every task with `writes=true`, including writing quality gates, must have a direct
options dependency. If it lacks one, the runtime inserts a read-only options task
before it, inheriting its scope, original dependencies, and requirement and criterion
links. The writer retains its original dependencies and gains the
new one. An existing direct options dependency is reused; shared options must
cover every dependent writer. Normalization is idempotent and applies even with
`policy.required_gates=false`. Configuration validation at startup always requires
`routing.options` with valid executor and verifier profiles, regardless of that
policy setting.

Each generated options task compares 2–4 distinct realistic variants under common
explicit criteria, excludes variants violating requirements before ranking, and
selects the best-supported available approach with evidence and a condition that
would justify departure. It reads files without implementing changes, inventing
measurements or copying repository contents into its report. Missing evidence and
assumptions must be named. Its complete report must end with exactly one line
`Selected: <id>`, identifying the chosen variant. This is an agent and verifier
contract; the runtime does not parse this line into a structured task decision.

The scheduler waits for approved dependencies. Approved direct dependency reports,
including options, are supplied to the initial worker and verifier prompts. The
selection binds the writer: implement it or explicitly report and justify a
departure with evidence, including any discovered requirement violation. A shared
report that does not cover the writer's task must be reported as a gap. A better
variant or broken premise must be reported openly, without silently changing or
reopening the choice. Such a report does not automatically stop the plan. The
independent verifier compares the actual implementation with the selection and
checks the evidence and justification for any departure or coverage gap; the
worker's declaration alone is not proof.

### Interview question cards

During discovery, the orchestrator emits one fenced `question` JSON block at a
time with `id`, `context`, `question`, and 2–6 `options` (`label`, `description`).
The first option is the recommendation. The TUI displays context above the
question, selectable options with tradeoffs, and an always-available custom
answer field. Agent protocol JSON stays out of the ordinary chat transcript.

Use **Up/Down/Tab** to choose, **Enter** to submit, or type/paste a custom answer.
**Alt+Enter** inserts a newline. **Esc** returns to chat; `/interview` reopens the
pending question. Selection alone sends nothing. Interview answers have their
own drafts per conversation and retain the raw failure reason on send errors.
The question event is durable and restores unanswered cards after a restart.
