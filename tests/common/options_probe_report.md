# Options visibility probe — recorded run

Rule written by the runner before launching Cargo (`pre-run.json`):

> PASS iff options sees W1 version or a dependency report yields a decoder that correctly recovers required records [7,11] from actual writer-base file. FAIL iff old checkout and report cannot justify a working decoder. Assess by decoding writer input, never by matching report prose.

The earlier rule was stated in the conversation before the first probe. Its original timing cannot be reconstructed from a source comment; the new measurement has a separate pre-run record. No observed result remains in the test header.

## Inputs and provenance

- Started UTC: 2026-10-22T18:51:13.676763+00:00
- Finished UTC: 2026-10-22T18:51:50.116034+00:00
- Current HEAD: `b133e8e339d443e8e4b89b029c17319ff740415c` plus the test changes fingerprinted below.
- Baseline: `3f77db2e09319328017fd692021ffdb4617a989c`, exported with git archive into ignored runtime state; no checkout or configuration was changed.
- Input hashes before/after measurement: identical.
- Runtime evidence directory: `.antislop/probe-evidence/`.
- stdout and stderr were captured directly from subprocesses by the runner, separately from fixture observations. These are local, editable artifacts, not externally attested logs. Independent verification can rerun the commands; no claim of tamper-proof provenance is made.

```text
rustc 1.98.1 (48a229cea 2026-09-01)
binary: rustc
commit-hash: 48a229ceaefd4985c50990b14116b6d856af0985
commit-date: 2026-09-01
host: aarch64-apple-darwin
release: 1.98.1
LLVM version: 22.1.8
cargo 1.98.1 (797e8a9bc 2026-08-05)
Darwin m4.local 27.0.0 Darwin Kernel Version 27.0.0: Tue Aug 11 21:05:48 PDT 2026; root:xnu-13432.1.9~1/RELEASE_ARM64_T6041 arm64
```

| Input | SHA-256 |
|---|---|
| `Cargo.lock` | `d473ebfd8f661a93ee7ed0312cd29a629cdd5ee1e8211b478b770ae8639c9156` |
| `rust-toolchain.toml` | `85db324345b7937c49ea2ac8b962d741f710719f4f751cab809985cbe6afbe3c` |
| `tests/lean_options_probe.rs` | `95d5b026bec859bd3d12ec253444425c366fa0c59b44e7ee88d1ae0b24a1fa8f` |
| `tests/common/mod.rs` | `be8f5465d6378e29ca863febf9871e4d5ff0ed2218512a53d870d1ec24734ed7` |
| `tests/support/ledger_cli.py` | `9d54156c873cfcd533f95a9f7c8dfb253502d0e86d3a8f49ceaa4fb4637b5313` |
| `src/lean/mod.rs` | `45d9e7e7c874cf92018adf2e98c2e7ad62f6e714766e77a7a252e2d0524c5ffc` |
| `src/lean/tasks.rs` | `4db151b4d718662b8d3c7d901dd71637a74f0623da690ec2a95911077ff01603` |

## Actual probe stdout

```text
probe {"case":"normal-opaque","choice":{"field":"records","format":"json"},"decoded":[7,11],"outcome":"PASS","read":"{\"records\":[7,11]}","report":"done","task":"W2-options","writer_input":"{\"records\":[7,11]}"}
probe {"case":"normal-opaque","choice":{"field":"records","format":"json"},"decoded":[7,11],"outcome":"PASS","read":"{\"records\":[7,11]}","report":"done","task":"gate-options","writer_input":"{\"records\":[7,11]}"}
probe {"case":"unmerged-opaque","choice":{"format":"lines"},"decoded":null,"outcome":"FAIL","read":"7\n11","report":"done","task":"W2-options","writer_input":"{\"records\":[7,11]}"}
probe {"case":"unmerged-opaque","choice":{"format":"lines"},"decoded":null,"outcome":"FAIL","read":"7\n11","report":"done","task":"gate-options","writer_input":"{\"records\":[7,11]}"}
probe {"case":"unmerged-accurate","choice":{"field":"records","format":"json"},"decoded":[7,11],"outcome":"PASS","read":"7\n11","report":"Mode contract: {\"format\":\"json\",\"field\":\"records\"}","task":"W2-options","writer_input":"{\"records\":[7,11]}"}
probe {"case":"unmerged-accurate","choice":{"field":"records","format":"json"},"decoded":[7,11],"outcome":"PASS","read":"7\n11","report":"Mode contract: {\"format\":\"json\",\"field\":\"records\"}","task":"gate-options","writer_input":"{\"records\":[7,11]}"}
probe {"case":"unmerged-wrong","choice":{"format":"lines"},"decoded":null,"outcome":"FAIL","read":"7\n11","report":"W1 evidence: mode.txt now uses v2 structured records; legacy parser is incompatible. Mode contract: {\"format\":\"lines\"}","task":"W2-options","writer_input":"{\"records\":[7,11]}"}
probe {"case":"unmerged-wrong","choice":{"format":"lines"},"decoded":null,"outcome":"FAIL","read":"7\n11","report":"W1 evidence: mode.txt now uses v2 structured records; legacy parser is incompatible. Mode contract: {\"format\":\"lines\"}","task":"gate-options","writer_input":"{\"records\":[7,11]}"}
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.44s
```

