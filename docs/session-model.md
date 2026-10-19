# Session & Plan Model

This document defines the identity contract, storage model, lifecycle, and scope validation for multi-plan conversations in the lean core runtime.

---

## 1. Identity Contract & Hierarchy

Every runtime entity belongs to a strict, unambiguous hierarchy:

```
Project (canonical filesystem path)
  └── Endpoint / Config / State (`core.sock`, `architect.toml`, `<state>/core.db`)
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

1. **Project**: The absolute, canonical path to the repository directory (e.g., `/Users/.../project`). A daemon serves exactly one project.
2. **Endpoint / Config / State**:
   - Unix domain socket: `<state>/core.sock` (or `/tmp/antislop-<uid>/<hash>.sock` when state path is long).
   - Configuration file: `architect.toml` with loaded timestamp for hot reload detection.
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

## 4. Event and Turn Plan Association & Database Migration

### Schema Version
The database schema version is bumped from `3` to `4` (`PRAGMA user_version = 4`).

### Migration 3 → 4
- The migration is non-destructive: existing tables are preserved without data loss.
- Column `plan_id TEXT` is added to `events` and `turns`.
- Indexes `events_by_plan` on `(conversation_id, plan_id, seq)` and `turns_by_plan` on `(conversation_id, plan_id, started_ms)` are created.
- The `plans` table is created and backfilled from existing `tasks` grouped by `(plan_id, conversation_id)`.
- `turns.plan_id` is backfilled where `role` matches `worker:<task_uuid>` or `verifier:<task_uuid>`.
- `events.plan_id` is backfilled from linked `turns` and from payloads containing `plan` or `plan_id`.
- Unassociated entries (such as global messages, conversation setup notices, or events predating plan tracking) explicitly retain `plan_id = NULL` (`None`).

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
