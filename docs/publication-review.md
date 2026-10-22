# Publication review — 2026-10-22

Scope: local working tree (including existing uncommitted changes), tracked file
names, all 161 commits reachable from local Git refs and 1,588 historical blobs.
Remote-only refs, GitHub issues, releases, Actions artifacts and external storage
were not inspected. This is a focused publication review, not a complete security audit.

## Findings

- `tests/lean_protocol.rs:847`: recovery validation returns `stale_token`
  where the test expects `not waiting for recovery` for a proposed task.
  This failed both in the complete suite and in isolation. The mismatch needs
  investigation to establish whether the protocol ordering or fixture is wrong;
  this review does not establish a security vulnerability from this assertion.
  Reproduce with `cargo test --locked --test lean_protocol recovery_requests_cross_the_wire_and_only_apply_to_stopped_tasks -- --exact`.

- **Publication readiness: not green.**
  `tests/lean_messages.rs:587` fails in the full suite and when run alone:
  `only_the_comments_the_agent_lists_as_addressed_are_resolved` observes
  `resolved` instead of `open` for the comment not listed as addressed.
  This can silently close unresolved user feedback. Reproduce with
  `cargo test --locked --test lean_messages only_the_comments_the_agent_lists_as_addressed_are_resolved -- --exact`.
  Inspect propagation of addressed comment IDs and follow-up plan turns before
  changing resolution behavior; the exact root cause has not been established.

- `design.md` still exists and is tracked (181,660 bytes). It is a Polish design
  document. Deleting it in a new commit would leave earlier copies in Git history.
- `src/lean/risks.tsv` contains Polish risk descriptions included in the binary
  by `src/lean/risks.rs:10` and supplied to tasks by `src/lean/tasks.rs:831`.
  Polish is therefore present in runtime agent context, beyond Unicode tests.
  Other Polish material includes `docs/parity.md`, design documentation and tests.
- `docs/m0/live-evidence.json` exposes `/Users/sebastian` in captured CLI evidence.
  Historical versions of other files also contain local paths. This is personal
  environment information, not a credential. Sanitizing current evidence alone
  would not remove historical copies; changing captured contents can also make
  their recorded hashes inconsistent.
- Author identities in reachable commits are `lazymaniac` with a GitHub noreply
  address and `antislop@localhost`; no personal email was found in author fields.
- Pattern scans found no confirmed credentials in current files or reachable
  historical blobs. Historical provider-token matches were temporary log names
  containing `task-`; a private-key header was a synthetic redaction unit test.
  Current email matches are test fixtures. No historical filenames matched the
  scanned credential/environment/key/database/log naming patterns.
- No dedicated Gitleaks/TruffleHog executable or cargo-audit/cargo-deny executable
  was available. Custom scans covered private-key headers, common provider token
  formats, quoted credential assignments, emails and home-directory paths.
  Unknown credential formats or unlabelled secrets can evade these scans.
- Cargo dependency license metadata was inspected offline. It includes MIT,
  Apache-2.0, Boost alternatives, Zlib, Unicode licenses and WTFPL; dependency
  terms remain separate from the project's MIT license. No current advisory
  database scan was performed; vulnerability status is unverified.
- Focused code inspection covered CLI permission selection, trusted-directory
  storage, daemon private-directory checks and process cancellation paths.
  No confirmed publication-blocking issue was established in those inspected
  paths. This does not establish security of every execution path.

## Changes

Added the standard MIT license with the existing author handle `lazymaniac`,
Cargo license metadata and a README link. Added environment-file ignore rules
(with `.env.example` allowed) and `.DS_Store`; removed a duplicate ignore entry.
Existing user changes were preserved. No visibility change or history rewrite
was performed.

## Validation

- `cargo fmt --check`: passed.
- `cargo check`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo test --test lean_core --test lean_tui`: 130 passed outside the sandbox.
  The initial sandbox run failed on daemon/process restrictions; the unrestricted
  rerun passed all tests.
- Full `cargo test --locked --all-targets`: failed on the comment-resolution
  test above (29 passed, 1 failed in `lean_messages`); later targets were skipped
  by Cargo. Isolated reproduction also failed.
- Complete `cargo test --locked --all-targets --no-fail-fast`: **579 passed,
  2 failed**. Failing targets: `lean_messages`, `lean_protocol`. Both failing
  tests were independently reproduced. Other targets passed.

Before publication, decide whether the Polish design document, runtime risk
catalog and recorded local path should be public. Removing a file from the latest
revision does not remove it from history. Do not treat this review as assurance
that remote-only content or every possible secret has been checked.

## Subsequent translation

After this review, Polish documentation, the runtime risk catalog, and ordinary
test prose were translated into English. The findings above describe the reviewed
pre-translation snapshot. Unicode character vectors and transliteration mappings
still include Polish letters intentionally; they are technical coverage rather
than product prose. Earlier Polish content remains in Git history.

Translation validation: `cargo fmt --check`, `cargo check --locked`, and
`cargo clippy --locked --all-targets -- -D warnings` passed. The complete
`cargo test --locked --all-targets --no-fail-fast` run finished with 579 passing
and the same two previously recorded failures in `lean_messages` and
`lean_protocol`. All 300 unit tests, 20 `lean_core` tests, and 110 `lean_tui`
tests passed. No additional failures remain after translating test fixtures.
Document structure checks preserved numbered design sections, all source links,
240 risk IDs/order/kinds, parity scenario IDs, and TODO checkbox states.
