# Runtime recovery validation

This change implements the approved activation, control, document, UI, and session recovery plan on an isolated branch. Runtime evidence is retained outside the checkout; the primary checkout and existing session branch names are preserved.

## Behavior and evidence

| Area | Invariant | Evidence |
| --- | --- | --- |
| Compatibility | Probe `--version --json` before any internal command; reject incompatible daemon capabilities | Activation operation fixtures, protocol round trips, live handoff integration test |
| Durable activation | Register before validation; typed transitions, request conflict detection, offline read-only status, cancellation and restart recovery | Activation operation fixtures and live handoff integration test |
| Validator lifetime | Durable PID/start/executable plus an ownership marker; verified SIGTERM cleanup, no SIGKILL escalation | Held preflight cancellation and recorded-validator restart regression |
| Agent answers | Atomically commit answer, delivery receipt and question state; validate caller and conversation; replay scheduling after commit | Agent exchange and held-integration delivery regressions |
| Document publication | Stream Git blobs and validate UTF-8/NUL incrementally; lease staging; serialize reference publication with deletion/GC | Content staging, publication-trigger failure, deletion-between-prepare-and-publish, and result document regressions |
| Artifact delivery | Prepare all replacement bytes outside the global scope; revalidate generation, receipt ownership, index identity and user files before publication | Artifact ownership and concurrent editor/symlink/index-lock regressions |
| Document UI | Snapshot metadata only; fetch selected content in chunks; match conversation and selection generations; retain drafts/search | Session API capture, independently selected documents, late-result and failure regressions |
| Branches | Create after confirmed scope, before task allocation; bind title to confirmation; preserve legacy recorded names | Scope-title confirmation and session branch regressions |
| Startup | Restore metadata first, isolate conversation failures, deduplicate repository checks | Startup Git invocation measurement and legacy session fixtures |
| Conflict repair | Repair in the writer worktree; independent reviewer before integration; preserve user changes and pending choices | Existing `lean_writer` conflict, repair, recovery, and variant-selection fixtures |

## Checks

Run from this worktree using the pinned Rust toolchain:

```sh
./cargo.sh fmt --check
./cargo.sh check
./cargo.sh clippy --all-targets -- -D warnings
LC_ALL=en_US.UTF-8 ./cargo.sh test --lib --test lean_core --test lean_tui --test lean_recovery
LC_ALL=en_US.UTF-8 ./cargo.sh test --test lean_writer
```

The final library run passed 893 tests, with two existing ignored tests. The required integration suites passed 25 core, 19 recovery, and 112 TUI tests. All 24 additional writer tests passed. The writer suite validates the existing conflict fixtures; its session lookup now tolerates planning before a session branch exists.

The 128 MiB streaming preparation test measured 6,144,000 bytes maximum resident memory on macOS. The isolated startup test recorded three Git invocations for both one and fifty conversations sharing one repository. The session API capture makes no result-document content request during the snapshot; content requests start after explicit document selection. Rendering reads a cached document order rather than sorting document IDs on each frame.

## Hologram review disposition

`hologram review --root <this-worktree> f2fab44fa --json` was run against this repository, bypassing unrelated Git hooks as approved. The API surface changes are intentional and covered by callers and protocol tests. Its name-similarity suggestions were inspected:

- Activation `cancel_all` operates on validator flags and unscoped activation records; conversation `cancel` also cancels task/plan agents. The shared activation cancellation implementation is reused.
- Activation `draining` and renderer `draw_remaining` have unrelated responsibilities.
- Activation `for_task` queries an operation by task generation; TUI `on_task` projects an event.
- `artifact_file` streams a Git blob; `artifact_files` discovers declared filesystem paths.
- Git `index_path` resolves the filesystem index lock; graph `edge_path` routes canvas edges.
- TUI `append_result_documents` calls the session metadata API; Store `result_documents` implements that API.
- Artifact `PreparedArtifacts.publish` preserves displaced user inodes; configuration publication atomically replaces a validated configuration. They require different publication rules.
- Store document preparation/publication and TUI document navigation cannot share implementations.
- Details `load_documents` installs disk-backed snapshots; Store `plan_document` reads a saved plan.

The old byte-only text classification helper was removed in favor of the shared streaming validation fixtures. Existing artifact and conflict fixtures were extended or reused rather than copied.

## Legacy naming exception

Existing persisted branch names are deliberately accepted without comparing them with the new title-based generator. Allocation still validates the branch syntax, recorded project and expected durable worktree path, common Git repository identity, actual worktree branch, and commit existence. This preserves older names without weakening repository or checkout ownership checks.

## Deployment record

The release executable must be copied to durable runtime storage before live recovery. Record its exact commit, build identity and file digest together with the SQLite Backup API snapshot and checkpoint copy. Verify the live old daemon and child immediately before signaling. Stop the old daemon first, then the verified erroneous preflight child, using SIGTERM without escalation. Classify the preserved technical preflight chat message through the runtime API. Deliver current-state answers through the orchestrator, retain their durable receipts, resume activation through normal task recovery/review, and release deferred tasks only through reviewed plan revision. Operational results belong in the runtime recovery manifest and final report.
