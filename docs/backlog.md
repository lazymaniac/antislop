# Prioritized implementation backlog

The authoritative intent remains `design.md`. `requirements.json` retains every nonempty source line with stable IDs, source text/hash, location, a provisional milestone and pending review status; these provisional assignments do not approve technical proposals.

| Priority | Work | State / acceptance |
| --- | --- | --- |
| P0 | Preserve source and risk catalog | Implemented: 1123 source records, 240 risks; `requirements.py --check` verifies no source omission and exact source freshness. |
| P1 | Atomic requirements and independent source audit | Source-preserving interpretation ledger and separate source-first audit implemented. 504 reviewed atoms; 458 parent interpretations still require semantic review. Reports and gap inventory explicitly retain this boundary. |
| P0 | CLI integration spike | Partial. See `m0/probe.json` and session reports. Codex/Claude real structured output and exact-ID resume observed. Linux runtime tests passed separately; Gemini authentication and Linux provider/isolation tests remain open. |
| P0 | License/distribution decision | User selected private code and no public distribution. Locked dependency/license inventory generated; source/legal review gaps remain explicit. |
| P1 | Deterministic local runtime | Implemented prototype. Transactional state/events/deduplication, supervised states, private socket, background workers, snapshot/cursor polling, restart reconciliation block. |
| P1 | Quality contract foundation | Implemented structural checks for all criteria, plan items, common checks, 240 risks, review identity/revisions/evidence, typed context/model outputs. |
| P1 | Complete M2Q | Plan DAG cycles/unknown dependencies and per-risk stage ownership are checked. Source/dependency invalidation and independent hash-bound revalidation implemented. DAG scheduling, complete role-specific contracts and full independent requirements acceptance remain in progress. Structural acceptance cannot prove the substance of a model report. |
| P1 | Real Codex vertical slice | Adapter, controlled edits, checks and disposable worktree demo implemented. Observed complete: `m0/live-demo.json`, six accepted phase reviews, 240 unique risk results, passing boundary tests; raw runner/review evidence is retained in `m0/live-evidence.json`. |
| P0 | Process and partial-edit recovery | Guardian lifeline, descendant cleanup, cross-daemon/nested workspace leases, write-ahead edit receipts and explicit hash-bound reconciliation implemented; regression tests added. Native provider resume/general workspace lifecycle remain open. |
| P1 | Bounded IPC | Protocol 2 rejects duplicate keys, returns small control summaries and paginates hash-bound document reads and run lists. SQLite v1→v2 document-hash migration retains history. |
| P2 | M3 TUI and team communication | Deferred until runtime/quality boundary is complete. |
| P2 | M4 full MVP | Three adapters, profiles/TOML, autonomous, general workspace management, integration, recovery and all task families. |
| P3 | M5 calibration | Compare single CLI, executor/verifier and full process on shared inputs; track defects, time to acceptance and user effort. |