Normal execution sends W1 through the real scheduler, CLI, read-only verification, approval, and integration. The test checks its persisted approved round and result commit. Both dependent options see the integrated JSON even with an opaque report.

The three unmerged cases deliberately seed an approved W1 result to reproduce the unmerged state. They are additional visibility fixtures, not evidence that the normal integration path leaves approved W1 unmerged. Both options run through the real runtime; their base/result remain absent. Writers run in isolated worktrees and read W1 JSON. No writer_base call is used for read-only work.

The Rust oracle decodes actual writer input using the options-selected format/field and compares recovered records with required [7,11]. It does not classify prose by substring. The misleading report retains the former magic phrase but selects an incompatible decoder and is classified FAIL. This is a finite machine-readable fixture, not proof of general language-model reasoning or all options requirements (the broader integration evidence belongs to t6).

## Validation and baseline comparison

| Run | Exit | Result | Log SHA-256 |
|---|---:|---|---|
| probe | 0 | test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.44s | `0cd2abf40a4e66c90f24c82c613e213ec46ff5e85c92e10118144dd2ae7435cf` |
| current-core | 0 | test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.15s | `5b869d1ab8a1ea79730e34a97cb0768e589c033f5e1837e481e4a415b0c0c50d` |
| current-tui | 0 | test result: ok. 110 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.64s | `4ebdff3c0949f43bc17a5c747821644ee66c27a3a76645768ee2cff95f578d26` |
| base-core | 0 | test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.12s | `00de71d420118ecdf8bb979910967e2685c9ffddacb91feaafddbc72679ba71d` |
| base-tui | 0 | test result: ok. 110 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.70s | `9e44c328dcb81e6f63e9a0dadfb4fe7a421b2a04615037c65a234307ac01fb04` |
| check | 0 | completed | `ee1d3fffa2022e962e63e0dbe4d35c9c198450a20c29fc09030f635dbb38bfcd` |
| clippy | 0 | completed | `c4a297a6762db9fb3a9f04959b27ae66ba1681979db11828070abc56d33ab896` |
| format | 0 | completed | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |

Commands: `cargo test --test lean_options_probe -- --nocapture`, `cargo test --test lean_core`, `cargo test --test lean_tui`, `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. Baseline runs use the same Cargo target directory and toolchain, with an independently exported source tree.

Earlier restricted execution reported core 13/20 and TUI 101/110, with Operation not permitted, orphan process and timeout messages. Those failures were not baseline-compared then. Current unrestricted execution gives 20/20 and 110/110 for BOTH current and baseline source. Thus this rerun shows no regression; it does not retroactively prove the precise cause of each earlier failure.

## Controlled mutation

In a disposable source copy, replace the selected JSON field with missing. The same oracle rejects the previously passing accurate-report case; the test exits 101. Production and probe sources were not mutated.

```text
  left: false
 right: true
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.96s
```

- Mutant SHA-256: `4897e5c6cd1f13ee8cd8973f3bf8426f95cd205527a2f0f9ae7ae9610891e9f7`
- Mutation log SHA-256: `da57d07b310b145e070a1fcb217ca9a2bd3777c623efeed3b782fa3e2a41e57f`
- Pre-run manifest SHA-256: `dcc8a1f1d14ef04d579b2086c1686d47f07127d8ff89ebe7e4feebf3ec0ef961`

No production fix was made. Possible follow-up: expose a read-only view of dependency results, without assigning writer_base to read-only tasks. A decision about that extension belongs to t7.
