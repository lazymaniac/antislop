# Baseline size inventory

## Snapshot

- Worktree: allocated isolated worktree `a6e6d83e-9ec3-4f49-b82f-90ce2ae5fc90`.
- Measured: 2026-10-23T21:01:34Z (UTC; recorded output timestamp).
- HEAD: `9b51fe4d15364017a3cb30a9c8a90e42ab6f8bdb`.
- Start state: clean at HEAD; no staged, tracked, or untracked changes before this remeasurement.
- Changes since context capture: the shared-options context snapshot was `93dc3166a6cb5f9d0bbb4e18a768feacbbfbc639`. The read-only source diff from that commit to this measured snapshot (`9b51fe4d15364017a3cb30a9c8a90e42ab6f8bdb`) changes 21 paths under `src/`, with 2,846 insertions and 520 deletions. This consists of `src/bin/diagnostic-size-check.rs` (+6) and 20 application source paths (+2,840/−520); these changes were already present at measurement time and are part of the measured baseline, not edits made by this inventory task. Separately, this task's diff from its supplied base `3c9973244affbffccfb8d079f714ae9f968f0fba` through the current result contains only the three inventory evidence files listed below, with no `src/` changes. The prior inventory and diagnostic output were already recorded in earlier rounds; this remeasurement refreshed fixture results (temporary-directory names only).
- Raw output SHA-256: `f5ba1e42917f9def5bfdf8a6391ddb51c396cdccd5296f9c115cc7ae250a1619`.
- The earlier shared-options snapshot was HEAD `93dc3166a6cb5f9d0bbb4e18a768feacbbfbc639` at about 2026-10-23 13:45 UTC; it is a different snapshot. Use this report only for the HEAD recorded above.

## Measurement

- Checker: the fixture runner built the checker with the in-worktree Cargo target, passed all fixture assertions, and then invoked the checker at this worktree root. The repository diagnostic exited 1 because of size violations.
- Fixture gate: `CARGO_TARGET_DIR="$PWD/.antislop/target" CARGO_NET_OFFLINE=true python3 tests/size_check/check.py` passed; 76 fixture cases plus the repository diagnostic record (77 lines in `validation-results.txt`). The runner's expected `target/debug/diagnostic-size-check` path was temporarily linked to the in-worktree `.antislop/target/debug/diagnostic-size-check`; that link was removed after capture. Independent inspection after the run confirmed the temporary link was absent.
- Limits: 299 physical lines per maintained file; 29 physical lines per callable.
- Source inventory: 59 Rust files under `src/`; all are maintained production files in this snapshot. The checker treats unreferenced `src` files as production. No file-level `#![cfg(test)]` source and no test-only external module files were found. Classification errors: 0.
- Violations: 38 file-size overages and 342 callable overages (380 total), affecting 48 distinct files. The complete path, line, callable name, measured size, and exit status are in [`diagnostic-run.txt`](diagnostic-run.txt).
- Stale prior record: 381 violations / 0 classification errors. Fresh output is 380 / 0 on this labeled snapshot; one fewer callable overage. `src/lean/daemon.rs` measures 3,252 lines here (prior output: 3,235); callable starts and sizes shifted with source changes. Do not carry prior line counts forward.

### File-size overages

