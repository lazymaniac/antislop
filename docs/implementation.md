# Implementation record

This records the initial foundation and historical validation. See [the current implementation audit](development-status.md) for subsequent adapters, TUI, communication integration and the remaining runtime boundaries.

This private local prototype implements the initial runtime and quality-contract foundation from `design.md`. Public distribution is not authorized; `Cargo.toml` sets `publish = false`.

## Decisions

- Modular Rust library and one diagnostic binary; domain rules stay outside IPC/client rendering.
- Pinned Rust 1.98.1, SQLite WAL with FULL synchronization, bundled SQLite for reproducible local builds. Each mutation commits state, event and idempotency response atomically.
- Synchronous threads and bounded provider transport channels suffice for the initial slice. Tokio and Ratatui remain later additions when asynchronous orchestration and the M3 UI need them.
- SQLite through rusqlite was chosen over a custom log/database: transactional state and deduplication are central requirements. Serde/serde_json replace a bespoke protocol parser. RustCrypto SHA-256 identifies pinned inputs/artifacts. UUID v4 supplies process-independent IDs. The lockfile, features and source license files are inventoried by `scripts/dependencies.py`; vulnerability scanning and distribution obligations are separate checks.
- Codex exec JSONL is the first real adapter. Version 0.162.1 supports ignoring user configuration and rules, disabling hooks/plugins/shell/native subagents, read-only sandboxing and structured output. App-server passed a real initialize/thread-start handshake and remains the preferred richer transport. Its configuration isolation requires further testing before promotion. No direct model API or copied authentication store is used.
- Supervised execution only. Every stage has a fresh executor session and a separate verifier session. Every start, repair and accepted result requires a matching scope hash. A diagnostic fixture driver supplies these approvals only for its disposable demonstration.

## Runtime model and scenarios

`awaiting_start → ready → running → awaiting_result → next stage`; the final accepted delivery becomes `completed`. Rejected review becomes `changes_requested`; only explicit repair approval returns it to `ready`. Protocol/provider uncertainty becomes `blocked`. Pause retains the continuation state; cancellation is terminal and never deletes user work.

Invariants: a worker starts only from an approved state; one request key has one payload/result; session identity is durable before proceeding; every executor has an independent reviewer; every required criterion, plan item, common checklist item and all 240 risk IDs must be accounted for; evidence and review revisions must match; unresolved findings or unsuccessful checks block acceptance.

A spec revision preserves history, returns to Discovery and invalidates old approval hashes. Concurrent revision commands use `expected_revision` and reject stale writers. Input sources are read and hashed by the runtime. Discovery receives actual baseline runner evidence. Discovery output requires sourced facts and resolved material unknowns. Planning output requires comparison criteria, distinct options, mandatory-requirement coverage, invariants and success/failure/counterexample simulations. Structural completeness is deterministic; substantive correctness still requires independent review.

A crash after a mutation commit but before its response is handled by persistent idempotency. A crash during a provider attempt becomes `blocked`, preserving the exact provider session ID and any recorded artifacts. The adapter demonstrates native resume by exact ID, but daemon recovery deliberately does not pretend that resuming a conversation reconciles an interrupted filesystem operation. Automatic interrupted-turn recovery remains open.

During Execution the model proposes full-file edits. The runtime permits only explicitly editable, pinned, existing relative paths, rejects symlinks/traversal/duplicates, and compares original hashes before writing. Tests run from explicit argv, never a model-supplied shell string. Failed checks remain evidence and prevent acceptance even if a verifier says pass. Files changed after review cannot be accepted. A runner-created before/after comparison preserves the original source snapshot; earlier successful checks can be cited only when their recorded input snapshot matches the current inputs. Worktrees isolate ownership, not security; arbitrary concurrent hostile filesystem races and arbitrary project test execution need stronger isolation before general use.

## Completion boundary

Working foundation: supervised stages, fake provider, Codex adapter, SQLite/IPC, pause/cancel/reattach, durable approvals, revision checks, complete risk/checklist validation, typed context/planning outputs, controlled file edits and an explicit check runner.

Open before claiming the planned milestones fully complete:

- The source ledger is lossless and stable under line movement; semantic atomization and an independent requirements audit are pending. No automatic source-coverage check is called a semantic audit.
- Runtime tests also passed in a Linux Podman container; native CLI isolation and authenticated provider behavior on Linux remain unverified. Gemini's isolated profile cannot complete a model turn without authentication. Claude/Gemini transports are researched/probed, not production adapters.
- Every risk has one explicit owning phase and executor/verifier pair. A phase reviewer returns its assigned atomic results, and Delivery requires all 240 risks accounted for across the run. Domain-specific role expansion/checklists, task DAG scheduling and incremental evidence-dependency invalidation remain M2Q work.
- Automatic reconciliation/resume of interrupted runtime attempts, a complete provider send/interrupt receipt API, IPC push subscription and stronger process isolation remain open. Event polling is implemented.
- M3/M4 features (Ratatui, three full adapters, autonomous mode, TOML profiles, general worktree lifecycle/in-place integration and all task templates) are not claimed.

## Verification commands

```sh
scripts/cargo.sh fmt --check
scripts/cargo.sh clippy --all-targets -- -D warnings
scripts/cargo.sh test --all-targets
python3 scripts/requirements.py --check
scripts/cargo.sh build
python3 scripts/demo.py
python3 scripts/demo.py --reject
```

Explicit authenticated smoke checks, excluded from ordinary tests:

```sh
scripts/cargo.sh run --example probe_codex
python3 scripts/probe_claude.py
python3 scripts/demo.py --live
```

Reports under `docs/m0/` distinguish observed behavior from advertised/unverified capabilities. No remote CI run is claimed from adding the workflow.

## Primary references

- [Codex app-server](https://learn.chatgpt.com/docs/app-server), [non-interactive mode](https://learn.chatgpt.com/docs/non-interactive-mode), plus the installed CLI help/schema.
- [Claude programmatic use](https://code.claude.com/docs/en/headless), plus installed CLI help.
- [Gemini ACP](https://geminicli.com/docs/cli/acp-mode/), plus installed CLI help.
- [rusqlite](https://docs.rs/rusqlite/0.40.2/rusqlite/), [Serde](https://serde.rs/), [RustCrypto SHA-2](https://docs.rs/sha2/0.11.1/sha2/).


## Observed validation on 2026-10-13

- macOS: 36 Rust tests passed; `cargo fmt --check` and Clippy with `-D warnings` passed.
- Linux aarch64 in Podman: 33 Rust tests passed on Rust 1.98.1, including daemon crash/no-replay and socket ownership. Two later evidence-lineage tests and one JSON duplicate-key test were subsequently added and passed on macOS. Native provider authentication/sandbox lifecycle on Linux is not claimed.
- Both fake E2E paths passed: six-stage completion and seeded rejection requiring repair approval.
- Codex live E2E completed Discovery → Planning → Execution → Validation → Integration → Delivery. Four fixture tests passed, including i32::MIN. Six independent reports account for exactly 240 distinct risk IDs. A controlled verifier interruption required one repair approval; existing stages and approvals survived daemon replacement. The diagnostic driver retried interrupted IPC with the same request/key.
- `m0/live-demo.json` summarizes the accepted run; `m0/live-evidence.json` preserves its exact source snapshots, artifacts, runner output, report items and approvals. Python independently verified every artifact/evidence SHA-256 in the export.
