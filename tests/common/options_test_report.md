# t6: integration evidence, corrected after verification

Date: 2026-10-22. No production changes; no commit made by this worker.
The runtime committed the first round as `caead2d`; this round tests that HEAD
plus the source changes identified by SHA-256 in the manifests. The comparison
base is the full commit resolved from `3f77db2` in
[metadata](options-evidence/metadata.json), also used by t5. Base was extracted
with `git archive` into a temporary directory; no checkout or Git configuration
was changed. Both versions use the same toolchain, environment and target directory.

## Observed results

| Version | Command | Result | Raw process output |
|---|---|---|---|
| Base | cargo test --test lean_core | 20/20 PASS | [log](options-evidence/base-lean_core.log) |
| Base | cargo test --test lean_tui | 110/110 PASS | [log](options-evidence/base-lean_tui.log) |
| Head + recorded changes | cargo test --test lean_options | 3/3 PASS | [log](options-evidence/head-lean_options.log) |
| Head + recorded changes | cargo test --test lean_core | 20/20 PASS | [log](options-evidence/head-lean_core.log) |
| Head + recorded changes | cargo test --test lean_tui | 110/110 PASS | [log](options-evidence/head-lean_tui.log) |
| Head + recorded changes | cargo fmt --check | PASS | [log, empty stdout/stderr](options-evidence/head-fmt.log) |
| Head + recorded changes | cargo check | PASS | [log](options-evidence/head-check.log) |
| Head + recorded changes | cargo clippy --all-targets -- -D warnings | PASS | [log](options-evidence/head-clippy.log) |

Every log has an adjacent `.json` manifest containing the exact command, working
directory, start timestamp, duration, exit code, log SHA-256, and hashes of source,
test, example, Cargo and toolchain inputs before and after execution. Metadata
includes full base/head commit IDs, the tested tracked diff, runner hash, toolchain
versions and selected nonsensitive environment variables. Logs are combined raw
stdout/stderr captured by subprocess, without rewriting test results.
The reproducible recorder is [record_options_checks.py](record_options_checks.py).

The earlier reported core 13/20 and TUI 101/110 runs had no raw retained logs or
input manifests. Their failures included permission errors and timeouts. The
current base/head comparison does not reproduce them. This supersedes the earlier
unsupported attribution to the environment: the exact historical cause remains
unverified. The permission profile has changed between turns, but that is context,
not a controlled proof of causation. No production fix is justified by these runs.

## Coverage and oracle

- (a), (b): a planner fixture omitting options passes through Core.send and
  accept_plan with required_gates both enabled and disabled. Each writer, including
  generated writing quality gates, has a direct kind=options, writes=false
  dependency; persisted events show options approved before writer working.
- (c), (d): two writers share explicitly declared options without duplication.
  Both receive the report, binding-choice paragraph, and duty to report uncovered
  task scope. Exact task count is asserted only for this deliberately fixed plan.
- (e): identical input yields identical normalization; a normalized persisted plan
  is unchanged by another options normalization.
- (f): the actual options worker prompt requires 2–4 realistic variants, common
  explicit criteria, exclusion of requirement-breaking variants, and Selected line.
- (g): the actual worker and verifier prompts receive the complete options report.
- (h): worker prompts explicitly require evidence/justification for a departure,
  reporting requirement violations, and forbid silent switches. Verifier prompts
  require independent comparison and evidence assessment. The accepting scenario
  has implementation B, a cited repository contract requiring B, and an explicit
  departure report. The rejecting control uses the **same report and implementation**
  but a contract requiring A. This proves acceptance depends on inspected evidence,
  not an exact approved claim string. Preference-only and silent-switch cases
  also remain unapproved.
- (i): all writer reports begin with a deliberately false APPROVED compliance claim.
  Only the verifier's recorded verdict approves a task; verdict approval is asserted
  alongside task status. Contradictory cases have rejecting verdicts and no approved
  event, including retries. The fixture reads actual implementation and contract
  files and parses the report delivered by runtime; it does not use settings.claim
  to decide acceptance. The unused writing variable was removed.
- Gap A cases: absent selection and multiple task-specific selections in shared
  options are forwarded verbatim. These tests document the prompt-based contract;
  they do not claim structural runtime rejection of malformed selection reports.

The deterministic fixture demonstrates runtime plumbing and its controlled
file-based oracle. It does not prove that a real model chooses the best approach
or correctly evaluates arbitrary evidence. Semantic evaluation of contradictions,
selection quality, and justified departures belongs to the real verifier. No
structured selection gate or automatic stop mechanism was added.

## Provenance limits

The recorder controls capture and hashes; fixture agents cannot write these output
logs. It runs locally under the same user as this worker. These artifacts have no
external signature or independent runner attestation and are not presented as such.
An independent verifier can validate hashes and rerun the recorder; this worker
cannot manufacture independent attestation. Historical first-round output cannot
be reconstructed faithfully and was not fabricated.

## Repeat and integrity check

A second head execution also passed: [lean_core 20/20](options-evidence/repeat-head-lean_core.log)
and [lean_tui 110/110](options-evidence/repeat-head-lean_tui.log), with adjacent
command/input manifests. All ten execution manifests have identical before/after
input hashes; every captured log matches its manifest SHA-256. `git diff --check`
also passes. These repeats include every formerly failing test, not just filtered
success cases. They do not establish that historical failures can never recur.