| File | Physical lines | Callable overages in file |
|---|---:|---:|
| `src/config.rs` | 645 | 2 |
| `src/lean/agent.rs` | 1878 | 10 |
| `src/lean/cli.rs` | 569 | 5 |
| `src/lean/daemon.rs` | 3252 | 10 |
| `src/lean/git.rs` | 1066 | 7 |
| `src/lean/history.rs` | 1134 | 8 |
| `src/lean/ledger/runtime.rs` | 628 | 5 |
| `src/lean/ledger/storage.rs` | 1517 | 19 |
| `src/lean/mail.rs` | 414 | 4 |
| `src/lean/mod.rs` | 5762 | 35 |
| `src/lean/native.rs` | 1648 | 10 |
| `src/lean/native/codex.rs` | 1402 | 6 |
| `src/lean/native/opencode.rs` | 1012 | 4 |
| `src/lean/native/opencode/stream.rs` | 577 | 4 |
| `src/lean/rounds.rs` | 1218 | 2 |
| `src/lean/run_context.rs` | 384 | 1 |
| `src/lean/store.rs` | 2982 | 23 |
| `src/lean/tasks.rs` | 3523 | 19 |
| `src/lean/terminal.rs` | 1109 | 8 |
| `src/lean/tui/config.rs` | 748 | 7 |
| `src/lean/tui/details.rs` | 2490 | 12 |
| `src/lean/tui/diffview.rs` | 573 | 5 |
| `src/lean/tui/document.rs` | 861 | 7 |
| `src/lean/tui/editor.rs` | 870 | 6 |
| `src/lean/tui/graph.rs` | 2443 | 10 |
| `src/lean/tui/inspection.rs` | 370 | 4 |
| `src/lean/tui/interview.rs` | 347 | 2 |
| `src/lean/tui/ledger.rs` | 1451 | 10 |
| `src/lean/tui/mod.rs` | 6802 | 44 |
| `src/lean/tui/native_view.rs` | 865 | 4 |
| `src/lean/tui/palette.rs` | 894 | 5 |
| `src/lean/tui/planview.rs` | 1113 | 5 |
| `src/lean/tui/session.rs` | 514 | 3 |
| `src/lean/tui/taskinfo.rs` | 823 | 3 |
| `src/lean/tui/theme.rs` | 696 | 2 |
| `src/lean/tui/unblock.rs` | 360 | 2 |
| `src/lean/tui/view.rs` | 3342 | 7 |
| `src/lean/workflow.rs` | 362 | 3 |

## Exclusions and classification

- No generated-source path was excluded; the checker configures no generated exclusions, so there is no generator provenance to claim.
- No whole file under `src/` was excluded. Rust test sources under `tests/` are outside the checker root by contract: `tests/common/mod.rs`, `tests/lean_core.rs`, `tests/lean_ledger.rs`, `tests/lean_ledger_tui.rs`, `tests/lean_messages.rs`, `tests/lean_options.rs`, `tests/lean_options_delivery.rs`, `tests/lean_options_probe.rs`, `tests/lean_plan.rs`, `tests/lean_plan_history.rs`, `tests/lean_projects.rs`, `tests/lean_protocol.rs`, `tests/lean_recovery.rs`, `tests/lean_review.rs`, `tests/lean_tui.rs`, `tests/lean_workflow.rs`, `tests/lean_writer.rs`, `tests/support/mod.rs`. Their `tests/` location is the test-only scope proof.
- Inline test-only callables and descendants are excluded by `#[test]` or a `#[cfg(...)]` predicate logically implying `test=true`; test-only ancestor context propagates. Mixed production/test files still count every physical line. Fixture evidence: cases 25–37, 47–48, and 51–61 cover cfg implication, ancestor propagation, and external-module handling; cases 26–27 cover test callables.
- No `cfg_attr`, unresolved module/include, parse, or item-position macro classification errors occurred in measured source. Any such error in a later snapshot makes the inventory incomplete and blocks extraction decisions.

## Dependencies and private-test access

- Checker-specific direct dependencies in `Cargo.toml`: `syn = 2` (`full`, `visit`), `quote = 1`, `proc-macro2 = 1` (`span-locations`), and `toml = 1.1.6` (`parse`, `serde`). `toml` reads declared external crates to distinguish external macro calls from maintained macros.
- Production crate direct dependencies in `Cargo.toml`: `base64`, `serde`, `serde_json`, `rusqlite`, `sha2`, `uuid`, `thiserror`, `libc`, `toml`, `ratatui`, `crossterm`, `unicode-width`, `unicode-segmentation`, `toml_edit`, `portable-pty`, `zeughaus-wezterm-term` (renamed `wezterm-term`), `tungstenite`, `ureq`, and `tiny_http`; `tempfile` is a dev dependency.
- Inline `#[cfg(test)]` unit tests compile in the implementation crate and can access private items from their enclosing module. Integration tests under `tests/` compile as external crates and access only public API, using `tests/common` and `tests/support` helpers where needed.

## Independent inspection and limits

