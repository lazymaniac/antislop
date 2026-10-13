# Architect

Private local Rust prototype for supervised AI-agent orchestration. The product specification is [design.md](design.md). This implementation establishes the runtime and quality-contract foundation; it is not the full MVP.

## Build and verify

Rust 1.98.1 is pinned. The wrapper prefers rustup when an asdf shim shadows the selected toolchain.

```sh
scripts/cargo.sh build
scripts/cargo.sh test --all-targets
scripts/cargo.sh clippy --all-targets -- -D warnings
python3 scripts/requirements.py --check
python3 scripts/audit_requirements.py --check
python3 scripts/requirements_gaps.py --check
python3 -m unittest discover -s tests -p 'test_*.py'
python3 scripts/demo.py
python3 scripts/demo.py --reject
```

The fake demo exercises six stages with independent fixture reviews and explicit approvals, daemon restart/reattach and duplicate requests. Fake results are simulations, never evidence of model quality.

For an authenticated real bugfix in a disposable Git worktree:

```sh
python3 scripts/demo.py --live --repair
```

This uses Codex 0.162.1 through its CLI, consumes provider usage and runs only the fixture's explicit local checks. The demo driver approves its fixed test stages; the daemon itself always enforces supervised approval hashes. No direct provider API is used.

## Diagnostic client

```sh
target/debug/architect daemon .architect
target/debug/architect list .architect
target/debug/architect create .architect examples/fake-spec.json
target/debug/architect snapshot .architect RUN_ID
target/debug/architect events .architect 0
```

The daemon stays active when the diagnostic client disconnects. Run it in your own process supervisor or background job as needed. `Ctrl-C` stops the daemon; interrupted work is marked for reconciliation on restart.

To approve a stage, copy its `scope_hashes.start` from a fresh snapshot and send a versioned request:

```json
{"version":2,"request_id":"user-request-1","idempotency_key":"stage-start-1","operation":{"type":"mutate","command":{"type":"approve_start","run_id":"RUN_ID","scope_hash":"START_HASH"}}}
```

```sh
target/debug/architect request .architect < request.json
```

Snapshots and mutation replies contain summaries. Use `architect export STATE_DIR RUN_ID` for a consistent paginated full document. Protocol 2 rejects older clients explicitly.

Then send `advance` with another idempotency key. After a verified result, `accept_result` needs `scope_hashes.accept`; rejected/blocked attempts need `approve_repair` and `scope_hashes.repair`. `pause`, `resume`, `cancel` and `revise` are explicit commands. Revisions need `expected_revision` and the full new spec. A restart does not grant approvals or blindly replay an uncertain action.

The spec selects `fake` or `codex` explicitly. `input_files` pins existing relative source files; `editable_files` is a subset; `check_command` is an explicitly authorized argv executed locally. Arbitrary untrusted project commands are not sandboxed by this prototype. Do not use it as a general unattended execution environment.

See [stabilization, protocol and recovery details](docs/stabilization.md), [implementation decisions and limitations](docs/implementation.md), [prioritized backlog](docs/backlog.md) and [M0 evidence](docs/m0/probe.json). No public code or binary distribution is authorized.