- Independent inspection in this round reviewed `tools/size_check/checker.rs` and its fixtures. The AST visitor covers free/nested functions, inherent methods, trait declarations/defaults, and foreign declarations; line counting includes attached prefixes and physical source lines. Test exclusion is based on `#[test]` and symbolic `cfg` implication, with ancestor propagation. Fixtures exercise generated-looking paths as counted source (case 038), module ambiguity/unresolved paths (040–050), `cfg_attr` fail-closed behavior (051–061, 067), parse/include/macro errors (063–066, 073), and callable/counting boundaries. All 76 assertions passed before the measured repository run; the measured source has no classification errors.
- Repository diagnostic intentionally exits 1 for measured size violations; this is not a classification failure. No extraction-order winner is claimed.
- Earlier failed attempt (not used for measurement): `python3 tests/size_check/check.py` started `cargo build --quiet --bin diagnostic-size-check` with Cargo's default target, whose observed rustc output path was `/Users/sebastian/workspace/architect/target/debug/deps` outside this worktree. I interrupted it while Cargo was still building; Python reported `KeyboardInterrupt`. It did not reach a fixture assertion or invoke the repository diagnostic. This was a build-only failure, not a diagnostic result. A previous round recorded a read-only inspection of some artifacts in that external target; their provenance is unknown, and I left them untouched.
- Earlier in-worktree harness failure: the first retry set `CARGO_TARGET_DIR` to `.antislop/target`, but the runner failed before fixture 1 with `FileNotFoundError` because it expects this worktree's `target/debug/diagnostic-size-check`. No inventory was captured by that attempt. The successful run below supplied the expected path with a temporary in-worktree link and removed it afterward.
- Successful isolated run: `CARGO_TARGET_DIR="$PWD/.antislop/target" CARGO_NET_OFFLINE=true python3 tests/size_check/check.py`. Cargo built under this worktree's `.antislop/target`; all 76 fixture assertions passed before the script invoked the repository diagnostic. The runner's expected `target/debug/diagnostic-size-check` path was temporarily linked to the in-worktree binary and the link was removed after capture. This successful run is the sole source for the inventory and is reproducible without the external target artifacts.

### Follow-up inspection of the interrupted build path

- Read-only inspection on 2026-10-23 found the parent target directory and its `debug/deps` directory present. The expected executable `/Users/sebastian/workspace/architect/target/debug/diagnostic-size-check` was absent. A dependency file, `debug/deps/diagnostic_size_check-ed6bde7732bad80b.d` (430 bytes, modified 2026-10-23T22:56:39Z), was present, along with recent `syn`, `quote`, `proc_macro2`, and `toml` dependency artifacts; one inspected `toml` object file was modified 2026-10-23T19:21:19Z. The target directory's `debug/deps` modification time was 22:56:39Z. These timestamps do not establish which process created the files, and the interrupted build cannot be distinguished from other Cargo activity in that shared target directory.
- The expected executable is absent, but dependency artifacts are present and cannot be attributed or safely cleaned from this allocated worktree. A previous round's read-only inspection recorded their names and timestamps; I did not inspect or modify that external target during this remeasurement. The earlier interruption is disclosed above; it is not an inventory measurement and did not run the checker. The inventory's successful, fixture-gated measurement was built and run using only the in-worktree target. External target provenance remains unknown and no external artifacts are needed to reproduce the reported counts.
- `tests/size_check/validation-results.txt` records `381 output lines` for the repository diagnostic because that count includes 380 `VIOLATION` records and the final summary line. The raw diagnostic ends with `0 classification errors, 380 violations`; its SHA-256 remains the value recorded above.

### Follow-up worktree verification

- Read-only Git inspection succeeded on 2026-10-23 using `git -c core.fsmonitor=false -c core.untrackedCache=false ...`; disabling those optional features avoided the earlier attempted Xcode cache write under `/tmp`. Current HEAD is `1d27eecdec8d504ad12d94d5b1d516707052b57a`; the measured source snapshot `9b51fe4d15364017a3cb30a9c8a90e42ab6f8bdb` is its ancestor.
- At inspection, the worktree had no staged, unstaged, or untracked changes. The task-base-to-current-result diff (`3c9973244affbffccfb8d079f714ae9f968f0fba..HEAD`) contains only `tests/size_check/baseline-inventory.md`, `tests/size_check/diagnostic-run.txt`, and `tests/size_check/validation-results.txt`. The context-to-measured-snapshot diff is separately recorded above and includes the inherited `src/` changes. After this follow-up note, only this inventory file is modified. Thus this task adds no production code, while the measured snapshot does include pre-existing source changes relative to the earlier context capture.
- `git diff --check` passed for the base-to-result diff. The earlier verifier's status/diff blocker is resolved; no source or diagnostic measurement was changed in this follow-up.
