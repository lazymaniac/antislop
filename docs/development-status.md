# Implementation audit and next work

## Reasonless native error steps and commit checkpoint — 2026-10-16

The owned attempt-6 verifier terminated after 944.94 seconds with 117 committed
and native-observed context responses, no accepted review and no correction
event. Its task diagnostic records all four recognized error carriers as absent;
the run-level summary omits that shape. The exact terminal export is retained as
`run-after-error-carrier-verifier-failure.json` in the owned fixture. Original
source/candidate files, previous sessions/turns/artifacts/checks and sibling tasks
were preserved. No native process is still running and no replacement has been
started for the following change.

Exactly CLI 1.4.0 may now continue reading a reasonless DONE error-message step
with only allowlisted metadata, absent/null error carriers and valid duration
and usage shapes. The existing current-UUID, completed-send, advancing-index,
pre-finish and settled-reader guards remain mandatory. A durably committed
`UnclassifiedNativeStep` observation precedes continued reading of the same
process. This identifies an observed rejected step without claiming its cause,
a native retry, a completed review or task acceptance. Unknown fields, malformed
metadata, fatal terminal results and failed observation commits still stop.
Actual fresh finish, exact schema, complete context and clean exit remain
necessary. No input is resent or failed arguments promoted to artifacts.

The final focused regression passed 183 ordinary Rust cases across library,
adapters, diagnostics, ledgers, history and TUI. It includes successful native
fixture continuation and adversarial refusal/cleanup cases. A Darwin test-only
accepted-socket race was corrected by explicitly restoring blocking I/O, matching
production behavior. The broader orchestration/worktree/team regression passed
53 + 4 + 85 cases. Logs: `/tmp/architect-opaque-native-step-regression.log` and
`/tmp/architect-shared-context-team-regression.log`. Genuine continuation after a
reasonless step, complete native task/quality/delivery acceptance and the full
original design remain open.

## Immutable reader ownership and actual context growth — 2026-10-16

Read-only size inspection of the exact three owned ContextAnalyst verifier
prompts found 177,188, 351,280 and 361,202 UTF-8 bytes. The ordinary task JSON
grew from 150,092 to 158,181 to 166,270 bytes; its receipt field grew from
59,422 to 66,959 to 74,496 bytes. The first replacement additionally carries its
explicit handoff. The newest referenced history is not recursively re-embedding
entire prior turns. These are metadata observations of original prepared inputs,
not permission to omit any receipt, history, requirement or captured source.

Transport and reader clones now share an immutable `Arc<Bundle>` owner. A cache
hit requires ownership identity; any copy-on-write or deserialized replacement
must validate again. A live cached reader prevents unique in-place mutation.
Cache eviction cannot leave an attestation for an exclusively mutable old owner.
The serde `rc` feature of the existing dependency preserves the original JSON
representation; no dependency version, native schema, input bytes, bundle/page
hash or required delivery coverage changes. No authority is deserialized from
the cache. Canonical snapshot serialization, persistent CAS/retention/compaction
and duplicate prompt fields remain separate work. No general latency guarantee
is inferred from this ownership change.

87 ordinary Rust cases passed across library, Antigravity/Codex context,
paging/bridge and history. New adversarial tests cover shared ownership,
reopening, same-sized tampering, eviction followed by unique mutation, concurrent
scopes and exact noncanonical external encoding hashes. The actual retained
failed attempt authenticated all twelve original native planning reports, the
unchanged v1 handoff hash and its original full 355,766-byte required coverage
without provider work or canonical mutation (9.40 seconds; not an isolated
performance benchmark). All-target warnings-denied Clippy and locked build passed.
Logs: `/tmp/architect-shared-context-regression.log`,
`/tmp/architect-shared-context-native-compatibility.log`,
`/tmp/architect-shared-context-clippy.log`, `/tmp/architect-shared-context-build.log`.
Broader orchestration/worktree/team regression passed 53 + 4 + 85 cases. The
owned attempt-6 native verifier used its frozen earlier binary and subsequently
terminated as recorded above; no active provider was rebuilt or restarted.
The full original design remains open.

## Native error carriers and preserved failed verification — 2026-10-16

The selected verifier terminated after 844.11 seconds with all 355,766 required
bytes and 91 context responses committed/native-observed, but zero accepted
reviews and zero model-format rejection events. Original source/candidate files,
all earlier turns/sessions/artifacts/checks and sibling tasks stayed unchanged.
The exact failed attempt-5 state is retained as
`run-after-selected-verifier-format-failure.json` in the owned fixture. Its
historical runtime failure is still `Process/unknown`; read-only diagnosis of
only native UUID `7c25dcb2-beb4-43e4-a8da-2b01a52bbc26` found the malformed-call
marker in its native error record. This cannot reconstruct its discarded NDJSON
frame, identify its exact public carrier or prove native retry behavior.

Only native error-message steps now support string `text_delta` alongside the
explicit non-null error fields. Conflicting carriers and malformed delta types
cannot enable correction. Successful artifact text is still never diagnosed as
an error. Unhandled errors include only fixed field/type shape metadata, so the
next actual observation can distinguish missing error content from an unsupported
payload without collecting raw diagnostics or attempted arguments. Independent
fixtures check both supported/older versions, typed fatal errors, unknown text,
null carriers and absence of false completion/secret disclosure.

All 179 distinct ordinary Rust cases passed in one final regression command,
all 42 Python cases passed, and warnings-denied all-target Clippy plus the locked
build passed. Logs: `/tmp/architect-native-error-carriers-final-regression.log`,
`/tmp/architect-recovery-source-audit-tests.log`,
`/tmp/architect-native-error-carriers-clippy.log`,
`/tmp/architect-native-error-carriers-build.log`.
The failed owned process is terminal. Checkpoint-only reconciliation passed in
16.28 seconds without provider work or changes to retained results, native
history, team or original files; the exact reconciled attempt-5 export is
`run-after-native-error-carriers-checkpoint.json`.
The subsequently scoped single-verifier replacement terminated
(`/tmp/architect-native-error-carriers-selected-verifier.log`) as recorded above.
Its approval was acknowledged in 3.870 seconds. Attempt 6 used native
conversation `e81c0a45-6bcc-4cf6-a050-606d12986b75`, managed turn
`cd3a06a1-0503-460e-85a3-00249791131a`. Its target
`/tmp/architect-agy-140-candidate-validation` stayed frozen while live. No check,
executor write or accepted artifact is replayed. The user's separate daemon and
task remain untouched. Native correction, completed task/quality/delivery and
the complete original design are still unproven.

## Recovery source audit and live verifier coverage — 2026-10-16

All thirteen normative source fragments in section 14 now have a separate
hand-authored interpretation: 78 obligations preserving crash recovery, exact-ID
resume or explicit replacement, unknown-effect reconciliation, pause/cancel,
safe retry/backoff, distinct waiting/error states and every no-budget qualifier.
The source-first audit has thirteen separately authored expectation anchors.
A mutation test replaces each obligation while retaining the original source,
authority, identifiers and parent linkage; each missing obligation must fail the
semantic oracle. This improves the interpretation inventory, not implementation
acceptance. Proposals and pending design authority are unchanged.

The current original-source ledger remains 1,146 fragments. It now contains
1,727 interpretation units, 932 reviewed atomic units and 82 semantic anchors;
409 parent interpretations remain open. Package assessment is still provisional:
1,696 partial, four narrow implemented observations and 27 pending. No new
requirement is marked implemented or independently verified. All 42 Python cases
passed (`/tmp/architect-recovery-source-audit-tests.log`). The container spike
documentation also makes explicit that its sampled resource quotas cannot become
hidden product limits under the original MVP contract.

The selected genuine verifier subsequently terminated as described above,
without an accepted task review. A read-only saved observation
authenticated all 355,766/355,766 required index/prepared-prompt UTF-8 bytes,
all twelve original native planning reports and the unchanged v1 handoff hash
(`/tmp/architect-native-format-current-coverage.log`). Full input delivery is not
semantic task acceptance or proof that native model-format correction succeeds.
The native process was not restarted from an observation timeout; its actual
terminal failure was inspected before separately scoped checkpoint recovery.

## Same-session native model-format correction — 2026-10-16

The Antigravity 1.4.0 adapter can keep reading its same supervised process after
a narrowly recognized, indexed native malformed-call rejection. A separate typed
`NativeOutputRejected` event commits the fixed reason and step before continuing;
no attempted arguments or reasoning are retained. Supported-version/current-UUID,
send, strict step advancement, reader settlement and pre-finish guards apply.
Conflicting codes, known fatal prefixes, explicit non-retryable errors, unknown
shapes and other versions still stop. This path neither resends input nor starts
a replacement. A valid new finish, successful terminal output, exact echoed
schema, complete context and clean exit remain necessary. An error or missing
result after correction cannot complete a turn or create a review.

The durable ledger checks scope, session, version and increasing rejection
indices, rolls back failed commits and rejects events after completion or
cancellation. Empty new fields preserve old serialized turn hashes. Existing
lossless history archives retain actual rejection observations. Active-session
progress displays the committed count and pending result; it does not declare
that native retry or verification has succeeded. The two-line graph overlay
keeps incomplete context delivery visible first.

178 distinct ordinary Rust cases passed across adapter, durable sessions,
history, library, model contract, provider/verifier diagnostics, UI and the
native-experiment harness. All 41 Python cases also passed. Adversarial cases cover absent
native initialization, wrong UUID/index, duplicate/old/unindexed steps, known
fatal/non-retryable errors, active reader calls, incomplete context, missing
finish/result, schema drift and nonzero exit. A failed rejection callback stops
the process and delayed descendant without leaking the raw error. Warnings-denied
all-target Clippy, locked build, formatting/diff and all three requirement-ledger
freshness checks passed. Logs:
`/tmp/architect-native-format-correction-regression.log`,
`/tmp/architect-native-format-correction-final-protocol.log`,
`/tmp/architect-native-format-correction-clippy.log`.
Additional logs: `/tmp/architect-native-format-foundation-regression.log`,
`/tmp/architect-native-format-python.log`, `/tmp/architect-native-format-build.log`.
All twelve original native planning reports and the original v1 handoff hash
authenticated again without native work or canonical mutation
(`/tmp/architect-native-format-legacy-compatibility.log`).

The genuine failed three-verifier wave remains preserved. An explicitly selected
verifier-only fixture mode now uses the actual UI palette and frozen confirmation
to request one replacement after checkpoint reconciliation. It retains the
original executor artifact, all previous turns/sessions/checks, sibling tasks and
original four source/candidate files, then stops after that verifier. Actual
native correction and independent review acceptance are still under evaluation;
neither the fixture cases nor a source checkpoint count as native task success.
The actual checkpoint passed in 14.31 seconds with unchanged retained state and
no provider work (`/tmp/architect-native-format-checkpoint.log`). The selected
ContextAnalyst verifier replacement was acknowledged in 3.648 seconds and ran
at attempt 5 in native conversation
`7c25dcb2-beb4-43e4-a8da-2b01a52bbc26`
(`/tmp/architect-native-format-selected-verifier.log`); it subsequently terminated
as described above. A separate user's daemon/session remains
untouched. These single observations are not general latency guarantees.
The complete original design and package statuses remain open.

## Scoped citation reuse, actual session display and malformed native calls — 2026-10-16

The genuine three-verifier wave terminated after 767.11 seconds. All required
prompt/index bytes had been native-observed (179,163 for ContextAnalyst,
148,681 for Orchestrator, 155,187 for RequirementsAnalyst). None produced an
accepted native review; there are still three retained executor artifacts,
zero task reviews and zero quality gates. The run is Blocked at attempt 4 and
requires reconciliation. Both source/candidate directories still contain exactly
the four original files with unchanged bytes. The owned daemon/native processes
have stopped; the user's separate daemon was not modified.

Read-only diagnosis was restricted to the three exact owned native UUIDs. Each
terminal error-message payload contained `Malformed function call`; raw attempted
arguments, planner text and unrelated conversations were not emitted or accepted
as results. The discarded NDJSON frames cannot be reconstructed from these
records. Historical `Process/unknown` failures remain unchanged. New known
code/message diagnostics classify this separately as `Protocol` with fixed
`malformed_function_call`, preserving identity/cancellation and withholding
completion. Static fixtures prove redaction and process cleanup. This does not
yet enable internal retry. The next priority is validating narrowly scoped
same-session model-format correction through the actual CLI, with durable
rejection/progress records and all existing terminal schema/finish/coverage gates.
The official [SDK retry types](https://github.com/google-antigravity/antigravity-sdk-python/blob/main/google/antigravity/types.py)
separate model-output retries from API retries; that API declaration alone does
not establish the CLI's actual recovery behavior. No malformed text is coerced
into a tool invocation, task result or proof.

Citation validation now creates one immutable borrowed-run resolver per model
or provenance walk. It hashes canonical sources once, lazily decodes selected
directory members once and memoizes check bindings by both source reference and
output hash. It retains hashes rather than extra complete source copies. Reader
construction borrows captured directory envelopes without changing any source
body, alias, bundle or approval hash. The resolver has no serializer or cross-run
reuse; a new validation uses the actual new snapshot. Duplicate-alias/conflicting
hash and source/check drift counterexamples still fail. This removes repeated
copy/hash work by construction; no isolated latency bound or persistent CAS claim
is made.

The agent session inspector also uses the latest reserved session for its
displayed task before an older committed agent pointer. Other task/agent scopes
cannot replace that choice. All owned history stays visible and legacy ambiguous
selection stays unknown. This is presentation only. A read-only projection of the
actual failed wave selected all three failed verifier sessions correctly without
rewriting bindings (`/tmp/architect-failed-native-session-ui.log`).

Validation: 296 distinct ordinary Rust cases passed across source/paging,
provenance/core/quality, library, orchestration/instructions, model/history,
native-protocol/diagnostic fixtures and UI/session projection. Native probes
revalidated all 32 context facts and all 12 original planning reports plus the
exact v1 handoff hash; neither probe launched a provider or accepted a task.
Main logs: `/tmp/architect-citation-resolver-regression.log`,
`/tmp/architect-citation-resolver-boundary-regression.log`,
`/tmp/architect-citation-resolver-model-review-regression.log`,
`/tmp/architect-malformed-call-diagnostics.log`,
`/tmp/architect-citation-session-ui-regression.log`,
`/tmp/architect-readonly-output-native-verification.log`.
The complete original design remains open with unchanged package counts.

## Completed read-only output admission — 2026-10-16

A completed native answer can now be recovered when semantic validation or an
execution-checkpoint failure stopped it before any effect binding was recorded.
The scoped `RestoreTaskResult` path accepts only a current read-only task with
zero proposed edits, no existing execution, a complete latest native output,
matching schema/session/provider/configuration, unchanged original sources and
dependencies, and settled required checks/resources. It does not reconstruct
writes or repeat inference/checks. Existing applied-effect restoration keeps its
own authentication and source-refresh requirements.

New executor turns durably record `task-turn-input-v1` before delivery. It binds
the actual invocation's task attempt/contract, specification, agent identity,
dependencies and private view. Configuration remains separately session-pinned;
mode-only history retains its existing rules. Earlier pinned-reader turns can
authenticate the same contract and dependencies from their exact full original
runtime-authored prompt. An old inline turn without either record is refused,
rather than inventing its missing task input. Absence of the new optional fields
preserves all retained legacy serialization and native hashes.

Admission records a distinct `readonly-task-admission-v1` receipt linked to the
exact native turn, artifact and local scoped restoration approval. It is not an
observed-write receipt or quality pass. The artifact remains awaiting its
dedicated verifier; neither stage acceptance nor delivery is granted. UI cards
show the completed session/turn, output hash/narrative and missing effect
checkpoint explicitly. Source observations, preparation, frozen confirmation,
full mutation guards, the workspace lease and transaction/deduplication remain
in force. Failed SQLite admission rolls back the entire operation.

The actual retained native experiment passed guided reconciliation and admission
in 36.00 seconds, with the result mutation acknowledged in 4.397 seconds. The run
was Ready with three artifacts awaiting verification at that checkpoint. The original four source
and candidate files, both earlier artifacts, all native turns/sessions, task
checks, specification revision and run attempt stayed unchanged. No model or
check was launched. Log: `/tmp/architect-readonly-output-owned-admission.log`.
The 4.397-second observation is not a general latency guarantee. All 12 original
native plan reports and the exact old v1 handoff hash authenticated again
(`/tmp/architect-readonly-output-native-compatibility.log`).

Two new executable failure-boundary tests passed admission/idempotency, actual
independent verification, missing/changed pins, contract/schema/attempt drift,
proposed edits, a newer failed turn, uncertain/missing baseline, pause/cancel and
failed SQLite commit without replay. Four existing restoration cases passed.
All four task-worktree cases passed, including the new original-view admission
case with no Git changes or inference. All 85 team cases passed in 469.00 seconds.
The final foundation regression passed 41 library, 53 orchestration, 6 session,
3 history, 4 model-contract and 58 UI cases; all 41 Python cases passed.
All-target warnings-denied Clippy, locked build, format/diff and all three
requirement-ledger freshness checks passed. Main logs:
`/tmp/architect-readonly-output-team-regression.log`,
`/tmp/architect-readonly-output-final-regression.log`,
`/tmp/architect-readonly-output-task-views.log`,
`/tmp/architect-readonly-output-final-clippy.log`.
The next genuine guided verification experiment started from the
retained Ready checkpoint (`/tmp/architect-readonly-output-native-verification.log`).
Its three-action verification wave was acknowledged in 1.268 seconds and then
terminated as described above, without an accepted review. The owned binary is
no longer serving a live run. Complete native task/quality/delivery acceptance,
autonomous progression, production restricted checks, source storage/compaction,
all families/platforms/providers and the full original requirement audit remain
open. The user's separate daemon/session/configuration was not changed.

## Lossless task history and explicit response schema — 2026-10-16

The new owned continuation terminated after 478.46 seconds. Native finish,
schema, completion and all 178,861 required delivery bytes passed; semantic
admission then rejected `context source missing or stale`. The exact output is
retained in its completed native turn, not attached as an accepted artifact.
All 32 cited hashes matched either the original reader documents or actual
baseline-check output. The defect was a split registry: the reader exposed
captured directory members and task prompts exposed check receipts, while model
validation recognized only top-level run sources.

Citation validation now resolves exact typed captured members and authenticated
completed task-check receipts. Opaque ordinary files cannot impersonate directory
captures; live filesystem text, archive aliases, invented commands, missing
receipts, changed outputs/inputs/approvals and stale hashes cannot supply proof.
Canonical source selectors and source/configuration/approval hashes are unchanged.
Provenance records derived citation identity and content hash separately and
rechecks actual source/check bindings instead of trusting cached validity.
This authenticates source identity, not the truth of model claims or successful
task delivery. The actual native result passes structural validation of all
32 sourced facts and eight sections; negative capture/receipt/hash probes fail.
Log: `/tmp/architect-context-citations-native.log`.

Citation/provenance regressions passed: one captured-member adversarial case,
34 core, 6 evidence-graph, 8 quality and 5 independent quality fixtures. The
existing actual expected-failing baseline case now also proves citation support
and refusal after loss of checks/approval, unknown execution state or changed
output/original inputs. Warnings-denied all-target Clippy, locked build and all
requirement-ledger freshness checks passed. All 83 team cases passed again after
this registry change in 466.31 seconds; all 41 Python cases also passed.
Logs: `/tmp/architect-context-citations-provenance.log`,
`/tmp/architect-context-citations-check-regression.log`,
`/tmp/architect-context-citations-clippy.log`,
`/tmp/architect-context-citations-build.log`,
`/tmp/architect-context-citations-team-regression.log`.

The next task-recovery gap at this checkpoint was concrete: this completed read-only native turn
failed validation before an execution/effect binding was admitted. Existing
result restoration requires that binding and native replacement correctly
refuses to replay a completed output. A scoped, source-current admission path for
that retained no-edit result was subsequently implemented and tested above.
The original stopped checkpoint and complete native output remain preserved;
no write or successful verification was fabricated to bypass the gap.

Pinned-reader task replacements now use `task-handoff-v2`. Prior turns are
represented by authenticated `turn-archive-v1` manifests and flat content-addressed
source documents, rather than embedding each old transport bundle again in the
next mandatory prompt. Reconstruction verifies the exact original serialized
turn, including context, original source references, receipts, observed reads,
failure and rejected-output state. The handoff also shows historical completion
and coverage explicitly; none of these become current delivery or approval.
Missing/changed originals and ambiguous aliases fail closed. Tool-free profiles
retain inline v1 history; existing v1 handoff bytes and hashes remain unchanged.
Canonical runs still store inline turn bundles. Persistent CAS storage, complete
source-kind registration, management-history flattening and compaction remain open.

The second actual ContextAnalyst attempt also failed at `finish_component` after
575.08 seconds, despite complete 261,207-byte required context coverage. Its
submitted context again omitted `model` and did not match the task schema. Both
previous sibling artifacts, all native sessions and the original files remain
retained; this is not a successful task. Newly accepted teams and explicitly
approved v2 replacements now select `task-prompt-v2`, which includes the exact
response JSON schema and states the nested context/model/sections wire shape.
The typed output contract and rejection rules are unchanged. Existing historical
inputs are not rewritten and diagnostic candidates never become artifacts.

All 83 team-execution cases passed. The selected regression additionally passed
41 library, 2 bridge, 12 paging, 53 orchestration, 3 history, 4 model-contract and
58 UI cases. The archive cases exercise UTF-8 paging, exact reconstruction,
partial historical delivery, corruption/refusal and repeated flat handoffs.
The final library/live-harness/archive rerun and warnings-denied all-target Clippy
passed. Logs: `/tmp/architect-task-history-team-regression.log`,
`/tmp/architect-task-history-final-regression.log`,
`/tmp/architect-task-history-owned-build.log`,
`/tmp/architect-task-history-clippy.log`.

The owned guided checkpoint then passed in 13.69 seconds with no provider work.
It preserved exact membership/bytes of all four source and candidate files, both
artifacts, the team, session ledger, specification revision and attempt. Log:
`/tmp/architect-task-history-checkpoint-owned.log`. The full design remains open;
these checks establish neither semantic task success nor delivery acceptance.

Read-only reconstruction of the actual owned fixture passed in 7.38 seconds
(7.55 seconds after the final archive-validator refinement):
both original native task turns were reconstructed byte-for-byte; the new
handoff was 122,809 UTF-8 bytes. Unsupported exact-session resume still admitted
no approval, attempt or worker in the disposable copy. All 12 retained native
reports and the original legacy handoff hash authenticated again both before
and after the new format was explicitly installed. The new owned continuation
was acknowledged in 2.413 seconds, with two referenced turns and no inline old
turns. Its required prompt/index coverage is 178,861 bytes versus 261,207 for the
previous attempt, despite adding the full exact response schema. That is a
transport-size observation, not an isolated benchmark or semantic acceptance.
Logs: `/tmp/architect-task-history-native-reconstruction.log`,
`/tmp/architect-task-history-final-native-compatibility.log`,
`/tmp/architect-task-history-post-v2-native-compatibility.log`,
`/tmp/architect-task-history-schema-native.log`.

## Guided interrupted-task recovery and immutable task documents — 2026-10-16

The next priority remains the genuine chat-to-delivery task and its blocking
recovery/UI paths. The full original design remains open. Production restricted
checks/autonomous progression, complete source/context storage and compaction,
all required task families/platforms/providers, calibration and the remaining
semantic requirement review follow; no competency or acceptance gate is removed.

Read-only inspection was restricted to the failed owned Antigravity conversation
`f1cd1a9f-c545-41e8-bd02-edb81fe083c5`. Its last persisted step was a failed
`finish` component with error metadata; the submitted `context` lacked the
required `ContextArtifact.model` and contained fields outside the advertised
task schema. This candidate cannot be accepted as a task result. These retained
data establish an invalid submitted shape; the discarded stdout frame still
prevents identifying the precise historical stream guard. Native reasoning,
raw error text, credentials and unrelated conversation bodies were not emitted.
The diagnostic component JSON is private under the owned fixture root and is
not an artifact, trusted handoff input or canonical output.

The chat approval guide now offers observed effect cohorts before generic
reconciliation, then explicit current checkpoint reconciliation. It waits for
task recovery preparation instead of suggesting an incomplete generic recovery.
After reconciliation, captured checks/results, exact-session continuation,
distinct replacement sessions, eligible verifier retries and read-only source
refreshes have guided confirmation cards. Existing dedicated cards retain exact
scope/session/handoff/effect details across refresh; no approval occurs without
confirmation. Generic reconciliation includes observed source versions and
effect receipts. Unsettled container or source-delivery custody disables this
generic path. Source refresh explains preservation of earlier writes.

The explicitly scoped owned checkpoint experiment passed in 9.82 seconds,
without provider work. It verified exact four-file source/candidate membership
and bytes, no unsettled writes/checks, and retained both artifacts, the complete
native ledger, team, specification revision and attempt. Only the explicit
checkpoint approval/reconciliation evidence was added. Log:
`/tmp/architect-guided-task-checkpoint-owned.log`.

A following read-only UI probe exposed a five-second IPC timeout while building
task recovery. Necessary-condition filters now skip unrelated planned/completed
tasks and already settled checks before full candidate authentication. This
removed the individual-call timeout, but full backend refreshes still cost
9.33 seconds initially and about 6.0 seconds repeatedly. Task recovery now builds
one immutable paginated document asynchronously outside the store lock, with
canonical run bytes plus verified database document identity as its key and
fresh observed source versions before/after publication and reuse. Source drift,
out-of-band canonical changes and late abandoned workers invalidate the view.
The generation-safe cache is shared with the existing sealed management/team
preparation; its authority and full admission checks are unchanged.

Task documents are read-only projections, not mutation proofs. The backend still
checks their full hash, owning run, counts and captured package bindings;
runtime mutations still recompute their current task/source/approval guards.
The current owned probe passed with roughly 3.55-second full backend refreshes,
1.11-second control snapshots and 0.97-second complete 212,490-byte task document
reads (`/tmp/architect-guided-task-boundary-profile.log`). This is a measured
improvement, not a general latency bound or full task acceptance. The UI displays
preparation/unavailability without claiming that a native agent is running.

The guided owned experiment initially exposed another defect: task recovery
advertised exact-ID resume for Antigravity although that adapter does not
implement it. The fixture refused the wrong action before any mutation; SQLite
still held both artifacts and zero replacements. Resume eligibility and direct
admission now use the resolved adapter/profile's declared implementation, with
the model override forwarding that property. Claude/Gemini and Codex read-only
retain their existing implementation; Antigravity, fake and Codex pinned-context
do not advertise it. Actual version/capability probing remains mandatory at
launch and is not replaced by this declaration. No probe/inference is needed to
reject a route which the adapter does not implement.

A six-profile executable matrix uses a nonexistent binary to demonstrate this
without probing. A separate test copied the actual stopped native fixture into
a disposable store and proved that even the former exact scope admits no
resume approval, attempt, event or worker while its valid replacement handoff
and both artifacts remain intact (`/tmp/architect-native-resume-owned-refusal.log`,
6.76 seconds). Actual supported-resume and replacement regressions then passed.

The corrected guided continuation acknowledged its explicit replacement in
2.431 seconds and started one new Antigravity session for the failed read-only
discovery task. It terminated after 575.08 seconds with an invalid finish
component, as detailed above. The previous two artifacts and all original native
sessions are retained. There was no completed task or quality/delivery acceptance
(`/tmp/architect-guided-task-supported-continuation-native.log`).

Validation: 316 distinct ordinary Rust cases, including all 83 team-execution
cases, passed. The three supported native-resume and four replacement tests were
rerun after the capability-admission change. All 41 Python cases, all-target
warnings-denied Clippy, locked build, format/diff and requirement/audit/gap
freshness checks passed. Main logs:
`/tmp/architect-task-projection-final-regression.log`,
`/tmp/architect-task-projection-final-ui.log`,
`/tmp/architect-task-projection-team-regression.log`,
`/tmp/architect-task-projection-boundary-regression.log`,
`/tmp/architect-native-resume-profile-regression.log`,
`/tmp/architect-guided-native-resume-regression.log`,
`/tmp/architect-guided-native-replacement-regression.log`,
`/tmp/architect-task-projection-final-clippy.log`.
The user's separately investigated daemon/session/configuration was not changed.
The full design remains open with unchanged package completion counts.

## Prepared team acceptance and durable check custody — 2026-10-16

The owned native planning review completed all twelve original reports. Its
following `AcceptTeam` exceeded the normal five-second IPC response deadline:
full native report authentication was inside mutation admission under the store
mutex. After that harness exited, read-only SQLite inspection confirmed no
installed team, artifacts or `team_accepted` event, exact original four-file
source/candidate membership and bytes, and twelve completed reports. No uncertain
mutation was replayed. A current read-only compatibility probe authenticated all
twelve against their original prepared inputs in 8.92 seconds, including complete
268,707/268,707 mandatory bytes in the last native context; no provider or state
write (`/tmp/architect-team-acceptance-twelve-native.log`).

Team acceptance now prepares the full authentication outside the store mutex in
the existing asynchronous management projection. An internal, non-deserializable
proof binds the entire canonical run, exact round/scope, validated team and risk
owners. Publication and admission re-observe sources. A canonical change or
source drift invalidates preparation. IPC admits no mutation while preparation
is missing, including a concurrent change after its first readiness check; it
does not fall back to cold synchronous authentication. Direct in-process callers
retain full cold authentication outside the mutex. The scoped local approval,
team validation, event and deduplicated request result still commit together.

The UI displays preparation and offers its explicit team confirmation only after
readiness. It never treats readiness as approval. Executable regressions cover
no cold mutation/provider work, failed SQLite acceptance, exactly-once approval,
canonical report/source drift and the preparation-to-confirmation UI flow. The
first 153 ordinary tests passed, followed by an all-target warnings-denied Clippy
check. The actual owned fixture then acknowledged team installation in 3.372
seconds and the first three-action wave in 1.112 seconds. That native experiment
then stopped after 382.47 seconds with two retained executor artifacts and a
third task's normalization rejection (`phase=normalization; boundary=unclassified`),
not task/quality/delivery acceptance. The reviewed team remains installed;
reconciliation is required. The harness and its owned daemon exited; no mutation
or failed task was replayed. Source and candidate still have their exact original
four-file membership and bytes. Private canonical export:
`run-after-first-task-wave-failure.json` under the same owned fixture root.
Its failed native conversation is `f1cd1a9f-c545-41e8-bd02-edb81fe083c5`, with
39 committed/native-observed reads; these do not prove complete context or task
success. The raw rejecting stdout frame was not retained, so the specific old
normalization guard cannot be established from this diagnostic alone.

Normalization now maps its remaining fixed catalog/step-state/finish-component
guard messages to private allowlisted categories. This improves future diagnosis
without changing admission, retaining raw text or relabeling the old failure.
Executable fixtures cover each category, including secret-bearing invalid state
and component metadata. The CLI also waits on read-only preparation for an
explicit `accept-team --approve-scoped`, preserving the captured document hash
and issuing a single mutation; it never retries an uncertain mutation.

The new canonical check-container journal persists custody before creation and
start, rejects uncertain replay, validates historical identity/transitions and
requires an actual engine removal proof before recording cleanup. Its seven tests
include real SQLite rollback and restart/pause/cancellation handling. Unresolved
custody blocks source-only reconciliation without issuing engine calls. Empty
journals preserve historical native input serialization. This is ordering and
custody support. Read-only exact-name observation now authenticates the returned
full ID/image/ownership label and consumes a sealed observation to recover a lost
create response. Absence remains unsettled; no create/start/removal is issued by
observation. The actual Podman valid/foreign-token observations and four cleanup
scenarios passed in 6.50 seconds, with all owned containers removed
(`/tmp/architect-custody-name-owned-podman.log`). Production restricted-check
selection/controller, recovery admission/UI, sandbox admission, configuration pins
and outcome evidence remain incomplete. See
[restricted runner evidence](restricted-runner.md).

The full design goal remains open; no requirement package is promoted to complete.

Final ordinary validation for this slice: 321 distinct Rust cases passed. The
broad regression has 283 top-level successes (its extra nested subprocess summary
is excluded), with one additional custody observation case, one cold CLI case
and 36 adapter/independent-adapter cases. All 41 Python cases, requirements/audit/
gap freshness, formatting and diff checks passed. Logs:
`/tmp/architect-team-acceptance-final-regression.log`,
`/tmp/architect-custody-name-observation.log`,
`/tmp/architect-team-acceptance-normalization-regression.log`,
`/tmp/architect-team-acceptance-cold-cli.log`,
`/tmp/architect-team-acceptance-python.log`. The initial cold CLI fixture lacked
the daemon's required 0700 state-directory mode; its setup was corrected without
relaxing admission. Its individual rerun passed in 4.41 seconds; all other 52
orchestration cases had already passed on the final CLI implementation. Explicit
native read-only authentication and Podman probes are separate evidence; the
382.47-second native task experiment remains a recorded failure, not part of
the passing ordinary count. The user's separately investigated daemon/session
was not changed.

## Exact Antigravity 1.4.0 admission — 2026-10-16

The installed CLI's move to 1.4.0 had stopped the owned review at preflight.
Before changing the main version gate, an owned source candidate was prepared
from 105 explicitly selected code/fixture files, with a source-hash manifest and
only exact-version/MCP admission plus disabled terminal status reporting changed.
The native executable was copied separately, without authentication stores.
Candidate: `/private/tmp/antislop-agy-140-candidate-9y_k5lzl`;
binary SHA-256 `a4502aaf310b2fd6eb657fcfec2fcc31a690fbbd0c2d14c8eefbb556409024ce`.
The main gate remained unchanged throughout the actual probe.

The new opt-in `tests/agy_version_live.rs` exercises the actual adapter, audited
home, selected configured Antigravity verifier model/effort, durable callbacks,
SQLite reopening and strict native identities/schema/finish/clean exit. On the
exact 1.4.0 copy, inline input at the existing 192,000-byte ceiling retained both
unpredictable end sentinels. A 224,768-byte prepared prompt used 22 committed and
native-observed reader responses, covering all 226,077 mandatory index/prompt
UTF-8 bytes. Both reopened ledgers validated and the executable hash remained
unchanged; the installed binary matched the tested copy. Total native probe:
162.78 seconds (`/tmp/architect-agy-140-owned-contract.log`), report:
`/var/folders/d2/vtn09lpd2717th937yj7smhw0000gn/T/antislop-agy-version-Bdea2Z/report.json`.
This proves the sampled transport/persistence contract, not semantic project
review, full task delivery, OS containment or exact-ID native resume.

Main admission now includes exactly 1.4.0 alongside 1.3.4/1.3.5; pinned-context
admission remains limited to 1.3.5/1.4.0. Its adapter source matches the
native-tested candidate after Rust formatting and updates to version-error text.
All version/help/turn invocations explicitly set `AGY_CLI_OSC7501=off`, using the
control documented in the primary
[1.4.0 release notes](https://github.com/google-antigravity/antigravity-cli/releases/tag/1.4.0).
Inherited arbitrary AGY overrides remain excluded. Unknown future/prerelease
versions, profile downgrade and native-resume fallback remain refused. Provider,
model/effort, user settings and historical input hashes were not rewritten.

The 35-case adapter/independent-verifier regression passed with explicit 1.4.0
success and adversarial matrices for both profiles, forged/incomplete/replayed
context, invalid finish/session/schema/terminal data, nonzero exit, terminal
status suppression and unknown-version/no-resume refusal. A further 87 library,
configuration, session, context, Codex-reader, daemon and live-harness cases
passed: 122 distinct ordinary Rust cases for this slice. Logs:
`/tmp/architect-agy-140-verified-version-matrix.log`,
`/tmp/architect-agy-140-contract-regression.log`. Preliminary matrix failures
exposed two fixture setup mistakes: a one-page input cannot demonstrate missing
later pages, and the independent fixture's lowercase `success` mode deliberately
represents an invalid terminal status. The corrected tests use a genuinely
multi-page source and the existing valid fixture branch; no admission guard was
relaxed. All 41 Python cases, ledger freshness, all-target warnings-denied
Clippy, locked build, formatting and diff checks passed
(`/tmp/architect-agy-140-python.log`, `/tmp/architect-agy-140-verified-clippy.log`,
`/tmp/architect-agy-140-build.log`). The opt-in exact-native experiment is not
added to the ordinary count. A separate current read-only probe authenticated
all ten original native reports in 6.24 seconds without provider launch or
canonical state write (`/tmp/architect-agy-140-retained-prefix.log`).

After the earlier unsupported-version preflight, checkpoint-only reconciliation
passed in 7.22 seconds without provider work
(`/tmp/architect-native-version-checkpoint.log`). The source and candidate still
matched their exact original four-file membership and bytes, with no installed
team. An explicitly scoped replacement for only the unfinished review ran
on production 1.4.0 support
(`/tmp/architect-agy-140-native-review.log`). An intermediate observation retained
eleven of twelve completed reports; turn
`e58c2ef5-727f-425f-a85b-d9244c1e45fb` was running the final part with five
committed/native-observed reads. A fresh read-only canonical snapshot additionally
authenticated all eleven original report bodies in 7.37 seconds, without provider
launch or state mutation (`/tmp/architect-agy-140-eleven-report-prefix.log`).
That then-active final part's mandatory context was still incomplete; it was not
accepted by this prefix probe. The later twelve-report authentication and team
acceptance are recorded above. Full genuine native task/quality/delivery
acceptance remains unproven.

The source-linked inventory now records exact 1.4.0 contract evidence and the
owned container lifecycle support while retaining partial package status.
Counts remain unchanged: 1,146 fragments, 1,678 interpretation units, 854 reviewed
units, 69 semantic anchors, 422 parent reviews open; 1,647 partial, four narrowly
implemented and 27 pending work items. The full original design remains open,
including production restricted-check admission/recovery/evidence,
source/context storage/compaction, all required task families/platforms and
calibration. The user's separately investigated session/configuration was not
changed.

## Owned container cleanup and stream-boundary diagnostics — 2026-10-16

`process::command_with_container` now attaches a runtime-owned, already-created
container to the external guardian. Its binding requires an absolute engine
path, full container ID, exact SHA-256 image ID and UUID invocation ownership
label. On local command exit, cancellation or parent death, the guardian stops
its local process group and confirms removal of that exact container before
releasing inherited workspace leases or reproducing the original exit/signal.
INT/TERM/HUP/QUIT request cleanup; their exec-startup window is protected and
child execution regains normal caught-signal dispositions. Interrupted wait
observations retry. Engine unavailability, bad/malformed/oversized metadata or
foreign ownership cannot count as cleanup: the guardian stays live, holds its
leases and retries with one fixed notice. It projects only ID/image/ownership
from the engine, bounds control replies to 4 KiB, drains output and discards raw
stderr. It never uses a latest/all/prefix/dependency/volume removal selector.

Ten ordinary executable fixtures cover invalid selectors/private-lifeline
admission, exit, cancellation, failed local launch, signal preservation,
parent SIGKILL, guardian termination signals and unavailable/foreign/malformed
engine responses. Negative ownership/metadata fixtures observe repeated failed
probes before restoring the engine, then verify eventual removal and lease
release. The separate actual Podman experiment passed all four scenarios:
exit 7, cancellation, guardian TERM and parent SIGKILL, including children that
create a separate session. Final completion took 4.14 seconds; every owned
container was absent afterward. Log:
`/tmp/architect-container-lifecycle-verified-owned-acceptance.log`.
An intermediate metadata-projection attempt used JSON's `Id` spelling where
the Go template requires `ID`; the real test held the guardian rather than
falsely confirming cleanup. Its failed owned fixture was stopped and removed
only after checking exact guardian/container/image/label ownership; the corrected
projection was rerun against the real engine. The failed log remains separate
(`/tmp/architect-container-lifecycle-final-owned-acceptance.log`).

This is production lifecycle support, not a complete check backend or permission
grant. It is not yet selected by run configuration or project checks. Durable
pre-start ownership/unknown-operation recovery, immutable input materialization,
effective restriction admission, engine/environment/policy pins and authentic
task-termination/evidence bindings remain required. Guardian loss/restart still
needs durable recovery. The existing autonomous project-check refusal remains
unchanged. See [restricted runner](restricted-runner.md).

The prior genuine native review stopped after 701.39 seconds on an older generic
stream ordering/structure/acknowledgement error
(`/tmp/architect-native-bounded-observation-review.log`). It retained ten of
twelve completed reports, with 74 committed/native-observed reader receipts in
failed turn `60106fef-e92e-470c-84ba-1467d73136f0`. Read-only prefix authentication
passed in 6.33 seconds and proved the latest turn's required context was still
incomplete (126,574/298,190 UTF-8 bytes); no final result or team was accepted.
Log: `/tmp/architect-native-ordering-prefix.log`. Only numeric metadata from
owned native session `9ba513cf-7f6b-4eb6-a867-8389d640eb7b` was inspected; it cannot
reconstruct the missing exact stdout frame. The adapter now adds a fixed phase
and an exact allowlisted boundary category for stream-guard errors. Unknown
callback/storage text stays unclassified and is not copied. Fixtures verify
reader replay/scope, init/result/finish/schema ordering and redaction of a private
callback error. No guard was relaxed or old failure retrospectively classified.

Validation passed 234 distinct Rust cases during this slice: the 232-case broad
library/adapter/daemon/management/team/recovery regression and two additional
container cases in the final 51-case affected regression. The broad run preceded
the last container metadata/signal refinements; the final affected run includes
all ten container, ten process, 17 Antigravity adapter and 14 independent verifier
cases. Logs: `/tmp/architect-container-lifecycle-complete-regression.log`,
`/tmp/architect-container-lifecycle-final-regression.log`. All 41 Python cases,
three ledger freshness checks, all-target warnings-denied Clippy, locked build,
formatting and diff checks passed (`/tmp/architect-container-lifecycle-python.log`,
`/tmp/architect-container-lifecycle-verified-clippy.log`,
`/tmp/architect-container-lifecycle-verified-build.log`). The actual container,
native-prefix and checkpoint experiments are separate from ordinary counts.

The owned source/candidate still matched the exact original four-file membership
and bytes. Checkpoint-only reconciliation passed in 16.38 seconds, with no
provider work (`/tmp/architect-native-ordering-checkpoint.log`). An explicitly
scoped replacement for only the unfinished review then stopped after 31.70
seconds at preflight: the installed `agy` now reports 1.4.0, outside the
validated 1.3.4/1.3.5 contract
(`/tmp/architect-native-stream-boundary-review.log`). It did not create a new
logical/native turn; all ten reports and exact source/candidate baselines were
retained. The new release is documented in the primary
[1.4.0 release notes](https://github.com/google-antigravity/antigravity-cli/releases/tag/1.4.0);
those notes and the current headless documentation are not evidence of adapter
compatibility. No allowlist was widened, provider profile downgraded, CLI
installed or global configuration rewritten. The owning experiment is terminal;
the target is no longer serving a paid native job. Next native work must verify
the exact 1.4.0 contract before resuming. Native task/quality/delivery acceptance remains unproven. The user's
separately investigated session/configuration was not changed. The full original
design remains open, including production runner admission/recovery/evidence,
context/storage work and 422 parent semantic reviews.

## Executable sandbox spikes and resilient native observation — 2026-10-16

The §13 backend spike is now reproducible through two provider-free, opt-in
scripts, documented in [restricted-runner.md](restricted-runner.md).
`scripts/probe_restricted_runner.py` builds an owned macOS deny-default policy
with literal path parameters, positive file/socket controls and independent
post-run sentinel checks. Selected reads/scratch writes worked; eight forbidden
file/network operations and a child's outside write returned EPERM. A child
could still call `setsid`, so the profile was rejected with exit 2 rather than
advertised as an autonomous runner. The report explicitly retains untested
resource, system-root, hostile-cleanup and integration obligations.

An already running rootless Podman VM and a local immutable Rust image allowed
the stronger alternative to be exercised without installation, image pull, VM
restart or user-project mounts. `scripts/probe_container_runner.py` requires an
exact local SHA-256 image ID, disables proxy/default environment/image-volume
inheritance and inspects actual restrictions before starting either generated
fixture. Added/writable host binds, extra volumes, weak namespaces/privileges,
devices, published ports and missing configured limits refuse start. The final
owned run passed all sampled controls: actual zero effective capabilities,
no-new-privileges, seccomp, loopback-only networking, 256 MiB/32-process/one-CPU
cgroup limits, unchanged selected/host sentinels, forbidden filesystem/network
attempts and separate passing/failing Rust checks (0/101). A new-session,
new-group child emitted a live heartbeat before container kill; afterward the
container was stopped/exit 137 and its heartbeat stopped. Both token-labelled
containers were removed. Measured client/server versions were 6.1.4/5.8.3;
host Darwin 25.6.0. Final report: `/tmp/architect-container-runner-final-probe.json`.
The narrower macOS report is `/tmp/architect-restricted-runner-probe.json`.

Both reports retain `autonomous_ready: false`. These are fixed owned programs,
not arbitrary project execution or completed security acceptance. Production
policy/image/environment/input/evidence bindings, cancellation/daemon-death
recovery, source-secret selection, engine trust, native Linux-host acceptance,
hostile resource exhaustion and toolchain coverage remain open. Autonomous
project checks still fail the existing unsupported-runner guard. No historical
approval or provider profile was changed.

The preceding paid native review terminated after 779.30 seconds when its harness
returned an IPC WouldBlock error (`/tmp/architect-agy-finish-component-native-review.log`).
It had completed a tenth report before interruption; it did not complete the
eleventh review or a task/team. A separate current read-only probe authenticated
all ten original report bodies against their exact prepared inputs in 6.17
seconds without provider launch or state mutation
(`/tmp/architect-native-ten-report-prefix.log`). The interrupted turn's required
context was incomplete, so no final result was accepted from it. Source and
candidate retained the exact original four-file membership and bytes.
Checkpoint-only reconciliation then passed in 9.70 seconds with no provider work
(`/tmp/architect-native-ipc-checkpoint.log`).

The live harness now permits two retries for transient read-only observation
timeouts/interruption. Every retry restarts a complete hash-pinned document
download; final JSON still passes the strict decoder. Other I/O errors and
invalid final data fail. This path cannot replay scoped mutations or approvals,
and the product's five-second IPC deadlines are unchanged. Four harness cases
and four daemon regression cases passed; the two explicit native/profile probes
remain opt-in. All 41 Python cases, three requirement-ledger freshness checks,
all-target warnings-denied Clippy, locked build, formatting and diff checks passed.
Logs: `/tmp/architect-runner-observation-regression.log`,
`/tmp/architect-restricted-runner-final-python.log`,
`/tmp/architect-runner-observation-clippy.log`,
`/tmp/architect-runner-observation-build.log`. Native prefix/reconciliation and
real sandbox spikes are separate experiments, not ordinary regression counts.

An explicitly scoped replacement is running against the frozen
`/tmp/architect-restricted-runner-validation` binary
(`/tmp/architect-native-bounded-observation-review.log`). At the latest observation
it retained ten of twelve completed review parts, with 23 committed/native-observed
reads in turn `60106fef-e92e-470c-84ba-1467d73136f0`. Do not rebuild that target
while it serves the experiment. This is not full native task/quality/delivery
acceptance. The separately investigated user's session/configuration remains
untouched. The complete original design stays open; P0 proceeds with native
acceptance and production restricted-runner integration. The unchanged source
audit still has 422 parent semantic reviews open.

## Explicit reader calls, native finish variants and security-source review — 2026-10-15

Scoped metadata-only inspection of owned failed native sessions `61d7c4d3-213f-4de5-b13c-fe09405461d1` and `3096fa7c-2784-4750-b7d7-debdf81de85a` found `list_resources` as each last recorded call, with the same server argument that did not match the owned reader. The stored step metadata and result lengths/hashes were collected without tool arguments, response text or reasoning (`/tmp/architect-agy-list-resources-metadata.json`). This suggests the earlier unmapped-frame trigger; it does not reconstruct the missing old stdout frame/state or establish permission for resource discovery.

Fresh Antigravity contexts now use `agy-pinned-context-v2`. The short native prompt supplies an executable first `call_mcp_tool` request, including exact server/tool names, bundle/index hashes, offset and page size; the owned agent profile repeats the same routing guidance. The immutable index supplies document hashes. Source labels and inline-registry aliases are not other MCP servers or resource URIs; resource discovery remains outside the reader contract. Retained agy v1 native inputs and Codex v1 rendering remain exact. A current read-only probe authenticated all nine completed native reports after the change in 5.41 seconds without a provider launch or state write (`/tmp/architect-agy-call-contract-legacy-prefix.log`). A version flip without its matching native input hash fails validation. Correct index access alone still cannot complete the turn.

The owned fixture was reconciled against its unchanged exact four-file source/candidate baselines, then explicitly resumed only its unfinished review (`/tmp/architect-agy-explicit-call-reconciliation.log`, 7.80 seconds, no provider work). That v2 attempt reached native `finish` after 69 committed/native-observed reads, but the former generic tool guard rejected the output component because the CLI encoded it as `step_type=tool`, `tool_name=finish`, `tool_info.name=finish`. It terminated after 548.60 seconds with nine completed reports retained and reconciliation required (`/tmp/architect-agy-explicit-call-native-review.log`). A later read-only probe validated complete delivery of all 298,080 mandatory index/prepared-prompt UTF-8 bytes and reauthenticated all nine original reports in 5.44 seconds (`/tmp/architect-agy-finish-component-prefix-observation.log`). Complete transport delivery is not a completed review or task; no tool-call arguments were promoted to an accepted artifact.

The adapter now recognizes that exact encoding of the already configured finish component as well as the original bare finish event. Conflicting component names, subagent metadata, component errors and malformed parameters remain refused. ACTIVE does not establish completed finish. Component arguments and native `toolAction`/`toolSummary` are data; the later full terminal structured result, exact session/schema, complete context and clean exit remain mandatory. Positive and adversarial executable fixtures cover both profiles, metadata privacy, missing output/terminal data, active-only completion, foreign identity, schema mismatch and nonzero exit. Independent verifier fixtures cover the same terminal obligations separately.

Validation passed 213 distinct Rust cases during this slice: 91 final adapter/context/library/session cases and 122 orchestration/intake/prompt/UI cases preceding the final narrow finish-handler change. Logs: `/tmp/architect-agy-finish-component-final-regression.log`, `/tmp/architect-agy-call-contract-orchestration-ui.log`. All-target warnings-denied Clippy, locked build and the explicit native harness build passed (`/tmp/architect-agy-finish-component-clippy.log`, `/tmp/architect-agy-finish-component-build.log`, `/tmp/architect-agy-finish-component-live-build.log`). All 30 Python cases, three requirement-ledger freshness checks, formatting and diff checks passed. The separate compatibility/owned-native experiments are not added to the ordinary regression count.

Source review additionally splits §9.4 capability dimensions and the five normative §13 security paragraphs into 59 reviewed obligations, preserving the original source and authority. It retains unavailable-capability reporting, interruption-request versus confirmed-stop receipts, protected assets/untrusted inputs, per-run policy, documented provider or external enforcement, explicit weaker-profile selection, prompts/worktrees not being isolation, verifier write restrictions, executable-test restrictions, secret collection/redaction/retention limits and terminal sanitization. The proposed profile list remains a proposal. The audit is still partial: unchanged 1,146 source fragments, 1,678 interpretation units, 854 reviewed units, 69 semantic anchors and 422 parent reviews open; inventory status is 1,647 partial, four narrowly implemented and 27 pending. Interpretation is not implementation or independent product acceptance. Package assessments now reflect already implemented scoped recovery/mode controls while retaining actual native/autonomous/security gaps.

After the finish correction, checkpoint-only reconciliation passed in 6.39 seconds without provider work (`/tmp/architect-agy-finish-component-reconciliation.log`), preserving the nine accepted report bodies and original file sets. A newly scoped replacement is running against the frozen current binary (`/tmp/architect-agy-finish-component-native-review.log`); it has not yet established a completed review/team/task. Do not rebuild `/tmp/architect-ui-document-validation` while it serves that owned experiment. No separately investigated user daemon/session/configuration was changed. The full original design remains open; next P0 remains genuine native review/task/quality/delivery acceptance, restricted autonomous execution and remaining context/storage/control costs, followed by full source/family/platform/calibration acceptance.


## Verified document reuse and continued native review — 2026-10-15

The daemon now captures the complete actual SQLite run bytes and their stored hash in one query, rejects a mismatch and advertises `document_hash_verified: true` only after verification. The frontend retains one immutable downloaded document and reuses it under that verified hash; a changed hash requires a new complete hash-pinned download. Older protocol-2 daemons without this guarantee still trigger a full download on every refresh. A same-length content change, formatting change or switch to another run cannot inherit a decoded view. This removes repeated unchanged transfers and JSON/canonical-run decoding while preserving exact control/document binding.

The locally authenticated management-package proof belongs to that immutable document, rather than a mutable backend slot. It is built through complete native-prefix authentication against that document's canonical run; changed original bytes produce a different owner and no proof. Reuse compares the entire package set and re-observes live sources, including proposed intake captures. Changed package content goes through full authentication and a source error fails the refresh. Runtime admission still compares the complete current canonical run, rechecks sources and requires scoped explicit approval with atomic persistence. No serialized field, IPC package or frontend cache becomes authority.

An alternating old/new frontend measurement against the same new daemon and an owned copy of the genuine seven-report fixture measured four unchanged refreshes each: old 3.1428–3.3186 seconds (mean 3.2408), new 2.3525–2.4543 seconds (mean 2.4205), approximately 25% lower. Initial frontend authentication with a ready daemon still took 7.8155–8.3155 seconds. The normal five-second IPC deadline was unchanged; initial local validation remains on the existing background UI thread. Exact copied/source canonical rows and event/request counts remained unchanged; no approval, provider launch or native session was requested by this profile. Its daemon is terminal. Log: `/tmp/architect-ui-document-owned-profile.log`; private copy: `/var/folders/d2/vtn09lpd2717th937yj7smhw0000gn/T/antislop-decode-x6yc176s`. Earlier decode-only and verified-transfer measurements are retained separately. This fixture measurement does not establish general responsiveness or scalability: daemon canonical serialization, source capture, presentation construction, cold validation and durable storage still need work.

Regression passed 229 distinct Rust cases during this slice: library 35, context pages 12, core 34, daemon four, orchestration 50, TUI 54, TUI sessions three, Antigravity adapter 16, independent Antigravity verifier 13, IPC six and storage two. The 192-case broad regression preceded the final narrow native-diagnostic change; its affected 29 adapter/verifier cases and eight IPC/storage cases passed afterward. Logs: `/tmp/architect-ui-document-final-owned-regression.log`, `/tmp/architect-ui-document-native-diagnostics.log`, `/tmp/architect-ui-document-storage-ipc.log`. New coverage verifies actual request counts for verified reuse and older-daemon fallback, same-length/document/run changes, source drift/restoration, forged package refusal, and an actual daemon/database edit retaining its old hash. All 28 Python cases, three requirement-ledger freshness checks, all-target warnings-denied Clippy, locked build, formatting and diff checks passed (`/tmp/architect-ui-document-python.log`, `/tmp/architect-ui-document-final-diagnostic-clippy.log`, `/tmp/architect-ui-document-final-diagnostic-build.log`). Initial logs retain a missing renamed-method caller, an assertion expecting a later hash-check boundary and a mock directory without required private permissions; final regression corrects the caller/fixtures and preserves the guards and original deadlines.

The owned genuine native experiment was separately reconciled after verifying both the exact original four-file membership and bytes in source and candidate (`/tmp/architect-native-step-reconciliation.log`, 4.95 seconds, no provider work). Its explicitly scoped independent-review replacement continued the retained seven-report prefix and completed two more reports. The tenth reviewer then failed the unknown-step guard after four committed/native-observed reads; the experiment terminated after 1,202.43 seconds. It is blocked with reconciliation required, nine of twelve reports complete and no installed team. The seven original report bodies and exact source/candidate membership and bytes remain unchanged. This is progress through native review, not task/quality/delivery acceptance. Its frozen reference binary predates these UI changes; no binary serving that experiment was rebuilt. Log: `/tmp/architect-native-step-review-replacement.log`; fixture: `/var/folders/d2/vtn09lpd2717th937yj7smhw0000gn/T/antislop-live-team-uZerlu`.

The new failure's type fingerprint exactly matches the JSON string `"unknown"` (`eb8bf0d80db323992f6b634aab492b1e6d9e96a8e87a511c2a0db75ab929452c`). This identifies the CLI's literal unmapped label, not the underlying native operation. Its old frame/state cannot be reconstructed from this diagnostic. A subsequent narrow diagnostic change records that known label, bounded ACTIVE/DONE state and numeric step index while continuing to reject the event. No tool arguments, source/response bodies or reasoning text enter this diagnostic. Unknown ACTIVE events remain refused as well as DONE; an incomplete category cannot establish a safe action or source read. The adapter/verifier regression separately covers these two shapes. A separate current-binary read-only probe authenticated all nine retained native reports against their exact original prepared inputs in 5.27 seconds, without a provider launch or canonical write (`/tmp/architect-ui-document-nine-report-prefix.log`); it is not added to the 229-case ordinary regression count. All owned native/profiling processes are terminal; no separately investigated user daemon/session/configuration was changed.

The full original design remains open. P0 continues with this genuine review/task/quality/delivery path and remaining control/UI costs, followed by restricted autonomous execution, complete contextual-source storage/compaction, task-family/platform acceptance and calibration. The source audit still has 428 parent semantic reviews open; test counts and transport observations do not establish complete compliance.


## Durable source kinds and original directory members — 2026-10-15

New input captures preserve runtime-observed file/directory kinds in `input-observation-v1`, bound to each original body's SHA-256 and UTF-8 length and the complete snapshot hash. Selected subdirectories now expose their original indexed text members and scoped project instructions through the pinned reader without expanding `input_files`. A literal file containing the same JSON as a directory capture remains a file; its payload cannot invent descendants, instructions or write permissions. Capture rejects selected links/special files, uses nonblocking no-follow regular-file reads and validates source stability. Binary directory members remain metadata only.

The type binding follows SQLite persistence, interview/proposed scopes, independent review, accepted inputs, workspace activation, task execution, mediated changes and effect/recovery checks. Snapshot and type updates commit together. Same-byte file/directory replacements invalidate currentness and require explicit revision; reconciliation retains the original pins. Diff-baseline types are independent of a reviewer's candidate input types. Diff reads also normalize absolute selectors, avoid following links/cycles and exclude private runtime state. New instruction indexes use `project-instructions-v2`; fresh orchestration review inputs use `inline-exact-utf8-v9`. Retained v7/v8/native inputs and metadata-free records keep their original encodings. Ambiguous legacy non-root directory-shaped scopes require explicit revision before new work; no historical type is silently fabricated. A new mediated version can bind known legacy root kinds without rewriting its original execution.

Validation passed 378 distinct Rust cases during this slice: 146 current-source core/orchestration/workspace/context/recovery cases, 149 current-source library/adapter/intake/session/UI cases, and the complete 83-case task suite. The task suite completed before the final narrow legacy/no-follow diff changes; those changes subsequently passed the 146/149 groups and three focused wave-reconciliation cases. Seven new input-observation cases cover persistence with live path replacement, envelope spoofing, same-byte type changes, atomic mediated updates, tampering, links/FIFOs and no-follow diff isolation. Logs: `/tmp/architect-source-observation-frozen-regression.log`, `/tmp/architect-source-observation-frozen-adapters-ui.log`, `/tmp/architect-source-observation-complete-team-final.log`, `/tmp/architect-source-observation-frozen-wave.log`. Earlier full-suite failures remain in their logs: an old instruction-format expectation, one delivery timeout under concurrent test load (focused rerun passed with its original deadline), and fault fixtures that restored the old snapshot without its corresponding type binding. Corrected fixtures retain both original pins; production validation was not weakened. The later complete task suite passed all 83 cases.

The read-only compatibility probe authenticated all seven retained genuine native reports against their exact original prepared inputs in 3.89 seconds (`/tmp/architect-source-observation-frozen-legacy-prefix.log`), with no provider launch or canonical write. This is compatibility evidence, not successful model/task acceptance. All 28 Python cases, all-target warnings-denied Clippy and locked build passed; formatting and diff checks are clean. Logs: `/tmp/architect-source-observation-python.log`, `/tmp/architect-source-observation-frozen-clippy.log`, `/tmp/architect-source-observation-final-build.log`. No user daemon/session/configuration was changed and all owned paid native experiments remain terminal.

Full `design.md` completion remains open. Next P0 work includes genuine complete native review/task/quality/delivery acceptance and remaining UI/control latency, followed by restricted autonomous runners, complete contextual-source registration/storage/compaction and outstanding family/platform/calibration obligations. The source audit still has 428 parent semantic reviews open; package statuses remain provisional and must not turn test counts into requirement acceptance.


## Authenticated recovery admission and frontend reuse — 2026-10-15

The prepared management projection now retains an internal authenticated package set, with private fields and no deserializer. Its construction still performs full native-prefix, ledger, proposal, input and scope authentication. It binds the complete canonical run and exact package bytes. Public pending-package JSON cannot create this internal proof or turn an empty set into an approval. Exact canonical changes, including historical context, obligations, diagnostics and reconciliation state, invalidate it. Live input/proposed-source captures are checked again at admission. A restored source may again match; no stale or altered report becomes accepted through this reuse.

Recovery approval reuses that sealed proof only for the exact current canonical state, requested round, kind and scope. Runtime reacquires the store and compares that state before applying any candidate mutation; it still enforces worker exclusion, overlap checks, workspace lease, explicit user approval and atomic persistence. A cold direct-runtime caller authenticates outside the store lock, then follows the same final checks. A cold IPC mutation starts read-only preparation and returns an explicit `no mutation admitted` error rather than waiting beyond the client deadline and later committing unexpectedly. The CLI observes preparation before sending its scoped approval and refuses intervening document changes. Idempotency checks remain before preparation and before actual mutation; duplicate committed requests return their original response without requiring the run to remain recoverable or spawning another native session. Cold failure paths recheck concurrent committed results.

The frontend independently authenticates an arriving package set once. Later refreshes reuse that internal result only if both the entire captured run and the complete package set remain exact, and recheck live sources. Changed package content triggers full authentication and cannot inherit a previous verdict. This avoids rerendering/authenticating every completed native input on every unchanged UI refresh. These results are process-local derived data, omitted from all run persistence; restart rebuilds them. They establish proof continuity, never permission without the explicit scoped command.

An owned copy of the genuine seven-report fixture exercised the actual backend and normal five-second IPC deadline. First ready backend snapshot, including initial local authentication, took 7.9750 seconds on the existing background UI thread; unchanged complete backend snapshots took 3.1733 and 3.1784 seconds. An exact prepared replacement reached its guarded persistence point in 1.7331 seconds, where a deliberately installed SQLite trigger rejected the transaction. Copied run bytes, event/request counts and the original SQLite bytes remained unchanged; no approval, native session or provider launch committed. This is failed-admission rollback/latency evidence, not a successful recovery or paid-model task. Log: `/tmp/architect-authenticated-admission-profile.log`; private copy: `/tmp/antislop-admission-_h14dp9x`. Its reconciliation flag was changed only in that copy to expose the hypothetical read-only scope. The owned daemon is terminal. No original/user daemon, session or configuration was changed.

Final regression passed 206 distinct Rust cases: library 33, context pages 12, core 34, intake 11, orchestration 49, prompt context seven, sessions six and TUI 54 (`/tmp/architect-authenticated-admission-final-regression.log`). The explicit native library probe and two read-only retained-fixture diagnostics remain ignored in ordinary tests. The separate backend profile is not added to that count. New fixture coverage verifies cold refusal without mutation/provider replay, atomic approval-commit failure, subsequent successful native-protocol-fixture recovery and same-key IPC response replay with exactly one new independent reviewer, one recovery record and one approval. Scope/live-source changes and forged-package refusal have separate regressions. All 28 Python tests, three requirement-ledger freshness checks, all-target warnings-denied Clippy, locked build, formatting and diff checks passed (`/tmp/architect-authenticated-admission-python.log`, `/tmp/architect-authenticated-admission-final-clippy.log`, `/tmp/architect-authenticated-admission-build.log`). A preliminary compile failure for the CLI protocol-version import remains in its log; the later regression compiles and exercises the corrected CLI.

Remaining latency includes canonical serialization, filesystem captures, complete UI document construction and initial client authentication; the approximately 3.17-second unchanged refresh is still too expensive to claim general responsiveness/scalability completion. This single retained fixture does not establish worst-case timing or native task quality. The next P0 is those remaining UI/control costs and actual complete native review/task/quality/delivery acceptance, followed by restricted autonomous runners, durable source-kind/compaction/storage semantics and the remaining family/platform/calibration/source-review obligations. The full design remains open, with 428 parent semantic reviews outstanding.


## Asynchronous recovery documents and visible preparation — 2026-10-15

Management recovery projection now runs in a daemon-owned background job. Snapshots report explicit `preparing`, `ready` or `unavailable` state, the completed package count and the hash of the prepared document. No recovery action is offered as ready while validation is pending. Chat explains that retained reports and sources are being checked; source-validation failure has a separate notice. This is runtime preparation, never a claim that a native agent is running. Existing UI input/render handling remains on its separate thread.

The process-local cache binds both the complete canonical `Run` and its database document hash. `Store::get` still checks actual database bytes, so a changed row with an unchanged `state_hash` cannot reuse a validated projection. Concurrent requests share one job for that exact view; an obsolete job cannot publish into a newer generation. Completed review-prefix authentication remains strict. The worker re-observes source captures after authentication, and publication through a new snapshot or initial document read checks live sources again. Source drift removes the offered package; restoring the source can rebuild it. Continuation pages reuse the same immutable bytes/hash while checking the current canonical run binding. Actual recovery admission still performs full independent scope/source validation; a cached document cannot authorize execution or waive proof.

Snapshot state/sequence are captured together under the store lock, then recovery derivation runs outside it. Non-run document derivations also release the lock before expensive work. The first reader receives the prepared document instead of recomputing the whole prefix per page. The frontend additionally checks its hash against the control snapshot and validates all candidate packages together, sharing one immutable authentication view. A direct cold recovery-document read returns an explicit preparation error; clients first poll the snapshot's preparation state. CLI replacement approval remains separately scoped, and the live diagnostic driver waits for the read-only projection before requesting any replacement.

A private backup of the retained genuine seven-report fixture measured: first snapshot 0.3186 seconds with `preparing`; event read during validation 0.0166 seconds; ready after 5.2311 seconds and ten observations; two packages; one 74,687-byte document read in 0.6257 seconds. The normal five-second IPC deadline was unchanged. This smaller document uses the previous slice's v2 references and therefore does not have the old 817,289-byte document hash. Log: `/tmp/architect-async-projection-profile-short.log`; owned copy: `/tmp/antislop-projection-9x3_ifsr`. Only that copy's reconciliation flag was cleared to expose the hypothetical read-only projection; no reconciliation/approval/mutation request or provider launch occurred, and exact original SQLite bytes were checked unchanged. The owned daemon is terminal. The first diagnostic attempt failed before serving because its temporary socket pathname exceeded the platform limit; its separate failure log is retained.

Final validation passed 203 distinct Rust cases: library 31, context pages 12, core 34, intake 11, orchestration 48, prompt context seven, sessions six and TUI 54 (`/tmp/architect-async-projection-final-regression.log`). One explicitly native library probe remains ignored. New coverage verifies immutable document reuse, canonical changes with a retained database hash, live-source invalidation/restoration, store availability during preparation, complete IPC document/package equality, refusal of a stale approval without another native call, and the actual chat notice. All 28 Python cases, three requirement-ledger freshness checks, all-target warnings-denied Clippy, locked build, formatting and diff checks passed. Logs: `/tmp/architect-async-projection-python.log`, `/tmp/architect-async-projection-final-clippy.log`, `/tmp/architect-async-projection-profile-build.log`. The initial Clippy failure was confined to missing imports in the opt-in harness and is retained separately; the final all-target check compiles it successfully.

This closes repeated prefix derivation for recovery-document pages and moves cold projection authentication out of synchronous snapshot requests. Fresh filesystem capture can still be expensive, the UI's local authentication and actual mutation admission remain synchronous, and cache/blob retention needs broader resource/storage work. This is one retained-state measurement, not a universal latency guarantee or a passed native task. The full design, genuine independent quality/delivery acceptance, restricted autonomous execution, source-kind/compaction semantics, remaining task families and platform/calibration coverage stay open; the source audit still has 428 parent semantic reviews outstanding. The user's separately investigated daemon/session/configuration were not changed.


## Recovery projections and referenced history — 2026-10-15

Management recovery now shares one authenticated completed-review prefix, session-ledger validation, source observation and exact proposal validation across the candidate kinds of one immutable borrowed run view. It caches only successful checks within that view; new snapshots/admissions observe sources and authenticate the prefix again. Prompt factoring counts exact repeated token bodies and borrowed JSON slices before hashing retained documents. Original reconstructed bytes, scope hashes, check coverage and native report authentication remain required. A seven-report regression compares shared candidates with independently constructed candidates and refuses a changed last report or changed live input without mutating the run.

Profiling an owned private backup measured a cold recovery snapshot falling from 38.385 to 4.435 seconds, within the unchanged five-second client deadline at that checkpoint. Four recovery-document pages each took approximately 4.1 seconds and reconstructed the exact same 817,289-byte document hash. Those pages still independently derive the same projection (16.385 seconds total). These are retained-state observations before subsequent integration, not a general scaling guarantee. Synchronous cold validation under the store lock and repeated document-page derivation still need an asynchronously prepared, exact-state-bound projection with fresh source validation at publication/admission. The diagnostic-only 45-second observer did not change product timeouts.

New pinned-reader recovery packages use `orchestration-handoff-v2`: the mandatory prompt carries the previous turn's exact source reference, SHA-256, UTF-8 length and explicit historical status. The reader bundle retains its complete original serialized record, including earlier context, receipts and rejected output. Building, delivering and authenticating a recovered input checks the original against the approved reference; unavailable/changed originals or forged references are refused. Only the active decorated recovery receives that historical document. Inline profiles keep the original v1 bytes. This removes recursive history from mandatory prompts; durable bundle/blob deduplication, native compaction and complete contextual-source storage remain open.

Integration also exposed an unversioned change from complete reviewer receipts to manifests. New `inline-exact-utf8-v8` inputs use manifests/receipt projections only with an explicitly configured pinned reader, and expose complete proposed-source captures under their exact document hashes. Manifests do not count as semantic source inspection. Inline profiles retain complete source/receipt bodies; legacy v7 and earlier prepared inputs preserve their original packet and instruction text. A read-only compatibility probe authenticated all seven completed reports from the retained genuine fixture against their exact original prepared inputs without launching a provider or writing canonical state (`/tmp/architect-native-prefix-reference-compatibility.log`). Projection fixtures do not establish model-quality acceptance.

The earlier genuine replacement was admitted after the snapshot optimization, preserving seven completed reports. It then terminated after 64.54 seconds on an unknown native step after two observed reader responses (`/tmp/architect-live-prefix-recovery-replacement.log`); it did not complete the remaining review or install a team. Unknown-step failures now retain a bounded category label, type fingerprint and field-presence flags while excluding payload text. The unknown event remains rejected; neither its precise past type nor successful native task completion is claimed. All owned native experiments are terminal. This implementation does not restart or change the user's separately investigated session.

Final scoped validation passed 228 distinct Rust cases across library, Antigravity adapters/verifiers, core, intake, orchestration, context pages, prompt context, sessions and TUI (`/tmp/architect-recovery-reference-final-complete-regression.log`), plus 28 Python cases and all three requirement-ledger freshness checks. One explicit native library probe remains ignored. The separate read-only seven-report native-prefix probe is not added to the regression count. All-target warnings-denied Clippy, locked build, formatting and diff checks passed (`/tmp/architect-recovery-reference-final-clippy.log`, `/tmp/architect-recovery-reference-final-build.log`). The earlier integration regression stopped on a bootstrap workspace-default mismatch; bootstrap now uses the shared default configuration, retaining the read-only interview contract instead of prematurely selecting task execution worktrees. The final run retains all original assertions and deadlines. Full design completion remains open: native task/quality/delivery acceptance, restricted autonomous execution, asynchronous recovery projection, durable source kinds/context compaction, remaining task-family semantics, provider/platform coverage and calibration. The unchanged source-first audit still has 428 parent semantic reviews open; interpretation counts do not establish product completion.


## Mode history and incomplete planning candidates — 2026-10-15

The former `ChangeMode` mutated the pinned configuration without retaining its predecessor, rebinding an installed team's spec hash or validating the resulting executable contract. A mode change could therefore invalidate retained native/result authentication or bypass the spec's unsupported-runner check. Changes now validate the complete candidate before committing, preserve an explicit local-user mode history, update only the team's configuration/spec binding and use a checked policy-revision increment. A live worker, including a paused worker that still owns its turn, must finish or stop first. Unknown effects still require reconciliation. Unsupported transitions and event-commit failure leave the persisted run unchanged.

History validation reconstructs mode/revision-only transitions back from their exact full configuration pins. Routing, models, capabilities and local permissions cannot change through this history. Completed-session authentication, recorded check proof and accepted-result bindings can recognize only these validated historical configurations; agent/task/role/source/specification bindings remain exact. New reservations and action scopes continue to use the current policy. Native inputs, findings, task results and approval records are not rewritten or retrospectively accepted. Empty history is omitted, preserving legacy run encoding. Positive history/reopen/scoping/team-binding evidence is structural/unit-level; full native acceptance of mode transitions with completed task results remains open.

Project-check admission now has one policy guard for spec checks, proposed task plans and actual planned-check launch. A check declared only in `task.check_plan` cannot bypass the existing autonomous restricted-runner requirement. A real local CLI fixture verifies refusal both with a spec command and with only a proposed research integration check, retains its independently reviewed discovery results and subsequently accepts those same results. Another owned-process fixture proves that a paused live executor cannot have its policy changed and remains cancellable. These are executable protocol fixtures, not paid-model quality acceptance. Restricted autonomous runners, automatic repair/refresh, runtime-policy result acceptance and complete future-only mode semantics remain required.

That regression also exposed a planning-entry defect: the starting template required complete runner coverage before the orchestrator had a chance to choose its checks. The template now checks structural validity while allowing explicit missing proof obligations. The returned native proposal still requires complete criteria/gate/check coverage before independent review or installation. New orchestration context explains this distinction. A fixture with no initial `check_command` can supply a concrete source-assertion argv for its proposed research integration check and reach independent review/installation, with synthetic semantic judgments; leaving coverage empty is rejected after the read-only planner completes, without a reviewer, check invocation or installed team. No fictitious compilation check or completed proof is introduced.

The source-first audit additionally reviews the two mode rows and the two stage/action/mode-change paragraphs in §6.2. It preserves optional combined stage approval, gates for autonomous progression, missing product-decision escalation, no retroactive approval or erased findings, scope invalidation/reassessment and task-only local commits. All 28 Python cases and three ledger freshness checks passed. The source is unchanged: 1,146 fragments, 1,638 interpretation units, 795 reviewed units, 63 semantic anchors and 428 parent reviews still open. Gap status remains 1,607 partial, four implemented and 27 pending; semantic extraction does not establish product acceptance.

The retained genuine correction experiment terminated and failed after 4,051.76 seconds (`/tmp/architect-live-pinned-team-proof-repair.log`). Seven of twelve independent reports completed. Eighth native reviewer `6f13be5e-67e3-4327-b2d9-5a356901a9f4` failed the external-tool/subagent event guard after 84 committed and natively observed reader responses. Its exact offending tool/component is not established by the generic retained diagnostic; the scoped native log did not expose reconstructable tool-event frames. No raw planner/reasoning text was inspected. At that terminal checkpoint, run `724edb6d-cafa-4e6f-8cfa-420a481777aa` was blocked with reconciliation required, round `a59f0d41-afc0-47c6-b307-851014cab0d6` failed, no installed team and all seven completed reports retained. Hashes of the four captured files matched both candidate and original fixture files; that file-only observation alone did not resolve all possible unknown effects.

New Antigravity policy failures retain only bounded known protocol identifiers and presence flags for tool/subagent metadata. Arbitrary tool names are redacted; arguments, source/response text and subagent bodies never enter the diagnostic. The same events remain rejected and the process-group/identity rules are unchanged. Both ordinary and pinned-reader profiles get an accurate "selected profile" diagnostic. Future replacement must first resolve the retained uncertainty, use a new explicitly scoped session for the unfinished review part and preserve the completed prefix; it must not rerun the planner or treat this failed experiment as task acceptance.

Subsequent fixture-only recovery verified actual source membership as well as every immutable baseline file in both candidate and original checkout. The scoped checkpoint-only reconciliation passed in 4.32 seconds, cleared the reconciliation flag and admitted no provider work (`/tmp/architect-live-policy-reconciliation.log`). The seven exact report bodies and native session count remained unchanged. The attempted explicit verifier replacement then terminated after 6.20 seconds with client `Io/WouldBlock` (OS code 35), before any recovery record or new native session was admitted (`/tmp/architect-live-policy-review-replacement.log`). The run is now reconciled but blocked, with the seven-part prefix intact and no installed team. This points to snapshot/recovery IPC latency; the precise expensive operation still needs profiling against this retained real state. No product timeout was raised and no native work was restarted merely because observation timed out. Both live harness processes are terminal; the cache-validation executable is no longer serving an active native experiment.

Final scoped validation passed 332 distinct Rust cases: 220 across library/config/core/orchestration/recovery/session/team/TUI targets, the complete 83-case task target (441.49 seconds), 16 Antigravity adapter cases and 13 independent Antigravity verifier cases. One explicitly native library diagnostic was ignored. Logs: `/tmp/architect-mode-history-regression.log`, `/tmp/architect-mode-history-team-full.log`, `/tmp/architect-native-policy-diagnostic-tests.log`. The 45 orchestration cases passed again after the final fresh-context contract addition (`/tmp/architect-mode-history-fresh-context-regression.log`); that repeat and the focused history cases overlap the 332 and are not added again. Initial focused fixture failures and the test-module-order Clippy failure remain in their separate logs; the final checks retain the actual product assertions and original deadlines.

Final all-target warnings-denied Clippy, locked build, formatting and diff checks passed (`/tmp/architect-mode-policy-final-clippy.log`, `/tmp/architect-mode-policy-final-build.log`). The checkpoint-only reconciliation is a control-path test, not a successful native project task, and is not added to the 332-case regression count. The next concrete P0 is the retained snapshot/recovery latency, followed by the remaining native review and actual task/quality/delivery acceptance.

Priorities remain: complete the retained genuine chat/team/quality/delivery experiment; implement and verify restricted local runners plus autonomous acceptance/repair/refresh; retain durable directory-type provenance and improve context/storage/compaction; finish substantive task-family/template and source-requirement acceptance, then provider/platform coverage and repeated shared-input calibration. The whole original design stays in scope. This slice does not change the user's state, daemon or project configuration.

## Reviewed template selection and specialized starters — 2026-10-15

Fresh orchestration context now advertises all eight design §7.3 templates with their purpose and required competencies. The intake orchestrator can select optional `task.team_template` from the actual interview. The proposal's independent review and frozen approval card retain that selection; accepted `RunSpec` and later team validation bind its family and exact template. The user does not configure each agent or manually select a template. Missing selections preserve existing defaults and serialized contract hashes. This is a reachable selection path, not a claim that every template's substantive specialization is complete.

The refactoring starter assigns each original plan item's transformation to its own Refactorer executor/reviewer pair, preserves plan dependencies and requires validation/integration to follow those transformations. All mandatory software competencies remain present, including implementation assessment. When the spec supplies a real check command, the starter proposes both an expected-success characterization baseline before execution and a separate current check before verification. The historical baseline cannot fulfill a positive current gate; independent reviewers still assess the command's actual behavioral coverage and unchanged contracts. Without a supplied command, no fictitious check is introduced and the execution proof obligation remains unresolved. This does not grant task execution or establish complete refactoring acceptance.

The security-fix starter now separates original threat/reproduction assessment from post-change adversarial review. The first task follows context/requirements discovery, every planning task waits for it, and implementation follows planning. The existing security validation task follows the actual implementation tasks; integration waits for that validation. Both tasks use the dedicated Security analyst/verifier pair and retain source/criterion bindings without write scopes. Descriptions require actual original-state evidence, investigation of unavailable reproduction and explicit missing observations; they do not manufacture a test command or result. A new DAG regression covers this ordering, independent authorship, source scopes, absent fabricated evidence and unchanged ordinary defaults. It first rejected an invalid test spec with editable code but no configured check; the corrected fixture supplies an explicit command without claiming it ran.

Initial final regressions twice exceeded the unchanged five-second per-stage core fixture deadline during late verifier preparation. Profiling identified repeated context rendering/hashing among the costs. Literal factoring now counts exact chunk bodies, discards unprofitable candidates and hashes each retained body once per pass rather than hashing every occurrence twice. Exact original bytes, complete registry reconstruction, SHA-256 bindings and reference semantics remain covered. The normal parallel 34-case core target then passed in 14.02 seconds; the seven exact-context cases also passed. This is an individual regression timing, not a representative performance benchmark.

The broader run exposed a separate startup `BrokenPipe` while probing a daemon just launched by the UI. The owned-startup loop now retries this disconnected read-only probe alongside its existing reset/EOF cases; it never replays a mutation or starts a second daemon. The regression exercises disconnects before and after request read, verifies one child launch and still refuses unexpected socket paths or live protocol/application rejection. The original failed run remains `/tmp/architect-template-final-regression-remaining.log`.

Final scoped validation passed 228 distinct Rust cases across library, core, prompt context, evidence graph, intake, managed workspace, orchestration, recovery, team and TUI targets, with one explicit native diagnostic ignored. Logs: `/tmp/architect-template-core-optimized.log`, `/tmp/architect-template-final-regression-remaining.log` (the passing library/evidence targets only), `/tmp/architect-template-final-regression-rest.log`, `/tmp/architect-security-template-corrected.log` (33 team cases; 32 overlap the earlier team run). The 45 orchestration cases also passed after security specialization in `/tmp/architect-security-template-tests.log`; its subsequent invalid-fixture failure remains recorded. Fixtures prove protocol/effect behavior; they do not substitute for native model acceptance. All 27 Python tests and the three requirement-ledger freshness checks passed. The twenty §7.2 role-catalog parents now have source-grounded semantic interpretations that preserve conditional specialist selection and illustrative artifact examples. Counts are 1,146 source fragments, 1,622 interpretation units, 766 reviewed units and 432 parent reviews still open. The implementation-gap ledger remains 1,591 partial, four implemented and 27 pending; interpretation work does not establish implementation completion.

The retained real-provider correction remains running in its separate executable, with five of twelve review outputs committed at this observation. No installed team or completed native task is claimed. Full design completion, all template specializations and the previously recorded P0/P1/P2 work remain open. The user's daemon/state/configuration were not changed by this slice.

Final warnings-denied all-target Clippy and the locked build passed in the cache-validation target (`/tmp/architect-specialized-template-clippy.log`, `/tmp/architect-specialized-template-build.log`), with formatting/diff checks clean. The dedicated live experiment executable was not rebuilt.

## Defect proof versus process acceptance, and native proposal repair — 2026-10-15

The first fresh reader-backed experiment terminated after 872.35 seconds with a retained, structurally rejected 30-agent/15-task proposal, not a passed task. In round `bf2913ba-6a5b-4392-a041-6ec29aa4c9d4`, the implementation bugfix listed all nine assigned criteria while its original-state check covered four. The five missing mappings included the boundary criterion and process/preservation obligations. Rejecting that inconsistent proposal was correct under its declared contract. It also exposed the need to distinguish an actual defect from other acceptance obligations rather than imply that reproduction proves approvals or final delivery. No team was installed or task file changed. Original log: `/tmp/architect-live-pinned-team.log`; native planner `01a13dda-b6d8-70f2-a3bf-1634f54f94aa` completed and its full output remains retained.

Bugfix contracts now support explicit `non_defect_criteria` rationales for assigned criteria that do not describe an original defect. For a pure bugfix, defect and non-defect sets are disjoint and together account for every assigned criterion. Every defect still requires both its original-state and post-change check mappings. Each additional classification has a mandatory `NON_DEFECT_PLAN` review check and a task-verifier `non_defect_assessments` entry with its own current inspected evidence and passing `CHANGE:non_defect` check. This classifies the obligation; it does not fulfill or remove it. Ordinary requirement/plan/check/gate acceptance remains mandatory. Missing, foreign, overlapping, unexplained or independently unreviewed classifications fail closed. Empty additive maps are omitted from serialization, preserving legacy contract hashes and the original strict behavior. Diagnostics now identify the task, defect, before/after check, actual phase/exit and exact missing criterion IDs. Fresh management contexts explain the complete contract; old frozen contexts are not rewritten.

The preserved run `724edb6d-cafa-4e6f-8cfa-420a481777aa` resumed through a new addressed correction, not replay of an uncertain turn. Codex native session `01a13deb-913d-7cc3-a0f3-d0ea5fbbfbe1` returned a version-bound amendment retaining tasks, checks, scopes, providers and all 240 risk allocations. Correction round `a59f0d41-afc0-47c6-b307-851014cab0d6` passed structural validation and entered twelve independent review parts; first native reviewer `c7ff4f64-40be-4030-a312-a03eb89c0f51`. The amendment's classifications still need substantive independent assessment; no team, task result or delivery is accepted yet. The live process/log is `/tmp/architect-live-pinned-team-proof-repair.log`, using the dedicated `/tmp/architect-codex-instructions-validation` executable. Do not rebuild that executable while it is live. The current SQLite record, rather than an older stopped `run.json` export, is authoritative during execution.

The executable defect-proof regressions passed positive before/after and alternative cases, rejected omitted/unbound proof, and demonstrated a verified task whose non-defect criterion retains both its ordinary acceptance check and independent classification evidence. Baseline failure remains separately retained and cannot supply a successful current runner gate. Logs: `/tmp/architect-non-defect-proof-tests.log` (three task cases plus one overlapping plan unit case), `/tmp/architect-non-defect-acceptance.log` (one additional task case), `/tmp/architect-non-defect-regression.log` (135 library/integration cases). The complete 80-case task target subsequently passed in 429.59 seconds in the separate cache-validation target (`/tmp/architect-non-defect-all-team-tests.log`), covering execution, recovery, source refresh and separately approved text/file-operation delivery. Together with the 135-case focused regression this is 215 distinct reported Rust passes; the earlier filtered cases overlap and are not added again. Warnings-denied all-target Clippy, locked build, formatting and diff checks passed. This is protocol/effect validation with synthetic fixture judgments, not proof of real-model task completion.

Source-first interpretation now additionally covers 16 compound parents in team selection/base/template constraints and actual before/after bugfix proof. The full source remains unchanged. Reviewed interpretations retain preparatory-analysis exceptions, all mandatory competencies, dedicated reviewers, small-task allowances, nonwaivable gates, finite task-scoped conversations, no numerical MVP agent/round limits, justified team changes and their history. The ledger has 1,539 interpretation units, 663 reviewed units, 39 semantic anchors and 452 parent reviews still open. All 25 Python cases and three freshness checks passed; extraction still grants no implementation/verification status (`/tmp/architect-team-proof-audit-tests.log`). The broad design, native task/quality/delivery outcome, source/type/compaction work and remaining priorities remain open.

## Exact-bundle reader reuse and fresh genuine team experiment — 2026-10-15

The reader now reuses its validated immutable representation in memory instead of repeatedly encoding, decoding and hashing the same full source bundle for successive page checks. Each lookup compares the complete bundle, including all original bytes and scope; mutations rebuild/revalidate it. The derived cache is absent from persistence/equality and rejected as an input field. Shared clones cannot reuse a reader from another scope. Page authentication, native input/hash bindings, complete delivery and SQLite acknowledgement remain unchanged. New regressions cover same-length body tampering, valid re-keyed body replacement, references, scope, length/key changes, concurrent cloned scopes and reopening without a cache. Canonical run snapshot serialization/storage and compaction still need work.

Focused Rust validation passed 146 distinct cases across the library and nine integration targets, including the ordinary native-adapter fixtures, source pages/bridge, session/restart bindings, full orchestration, task contracts and project instructions. The four-case Codex target includes a real process/SQLite fixture above the inline scalar ceiling: its whole-target time changed from 54.75 seconds in the previous run to 9.63 seconds (10.49 seconds on the subsequent regression). These are individual local regression timings, not a calibrated benchmark or general speed guarantee. Logs: `/tmp/architect-context-cache-tests.log`, `/tmp/architect-context-cache-unit.log`, `/tmp/architect-context-cache-regression.log`. All-target warnings-denied Clippy and the locked build passed in a separate target directory (`/tmp/architect-context-cache-final-clippy.log`, `/tmp/architect-context-cache-build.log`), preserving the executable used by the running native experiment.

The fresh ignored `live_team` experiment can explicitly choose both reader profiles in its disposable fixture using `ANTISLOP_LIVE_PINNED_CONTEXT=1`. This does not edit the user's configuration or upgrade a retained experiment. A new genuine run `724edb6d-cafa-4e6f-8cfa-420a481777aa` at `/var/folders/d2/vtn09lpd2717th937yj7smhw0000gn/T/antislop-live-team-uZerlu` has completed the actual Codex interview (`01a13dd3-4761-75d1-9034-22a8d6301b86`) and independent Antigravity contract review (`ae4a8966-c5c9-427e-8022-4b9cf856b37e`, 23 checks, no findings). Real native reads include every fixture file through the member index. The IPC/TUI driver accepted the reviewed contract, approved preparation and activated its private worktree in revision 3; its source checkout remains unchanged. Team planning is running, not accepted/completed: native Codex session `01a13dda-b6d8-70f2-a3bf-1634f54f94aa`; log `/tmp/architect-live-pinned-team.log`. The process must be observed to terminal state before calling this experiment passed or failed. It runs the already built pre-cache executable; the cache has separate offline coverage above.

Audit of design §§7.2–7.3 confirms that the baseline software team is intentionally 15 competencies with dedicated pairs (30 agents), including orchestration. Small-change artifacts may be short, but these mandatory competencies cannot be removed to ease native acceptance. Context-driven extra specialists, task partitioning, source-grounded adequacy and the full real task/quality/delivery outcome still require evidence. The whole design and existing P0/P1/P2 work remain in scope.

## Codex immutable reader and original interview/member sources — 2026-10-15

Codex CLI 0.162.1 now admits explicitly selected `pinned_context` through the same daemon-owned reader and durable turn checkpoints. It ignores user configuration/rules, registers only the invocation's required `read_context` MCP server and preserves the existing disabled shell/apps/hooks/plugins/subagent controls. Its distinct transport/native-input marker prevents cross-provider reuse. Matching native start/completion identities, arguments, the full echoed response and an actual committed backend receipt are all required before recording an observation. Incomplete final output is retained as rejected; failed commits and cancellation during preparation release no unapproved context. This is not a proven general tool-free allowlist or OS sandbox. Exact-ID native resume for this new profile is explicitly unsupported; the existing separately authorized replacement path retains history. Existing profiles, the user's configuration and daemon/state remain unchanged.

New interviews preserve complete pre-contract observations outside their bounded model packet, with an immutable context hash, so a reader-backed orchestrator can access original bodies before choosing task inputs or a team. Scope/authorship validation rejects altered retained sources. Project-root directory captures additionally expose an index and individual original text documents, retaining the parent envelope/hash and member metadata. No input selector or edit grant is expanded, no newer live body is reread, and links/binary/missing members remain metadata only. A regular JSON file imitating an envelope cannot invent directory members. Other directory selectors retain their whole original envelope pending durable type provenance. Existing frozen interview rounds are not silently backfilled or replayed.

The configured real Codex orchestrator passed actual `Runtime::observed_turn`, SQLite callbacks and runtime reattachment in 28.50 seconds: native session `01a13dcb-25da-7f43-9600-2493fb34950c`, six committed/native-observed responses, 72,113 complete source/index bytes and both random sentinels. Its disposable diagnostic run was explicitly cancelled after the native turn completed. Fixture: `/var/folders/d2/vtn09lpd2717th937yj7smhw0000gn/T/antislop-native-context-runtime-o6IagF`; log: `/tmp/architect-codex-context-native.log`. This is transport/persistence evidence, not a completed independently reviewed project task or full interview acceptance. The source-catalog extension has separate offline source/provenance regressions; the native probe does not establish semantic browsing of every repository file.

Focused validation passed 217 Rust cases across 15 library/integration targets, with the explicit native diagnostic ignored during the ordinary run; the provider suite's nested environment helper is excluded from that count (`/tmp/architect-codex-context-final-tests.log`, `/tmp/architect-codex-context-agy-regression.log`). This includes the actual CLI-fixture/SQLite reconstruction of a logical request above Codex's 1,048,576-scalar inline ceiling, foreign/forged/replayed/missing/uncommitted response refusal, complete interview-source availability before input selection, stale-live-file separation, symlink/binary metadata and altered capture/authorship rejection. All 22 Python tests, three requirement-ledger freshness checks and warnings-denied all-target Clippy passed (`/tmp/architect-codex-context-python.log`, `/tmp/architect-codex-context-clippy.log`). The locked build, formatting and diff checks also passed (`/tmp/architect-codex-context-final-build.log`). Counts remain test-runner evidence, not full-design acceptance.

P0 work remains genuine complete chat-origin task/quality/delivery acceptance, adaptive role selection, general directory type provenance, efficient source/context handling and compaction. Concurrent checkout integration, nonsoftware families, provider/platform coverage and all remaining semantic requirement reviews remain in scope. The complete design is still partial; no broad acceptance gate is closed by this change.

## Runtime context delivery, durable native read bindings and overlay — 2026-10-15

The explicit Antigravity `pinned_context` profile now supports ordinary turns on the validated CLI 1.3.5 contract. Before native launch, runtime commits the complete turn-bound bundle and deterministic short native-input hash. The native MCP client connects to a private daemon-owned Unix bridge; each page waits for the normal SQLite receipt transaction before it can leave the reader. Native start/completion, conversation/server/tool identity, arguments, committed bytes and replay protection bind the observed response. The 1.3.5 stream omits large output echoes; an omitted echo can match only an actual unobserved backend response for those exact arguments. An echo, when present, must match in full. Unknown tools, forged bodies, duplicate completions and completions without a real read are refused.

Turn completion requires complete native-observed coverage of the index and full prepared prompt; overlaps count once. A genuine schema-bound response based on incomplete delivery remains retained as `rejected_output`, with its hash and explicit rejection, rather than disappearing or completing the turn. Normal handoffs retain whole prior turns, including context and rejected output; completed context/receipt bindings are checked again during ordinary authentication/recovery. Run-ID validation adds the cross-run binding to existing acceptance paths. Contexts preserve the logical input/schema hashes while separately authenticating the actual short native input. The graph overlay reports unique observed/total bytes for concurrent readers and preserves pause semantics. Scoped input/requirement/plan/objective source bodies are indexed from existing captured bytes; no new live filesystem read or input selector expansion is introduced.

Existing `tool_free` configurations remain unchanged; choosing the reader requires an explicit configuration/revision. Other provider/version combinations refuse unsupported isolation rather than downgrading. Models, effort and permissions do not change. No OS sandbox, semantic source inspection or task completion follows from these byte receipts. The user's daemon, state and project configuration were untouched.

The real configured Antigravity high route passed through the **actual Runtime::observed_turn** path, SQLite callback checkpoints, native schema/identity checks, source coverage and ordinary runtime reattachment. Native session `460ebe96-ab7b-47fb-9675-5188f26dd330` delivered the entire 528,484-byte prepared source plus its 631-byte index in 39 authenticated responses, returned both random source sentinels and retained identical context/output after reopening. The test passed in 142.11 seconds. Its disposable diagnostic run was explicitly cancelled after the completed native turn, not declared a completed product task. Fixture: `/var/folders/d2/vtn09lpd2717th937yj7smhw0000gn/T/antislop-native-context-runtime-eLjWiV`; log: `/tmp/architect-native-context-runtime-complete.log`.

Earlier native attempts failed on missing large `step_update` output bodies (`383b5aa2-2299-47a5-a846-039a4dd5783f`, `d4b2350c-237f-4e54-ae31-6de31f94f66f`). Only owned MCP tool frames were captured in an explicitly opted-in unit-test trace, never planner/hidden reasoning. Their actual frame shape informed the strict committed-response matching above. Offline fixtures additionally exposed and fixed macOS accepted-socket nonblocking inheritance and closure of retained socket clones. These failures remain failures, separate from the passing native evidence.

Further P0 work includes complete pre-contract interview source access, browsing individual entries in large directory envelopes, compaction and full real-provider chat-origin task execution/delivery. The older retained thirteen-part proposal still has its original `tool_free` configuration and rejected review; it was not silently upgraded/replayed. Adaptive team selection, integration with concurrent user edits, source lifecycle/platform coverage, substantive nonsoftware acceptance/evaluations and the complete requirement-by-requirement audit remain in scope. Full-design completion is not established.

Rejected native report projections now reach fresh orchestration contexts and the corresponding task's executor/reviewer context. They retain findings, original output hash, author/scope and explicit non-acceptance; the private `agent_messages` outbox is excluded. The exact raw output remains in the original turn and authorized replacement handoffs. A transaction/privacy regression verifies that rejection cannot complete a turn, altered raw output is refused, replacements preserve history and unrelated tasks do not receive the projection. Cancellation after context preparation is rechecked before launch, and active cancellation takes precedence over releasing another queued page.

Validation covers all 69 Rust library/binary/integration targets across the full run and subsequent corrected/focused runs: 603 reported passing cases and 11 intentionally ignored cases. These are test-runner counts, including conditional diagnostic cases, not 603 substantive product acceptances. The first full invocation failed on an older partial-stdin fixture that exceeded the new native input ceiling; it now tests a genuinely pending escaped wire frame with valid decoded input and asserts no completed-send receipt. The remaining sweep exposed an optional installed-CLI preload smoke that conflated an unsupported Claude version with environment isolation; it now executes every available probe, permits only the exact typed post-version-probe rejection and still asserts that inherited preload code never ran. Supported provider/version admission was not widened. All targets then had passing results; the exact coverage/log mapping is `/tmp/architect-context-regression-coverage.json`. Logs include `/tmp/architect-context-all-tests.log`, `/tmp/architect-context-final-focused-tests.log`, `/tmp/architect-context-provider-env-tests.log`, `/tmp/architect-context-final-remaining-tests.log`, `/tmp/architect-context-rejected-report-regression.log`, `/tmp/architect-context-report-privacy-tests.log` and `/tmp/architect-context-task-packet-regression.log`. The complete 79-case task execution target passed, as did fresh orchestration, context, session and actual task-packet regressions.

All 22 Python tests, three ledger freshness checks, locked build, warnings-denied all-target Clippy, formatting/diff checks and doc-test invocation passed (`/tmp/architect-context-release-python.log`, `/tmp/architect-context-final-build.log`, `/tmp/architect-context-release-clippy.log`, `/tmp/architect-context-doc-tests.log`). Package-level gap descriptions now reference the actual context/scheduler/workspace implementation instead of older unconnected-path claims; their status remains partial. Interpretation counts remain 1,482 units, 570 reviewed units and 468 parent semantic reviews open. All owned native probes are terminal.

## Immutable paged context service and genuine MCP read — 2026-10-15

`context::Bundle` and `context::Reader` now bind exact UTF-8 documents to run/revision/turn/input/prompt/schema scope, retain all original source references and provide a paged source index. The internal `__context` subprocess exposes one immutable MCP read tool. It rejects foreign bundles/documents, invalid offsets, changed content and unsafe bundle files, and durably synchronizes each response receipt before attempting source delivery. A failed receipt commit returns no source bytes. No live file/path, shell, URL or write operation is exposed. Integral-valued JSON floats from MCP clients are accepted without rounding; unsafe numeric inputs are rejected.

The actual CLI text-block experiment failed: its joined input was still truncated after byte 192,000. A genuine controlled MCP experiment then passed in 29.40 seconds with the configured Antigravity high route, three authenticated page-response receipts and both random sentinels from a 238,426-byte immutable source. It used no sentinel values in its short model prompt. Native session `287bb365-1688-4e9d-9107-60a8175a5bda` and `/tmp/architect-pinned-context-native-integer.log` establish transport behavior only. The failed text-block experiment and earlier interrupted/refused attempts remain separate evidence. Details and dependency/protocol decisions are in [context.md](context.md).

Production integration is still P0: ordinary Antigravity sessions retain their earlier finish-only profile and preflight. The next change must explicitly admit the scoped reader profile, bind actual native input to the complete logical request, persist/authenticate page receipts through ordinary runtime/recovery and expose actual read progress in the UI. This service alone does not make the retained thirteen-part review acceptable, inspect all sources, handle compaction or complete a task. No user daemon/state/configuration was changed and no new team was installed.

Validation passed 77 distinct automated Rust cases (library 20, Antigravity 12, configuration 19, context pages 10, providers 11 and sessions 5), plus the separate genuine native reader probe and 22 Python cases. The provider suite's nested environment subprocess is excluded from the count. Context tests cover complete large/Unicode reconstruction, paged indexing/deduplication, scope/body/hash tampering, refused foreign reads, exact integral JSON numbers, commit-before-response/failure, actual stdio subprocess durability, stale live-source separation and unsafe-file refusal including nonblocking FIFOs and hardlinks. Locked build, warnings-denied all-target Clippy, formatting/diff checks and all three requirement-ledger freshness checks passed. Logs: `/tmp/architect-pinned-context-regression.log`, `/tmp/architect-pinned-context-final-build.log`, `/tmp/architect-pinned-context-final-clippy.log`, `/tmp/architect-pinned-context-python.log`. All owned native probe processes are terminal. The complete design remains partial with 468 semantic parent interpretations still open; ledger counts do not establish product completion.

## Native input truncation and version-bound exact review context — 2026-10-15

The retained genuine paginated review terminated after 325.15 seconds with a complete 7,843-character report covering all 26 assigned check IDs, all blocked. Its input/output hashes were empty, so runtime correctly rejected its scope and did not launch the remaining twelve parts. Read-only inspection of that experiment's native `USER_INPUT` record confirmed a truncation marker at byte 192,000 and 822,771 removed bytes. The missing candidate and receipts were a real transport failure, not verifier spam or task acceptance. The substantive blocked report and native history remain retained; no team was installed and no task code changed.

The Antigravity adapter now checks the final prepared prompt in UTF-8 bytes before startup inspection, CLI probes, launch or callbacks. Oversized input returns a redacted typed `Protocol/input_too_large` failure with no observed native identity or delivery. New rounds bind `review_input_format=inline-exact-utf8-v7` in their immutable context and use the existing exact inline registry for Antigravity review packets. Every original source, raw candidate, receipt, hash and reserved-wrapper-shaped user value is retained. Unmarked historical rounds keep their original plain request format; unknown format markers are rejected. Other providers retain their earlier packet format.

The transport improvement is partial. Offline read-only measurement of the actual retained proposal produces thirteen parts with approximately 360 KB of input each after deduplication, still above the observed native ceiling. Inspection only alters an in-memory measurement copy and can write a private diagnostic prompt with explicit opt-in; it does not update the saved run, call a provider or establish task success. Complete source/context paging is the next P0; neither dropping evidence, replaying the substantive blocked review nor sending the same oversized packet is an acceptable substitute.

Verification passed 166 substantive Rust cases: library 20, configuration 19, orchestration 44, TUI 52, Antigravity 12, exact executor context 6, independent verifier context 11 and risk-binding context 2. The opt-in metric case without supplied input and the read-only retained-proposal inspection are excluded from that count. New cases prove preflight ordering/privacy, multibyte UTF-8 boundaries, exact transmitted mock input and full review-packet round-trip/legacy compatibility. All 22 Python tests and three requirement-ledger freshness checks passed. Logs: `/tmp/architect-review-input-tests.log`, `/tmp/architect-review-input-regression.log`, `/tmp/architect-review-inline-inspect-packet.log`, `/tmp/architect-review-input-python.log`.

Locked build, warnings-denied all-target Clippy, formatting and diff checks also passed (`/tmp/architect-review-input-build.log`, `/tmp/architect-review-input-final-clippy.log`). All owned live-experiment processes remain stopped; these checks did not start or restart the user's daemon.

The source-first communication audit independently separates the three daemon/delivery/subscription paragraphs into 26 named obligations, including all seven message types, durable delivery, internal deduplication, unknown external outcomes and the qualified limits on status/transcript traffic. The ledger now contains 1,482 interpretation units, 570 reviewed units, 23 semantic anchors and **468 remaining parent reviews**. These counts describe interpretation coverage, not implementation completeness or independent product acceptance.

## Individually authenticated native review parts — 2026-10-15

Large team proposals now use native review pagination. `orchestrator.review_checks_per_part` defaults to 24 non-identity checks per report; V01 and V04 additionally bind input/output identity in every part. This is reply sizing, not a limit on the complete checklist, number of agents, correction rounds, runtime or cost. Zero preserves the legacy single-report path for compatibility/probes. The default is omitted from serialization so old configuration pins remain readable; an interview proposal cannot change the runtime pagination setting. Small replies and existing single-report records retain their earlier path.

Every part has a deterministic ID bound to the complete expected-check set, exact orchestrator output and assigned subset, its own native session and retained raw structured response. Runtime checks current sources/configuration, original approval, completed native authorship, canonical provider/native independence, exact prepared prompt/schema, current evidence IDs, per-check methods/status/rationales and full subset coverage. The agent-recipient catalogue is frozen in the original context or approved recovery handoff, so team installation cannot retrospectively change a report's authenticated input. No individual part approves the proposal. Runtime derives the aggregate only after every part passes, then re-authenticates it against all raw native reports and the entire original checklist before offering separate team installation approval. The aggregate is explicitly identified as derived data in the UI.

Completed prefixes survive interruption. Exact resume retains the interrupted part's native identity when supported; explicit replacement starts a distinct session only for its missing/incomplete reply. Captured final parts and an aggregate can be restored without repeating any CLI turn. A substantive failed/blocked finding stops later parts and cannot use the positive-but-structurally-incomplete retry path. New obligations, changed sources, altered partition plans, tampered native input/output and failed approval commits refuse progression. Subsequent orchestrator turns receive the retained structured partial reports/findings even if no aggregate became acceptable. The graph overlay shows actual received/total reports and the separate runtime authentication phase; recovery cards guide restoration or a distinct reviewer session without repeating Codex or the completed prefix.

Prefix authentication is performed once on entry, each new part is checked on receipt and the complete set is checked again before final acceptance. Shared evidence bodies are hashed once per validation operation, not repeatedly per referencing check; no validation cache survives across calls/scopes. Fixture observation allowances now scale with the actually observed report count without resetting elapsed time; production has no added timeout and tests still require terminal state, exact native call counts and full proof coverage.

Focused verification passed 171 distinct Rust cases: library 20, configuration 19, orchestration 44, TUI 52, Antigravity 10, providers 11, sessions 5, fixed-stage orchestration context 7 and three scheduler execution cases. Nested environment-probe subprocess output is excluded. Five new native executable fixture cases cover full paginated acceptance/authentication, interrupted/positive-incomplete prefix recovery, substantive rejection with finding propagation, repeated exact-ID resume and captured-part/aggregate restoration without replay. Fixtures prove transport/runtime behavior, not model judgment. All 21 Python tests and requirement-ledger freshness checks passed; 471 semantic parent reviews remain. Logs: `/tmp/architect-review-parts-final-verification.log`, `/tmp/architect-review-parts-final-protocol-tests.log`, `/tmp/architect-review-parts-component-tests.log`, `/tmp/architect-review-parts-team-regression.log`, `/tmp/architect-review-parts-stage-context-tests.log`, `/tmp/architect-review-parts-guide-test.log`. Read-only inspection of the retained genuine proposal still passes structural validation and yields 13 native parts (`/tmp/architect-review-parts-native-inspect.log`); it is not native review/task acceptance.

## Scoped instruction observations and native compact-review failure — 2026-10-15

`workspace::instructions` now derives a version-bound index of conventionally named `AGENTS.md`, `CLAUDE.md` and `GEMINI.md` observations, including directory scopes, content/entry hashes and every enclosing input's exact source reference/hash. Root and nested instructions remain separate; applicability uses path components and does not pick a winner by filename, provider or directory depth. The index reaches initial interview observation, later team planning, fixed-stage contexts, task executors and independent task reviewers. An intake proposal must pin applicable observed ancestor instructions through a file or containing directory; a whole-directory input remains one selector. Dropping an applicable instruction or accepting a changed instruction after review is refused.

Only pinned source bodies are used. Link/binary/NUL metadata is not instruction text. Overlapping observations with different content or link metadata fail closed. Legacy directory envelopes inside ordinary JSON files cannot invent instruction descendants: no-follow type metadata disambiguates them, and unavailable type observations remain explicit in `unresolved_directory_scopes`. An unresolved conventional document is not accepted as inspected instruction text. This is a conventional-file index, explicitly **not exhaustive discovery** of arbitrary project instruction documents. Other relevant documents, semantic conflicts, explicit decision records, full source-fragment retrieval and context compaction still need completion. No document can grant runtime permissions.

The source-first audit now atomizes the three previously compound context paragraphs into 31 obligations, preserving all mandatory packet fields, twelve durable source kinds, original-source references, summary authorship, post-compaction checks and non-exhaustive instruction filename examples. Three new semantic anchors and two mutation/traceability tests retain the qualifiers. The ledger has 1465 interpretation units, 544 reviewed units and **471 remaining parent reviews**. These are reviewed interpretations, not proof of implementation or independent product acceptance.

The explicitly approved replacement Antigravity review of the retained native team proposal used the compact report protocol and did not replay Codex. It still exhausted native output (the native log reports 62,911 output tokens). The adapter stopped on the first emitted failed step instead of permitting further internal retries; the experiment ended blocked with a stopped observed native session and retained executor output. Its runtime diagnostic was `Process/unknown`, so nested native error payload classification has additional fixture coverage rather than a claimed fresh native acceptance. `/tmp/architect-live-team-compact-review.log` records a failed test (419.41 seconds), not a completed task. All owned live-experiment processes are terminal. The next P0 review implementation must partition a large report into individually authenticated native parts while retaining the complete expected-check set, source/output scope, every original result/rationale, explicit recovery and final acceptance. Repeating the same whole report is not the next action.

Validation of this slice passed 200 distinct Rust cases: library 20, orchestration 39, project instructions 4, core 34, team 29, TUI 52, Antigravity 10, provider integration 11 and one native task-executor/reviewer instruction-packet case. The nested provider environment-probe subprocess is excluded. The task-packet case exercises actual local CLI fixture processes and authentic runtime receipts, not substantive model judgment. All 21 Python tests and three requirement-ledger freshness checks passed; 471 parent semantic reviews remain. Logs: `/tmp/architect-instruction-index-final-regression.log`, `/tmp/architect-instruction-index-final-tests.log`, `/tmp/architect-instruction-index-final-provider-tests.log`, `/tmp/architect-instruction-index-task-native.log`, `/tmp/architect-instruction-index-final-python.log`. The stopped genuine experiment was separately reconciled after immutable source/candidate comparison, without provider work (`/tmp/architect-instruction-index-checkpoint.log`); that diagnostic invocation is not counted as task acceptance. A separate Cargo target (`/tmp/architect-codex-instructions-validation`) kept builds from changing the companion binary of the then-running native experiment. Warnings-denied all-target Clippy, locked build, formatting and diff checks passed (`/tmp/architect-project-instructions-final-clippy.log`, `/tmp/architect-project-instructions-final-build.log`).

## Version-bound team proposal corrections — 2026-10-15

The orchestrator may now return a `team-plan-amendment-v1` envelope inside `team_plan_json`, bound to an earlier round ID and the hash of its complete native output. Runtime authenticates that proposal's completed native turn, approval and current source/configuration scope before applying configuration-only entity edits. An authentic rejected proposal can be corrected; its rejection is preserved and does not become approval. Backward-only iterative chains reject self/forward references, altered outputs, forged session/history fields and invented risk identities. The resulting complete team still passes every original DAG, gate, scope, role, check-plan and risk-allocation validator. Independent review receives the full resolved team and risk ownership, and the UI approval card displays that complete result and base provenance. Installation retains inherited risk ownership and still requires a new explicit approval.

Three native-adapter fixture tests cover chained amendments, correction of an authentic rejected risk allocation, full UI/install results, tampered authorship/base hashes and forbidden history/permission changes. These fixtures prove protocol enforcement, not real-model task completion. Validation passed: 136 distinct Rust cases (orchestration 37, library 18, team 29, TUI 52), 19 Python tests, warnings-denied all-target Clippy, formatting/diff and all three requirement-ledger freshness checks. Source records remain partial with 474 semantic parent reviews open. Logs: `/tmp/architect-team-amendment-regression.log`, `/tmp/architect-team-amendment-final-tests.log`, `/tmp/architect-team-amendment-final-ui.log`, `/tmp/architect-team-amendment-final-clippy.log`.

The genuine retained Codex experiment produced a 12,011-character amendment instead of regenerating its 93,601-character full response, then a 1,547-character chained correction. Native output hashes and original records remained intact; both were rejected before reviewer launch on subsequent coverage gaps. These are authentic correction attempts, not completed tasks. The first correction fixed invalid gate IDs, missing prerequisites and runner coverage; the second supplied global full-domain-proof coverage. That second attempt was still rejected on original-plan-item coverage. Runtime now supplies exact phase-qualified runner gate IDs, role-specific required runner coverage and per-original-plan requirement/criterion/dependency contracts; diagnostics enumerate concrete missing IDs. Logs: `/tmp/architect-live-team-amendment.log`, `/tmp/architect-live-team-amendment-coverage.log`.

A third authentic 28,171-character amendment corrected that per-item coverage, passed the complete structural validators and reached a fresh Antigravity review (round `34725001-aa1d-44bf-9e05-177abf09dc1f`). That review did not complete: its native CLI reported output-token exhaustion twice while internally retrying. The owned experiment was paused and its daemon stopped after these explicit failures; the guardian reaped the native process group. Its startup checkpoint was then explicitly reconciled only after confirming every original and managed-candidate fixture input was unchanged. The completed Codex output remains retained, the verifier is interrupted, and no team was installed or task code changed. `/tmp/architect-live-team-amendment-plan-coverage.log` is a failed experiment, not a passing task; `/tmp/architect-team-amendment-checkpoint.log` is only a read/reconciliation observation without provider execution.

Review transport now permits reviewer-authored shared proof records (method and receipt IDs), with an independent status and specific rationale for every original check. It expands references losslessly before unchanged validation; raw native JSON remains the authenticated output. Unknown/duplicate/unused/empty proof definitions and ambiguous checks fail closed; legacy expanded reports remain readable. New native schemas request compact references. This reduces repeated output fields, not the number of mandatory checks or inspection obligations; genuine compact-review acceptance and bounded review partitioning remain open. The Antigravity adapter also stops its owned process group immediately when an inadmissible failed native step is emitted, preserving a redacted typed diagnostic and observed identity instead of allowing internal retries that could never yield an admissible turn. Its output-limit/descendant cleanup test is a native executable fixture, not a paid-provider acceptance.

Final checks for the correction/compact-review/fail-fast slice: 160 distinct Rust cases passed (library 20, orchestration 38, team 29, TUI 52, Antigravity 10, shared provider integration 11), plus 19 Python tests, all-target warnings-denied Clippy, formatting/diff checks and all three requirement-ledger freshness checks. No diagnostic-only ignored live-harness invocation is counted as task acceptance. Logs: `/tmp/architect-native-output-limit-final.log`, `/tmp/architect-review-proofs-ui-regression.log`, `/tmp/architect-amendment-proof-final-providers.log`, `/tmp/architect-amendment-proof-final-clippy.log`. The next native acceptance should use an explicitly approved verifier replacement on the reconciled retained proposal, without replaying Codex; review partitioning and source/context scaling remain open priorities.

## Native interview acceptance work — 2026-10-15

A new explicitly ignored `tests/live_team.rs` experiment uses the real configured Codex orchestrator and Antigravity verifier through an actual daemon, IPC backend and frozen TUI approval cards. Its disposable Rust magnitude fixture retains immutable original inputs, warnings-denied compilation and boundary tests; it permits only `lib.rs` edits and the exact `python3 check.py` argv. State and native outputs remain in a private retained directory on failure. It is not part of ordinary fixture test execution and requires an explicit `--ignored` invocation with provider usage.

The first native turn exposed missing source context: intake had no executable inputs and the orchestrator attempted a forbidden native tool. New interviews now observe the project in the background before committing their native input receipt. Small captures include complete directory bodies; larger captures include an explicitly incomplete membership/type/hash manifest instead of unbounded source text. The complete capture hash detects drift before accepting an answer; an answer to changed sources is retained and rejected before contract review. The progress overlay names runtime source observation without inventing a native agent. This is not complete paged source retrieval or large-task context management. A local inventory measured 11,294,451 nonignored text characters across 230 files; the Codex adapter enforces its existing 1,048,576-character transport preflight.

Codex invocations now include a reasoning-only developer instruction and disable Apps and shell snapshots alongside the existing controls. Forbidden native events remain rejected and identify the event kind. The adapter still advertises read-only isolation, not a proven tool-free allowlist. The official [configuration basics](https://learn.chatgpt.com/docs/config-file/config-basic) and [configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference) document the separate tool features and managed-requirement caveat; no global Codex configuration, requirements or authentication files were changed.

A subsequent authenticated Codex response correctly chose one directory input `.` with the narrower editable leaf `lib.rs`. Runtime validation previously rejected this legitimate reduction in permissions. Editable leaves may now be contained by an actually pinned directory without expanding its input list; sibling files, traversal and file-as-directory grants remain refused. Native source input/output and historical rejections remain retained.

The real Antigravity review then exposed ambiguous evidence-ID instructions: it used a source label as an identifier and omitted the separately observed output receipt. The verifier schema now restricts references to actual supplied IDs and explicitly maps required V01/V04/intake-source checks to their UUIDs. Runtime evidence validation remains unchanged. A separately approved fresh verifier may replace a completed positive report that fails structural validation while retaining its actual raw output/history and original orchestrator result. Substantive failed/blocked checks or findings cannot use this path. The chat guide offers the eligible retry with its frozen handoff.

The experiment also exposed a driver read race. The harness now retries only paginated read observations when the document changes; it does not repeat mutations. Its interrupted read-only interview was explicitly reconciled, and the retained completed Codex answer proceeded to an independent Antigravity session without re-running the orchestrator. The corrected native Antigravity review passed. The real IPC/TUI guide accepted the interview/routing, separately observed and prepared a private workspace, activated it in revision 3 and started Codex team planning. The original protected files remained unchanged. Codex returned a 30-agent/15-task proposal with 240 distinct risk allocations, but runtime rejected its gate prerequisites before any team installation. Read-only preparatory artifacts had been incorrectly forced to wait for future implementation gates; they are now exempt while writers/code roles retain those gates. Missing-gate diagnostics name the exact task, role/stage and missing IDs, and the planner receives that machine contract explicitly. The real proposal still needs a corrected plan and independent review. The native experiment has not yet established complete task execution/delivery; its current retained state is `/var/folders/d2/vtn09lpd2717th937yj7smhw0000gn/T/antislop-live-team-wrjrBX/state`.

Validation of this continuation: 165 distinct Rust cases passed across library 18, Codex input preflight 3, file operations 7, intake/startup 11, providers 11, team 29, TUI 52 and native-adapter orchestration 34. Nested environment-probe subprocess results are excluded. The startup set was rerun with its original parallel execution after adding an owned-child-only disconnected-handshake retry; an existing server's protocol/application rejection remains terminal and does not spawn a replacement. Logs: `/tmp/architect-native-interview-final-focused.log`, `/tmp/architect-ui-startup-final.log`, `/tmp/architect-native-orchestration-regression.log`. All-target Clippy with warnings denied, locked build, formatting and diff checks passed (`/tmp/architect-native-interview-clippy.log`, `/tmp/architect-native-interview-build.log`). Requirement-ledger freshness checks and 19 Python tests passed; 474 parent semantic reviews remain open. The explicitly ignored real-provider experiment remains a partial native acceptance, not a passing Rust test or completed task.

Retained native evidence: `/tmp/architect-live-team-sources.log` (completed initial Codex proposal and the old directory/leaf validation rejection), `/tmp/architect-live-team-recovered.log` (retained executor restoration and invalid verifier citation), `/tmp/architect-live-team-proof-retry.log` (accepted real review, contract/workspace progression and rejected team proposal). The last experiment stopped after its authoritative test failure, with no team installed or task file edits admitted. Source retrieval for large tasks and efficient correction of a rejected DAG remain immediate work; source manifests do not close them.

## Current implementation priorities — 2026-10-15

The scope remains every established requirement in design §§2–18, the complete risk/checklist catalogue and M0–M5 acceptance. The sections below retain historical evidence; their earlier “next” lists do not override this order. None of these broader outcomes is closed by protocol fixtures.

| Priority | Next outcome | Evidence needed to close it |
| --- | --- | --- |
| P0 | Complete an actual small chat-origin task with the configured Codex orchestrator and Antigravity executor/verifier roles. | Retained native interview, selected role/configuration decision, independently accepted plan/task results, actual meaningful checks and separately approved delivery; include a real rejection/repair. |
| P0 | Integrate approved work with the current user checkout and concurrent task results. | Separately identified integration candidate, preserved staged/unstaged/untracked user versions, explicit conflict resolution artifact and independent checks of the complete merged candidate. Current transfer refuses unresolved drift/unselected changes. |
| P1 | Complete effect ownership and artifact production across software, research, documents and analyses. | Standalone absent-file lifecycle, binary/link/mode proposals where required, versioned deliverable files, independently opened sources/calculations and end-to-end substantive family acceptance. Text creation/deletion through directory scopes is implemented below. |
| P1 | Finish isolation/resources, intent controls, autonomous progression and exact-session recovery. | Per-task environments/resources or enforced serialization, capability-bound local automation, real-provider restart/cancel/recovery tests and no blind repetition or external-effect permission expansion. |
| P0 | Close large-project source retrieval/context handling and semantic requirement interpretation. | Independent reading of original requirements, every parent qualifier checked, adequate source access without recursively duplicated context and representative whole-repository tasks. Structural ledger freshness alone does not establish this. |
| P2 | Complete provider/platform acceptance and M5 calibration. | Authenticated supported-version matrix on macOS/Linux, seeded quality failures and repeated comparisons against common-input baselines, with observed limitations and reproducible retained evidence. |

## Authenticated file operations and directory write scopes — 2026-10-15

Native task and fixed-stage edit schemas now advertise explicit `write`, `create` and `delete` operations. Omitted operation retains legacy write semantics and serialization, preserving historical scope/receipt hashes. Directory inputs authorize their observed text leaves within explicitly assigned editable scopes without expanding the input list. Creation requires observed absence; deletion requires existing text and empty replacement content. Creation/deletion require a captured containing directory. Exact standalone file inputs retain writes; missing standalone input lifecycle, binary/link/mode proposals and general integration remain open.

Write-ahead receipts bind the operation and before/after presence as well as hashes. Directory state advances from the authenticated effect only, preserving unrelated members, original evidence and a single declared input. Captures retain Git tracked membership so deleting a tracked file produces an explicit missing entry, while deleting an untracked file removes its member. Presence remains distinct even if literal file contents hash to the absence marker. Ignored or unobserved existing create targets are refused before effects. Recovery compares actual leaf presence with directory membership, so changed ignore rules cannot disguise an applied creation as absent. New targets use exclusive creation; file effects reject symlinked ancestors and Git/runtime metadata, within the existing mediated-workspace race limitations.

Native effect authentication, same-wave proof reconstruction, explicit interrupted-effect adjudication, completed-result restoration and source-delivery ownership use the same operation/presence checks. Reconciliation reconstructs complete directory observations from selected native proposals and authenticated sibling effects; an unrelated user change prevents acceptance. The supervised wave card advertises its permitted file operations alongside the exact write scopes. Final source transfer admits owned creations/deletions and retains deleted source entries under the existing delivery backup/recovery protocol.

Focused regression passed 161 cases: library 17, core 34, directory inputs 7, file operations 6, recovery 5, source delivery 8, team 28, TUI 51, completed-verifier recovery 3 and policy/revision recovery 2. Native adapter fixtures also passed creation/deletion outcome-loss restoration without a repeated CLI/effect, and both existing exact-cohort recovery cases. The policy regression now checks the implemented worktree admission followed by refusal of execution before activation, rather than its obsolete assumption that worktree runs cannot be created. Logs: `/tmp/architect-file-operations-final-focused.log`, `/tmp/architect-native-file-operations-recovery.log`, `/tmp/architect-file-operations-cohort-regression.log`. All-target Clippy with warnings denied, locked build, formatting and diff checks passed (`/tmp/architect-file-operations-clippy.log`, `/tmp/architect-file-operations-build.log`). These fixtures supply synthetic role judgments and do not establish real-model completion, Linux support or complete integration. The final full software-team fixture also passed with separately narrowed write scopes: one native proposal wrote an existing file, created `new/sub.txt` and deleted `obsolete.txt`; every task/result received its independent review and stage acceptance, original sources stayed unchanged during execution, and separately approved transfer delivered all three effects while retaining the deleted source version and refusing replay. Evidence: `/tmp/architect-native-file-operations-final.log` (216.35 seconds for this local fixture). Together these are 165 distinct Rust tests, not a whole-suite or real-model acceptance claim. The three requirement-ledger freshness checks and 19 Python tests passed; the structural audit still reports 474 parent semantic reviews outstanding. No M0–M5 or full-design completion is claimed.

## Scoped source delivery — 2026-10-15

Completed managed teams now have separate source-delivery inspection and approval through runtime, IPC, CLI and the TUI guide/palette. Inspection authenticates every accepted team result and final quality gate, binds the full candidate and original source observation, and reports unsupported or conflicting effects. Transfer preserves prior entries and refuses stale source/candidate proofs or newly appearing destinations. Per-file write-ahead checkpoints, retained backups and explicit observation/restoration controls prevent automatic replay after interruption. The UI freezes the reviewed transfer and recovery details across refreshes and projects actual background inspection/application phases.

At this slice's validation, runtime transfer supported authenticated managed content updates to existing regular files; the subsequent file-operations work above adds creation/deletion. Link/mode ownership, integration with unrelated user changes, multi-candidate composition, backup history/cleanup, local commits and publication remain separate work. The lower-level Unix transport supports binary data, additions, deletions and relative links without claiming runtime authorization for them. See `workspace-lifecycle.md` for recovery and filesystem boundaries.

The complete-team fixture also exposed a bootstrap deadlock: DeliveryAuthor required `review_complete` before running, although its dedicated verifier produces that gate. The prerequisite was removed; independent delivery review and final `review_complete` acceptance remain mandatory. Final gate diagnostics now list the exact missing or stale phase-qualified checks. The complete flow also exposed a mismatch between persisted gate keys and final-check IDs; scheduler context now publishes the authenticated assessment under both identifiers, preserving legacy task prerequisites and exact final acceptance.

Focused validation passed 104 cases: library 17, file transport/runtime recovery 8, team 28 and TUI 51. The full native-adapter fixture passed separately: actual local CLI processes installed and completed the complete software team in a managed workspace, independently reviewed and accepted every result, inspected delivery, transferred only its owned content change after a separate approval, retained the previous source entry and refused replay. Evidence: `/tmp/architect-delivery-final-focused.log` and `/tmp/architect-complete-team-delivery.log`. The fixture supplies synthetic role judgments; it does not establish real-model task quality or Linux acceptance. All-target Clippy with warnings denied, locked build, formatting and diff checks passed (`/tmp/architect-delivery-final-clippy.log`, `/tmp/architect-delivery-final-build.log`).

## Guided managed workspace and directory admission — 2026-10-15

Worktree tasks now have a read-only background inspection followed by separate preparation and activation approvals in the chat guide/palette. Inspection persists the exact source observation and candidate plan without creating a worktree or starting a provider. The preparation card keeps directory selectors intact, identifies task-owned local working copies/deletions and shows the repository version, file count and size. Runtime authenticates this stored plan against the current task without rerunning Git while IPC waits; the worker revalidates source bytes/membership before copying. Candidate validation/activation also runs in the background, and controls return before expensive observation. The progress overlay names the actual runtime workspace phase instead of claiming a native agent is running. Confirmation retains its original selected changes, destination/version and scope across refreshes.

Directory inputs remain single task inputs while candidate admission checks every actual member's bytes/type/link identity. Git and non-Git sources, empty declared directory scopes, absolute in-workspace selectors and projects nested inside a repository are supported. Activation compares source content across relocation rather than requiring Git/non-Git capture metadata to match literally, then binds the candidate's current input representation in a new revision. Original selected baseline bytes remain retained; before any task executes, a file-scoped task also gains the complete initial private-project baseline so unchanged repository files do not appear as additions. The original source branch/index/working files are preserved. Untracked excluded runtime state is removed from repository membership metadata as well as the file map, preventing private candidate creation from falsely invalidating a parent-repository source snapshot.

Accepting a worktree interview contract requests only background inspection first. After explicitly approved activation of a chat-origin task, a runtime-authored status message queues team planning in the new workspace/revision. The next explicit orchestration approval starts it; inspection, copying and activation start no agents. Read-only inspection or activation interrupted before their outcome commit is recovered without replay or false project-write uncertainty. Preparation effects retain their existing uncertain-result adjudication; corrupted candidates and stale source captures refuse activation/copying. General uncertain-preparation recovery through the UI remains incomplete.

The focused regression passed directory inputs 7, intake 10, managed workspace 12, TUI 50 and source snapshots 8; the final workspace/TUI repeat passed all 62 cases. Bootstrap conversations now default to worktree mode, while explicit project in-place configuration stays unchanged. Both daemon/CLI-adapter interview scenarios passed: in-place acceptance directly plans the team, while worktree acceptance observes, separately prepares/activates and only then plans/installs the selected team. No executor task is started during those steps. The bootstrap/diff/library regression also passed 39 cases. Final all-target Clippy with warnings denied, locked build, formatting and diff checks passed. Evidence: `/tmp/architect-workspace-final-regression.log`, `/tmp/architect-managed-workspace-final.log`, `/tmp/architect-worktree-intake-verified.log`, `/tmp/architect-worktree-default-regression.log`, `/tmp/architect-workspace-complete-clippy.log`, `/tmp/architect-workspace-complete-build.log`. These checks establish preparation/activation and source preservation, not integrated delivery, resource isolation or whole-design completion.

## Independently observed diff review — 2026-10-15

The on-demand `ReviewDiff` action no longer invents an inspected V04 pass. It commits a scoped candidate-review intent, then starts an independent native verifier through the pinned implementation-review route, restricted to read-only or tool-free isolation. The immutable record binds the original baseline, observed candidate inputs, complete displayed diff, comments, profile and explicit approval. The existing session ledger retains native identity, transport events, completed output and usage. Runtime verifies native authorship, independence from executor sessions and exact response/candidate bindings before saving the advisory result.

Candidate reviews are separate `diff_reviews` records. They neither install task gates nor populate the stage-review ledger, advance a stage, change the task attempt or adopt external changes into the accepted task inputs. A passed, failed or blocked reviewer response remains visible as its actual judgment. Malformed, forged and contradictory responses retain their native output without a positive result; a completed read-only turn does not create false project-effect uncertainty. Actual interruption/unknown effects still require reconciliation. Changed candidate bytes or concurrent comments retain the response as stale. Intent-commit failure starts no provider; outcome-commit failure leaves the completed raw turn without an accepted review, and restart never replays it.

The TUI graph shows the reviewer and observed session state; its findings and version appear in the diff view and agent details. Confirmation freezes the displayed diff and prospective verifier profile across refreshes. Reviewer nodes support version-bound comments. Diff comments now queue an exact-reference notification to the configured orchestrator without starting work or granting permissions, and orchestration context includes candidate-review summaries rather than reviewer transcripts. The existing `review-diff ... --approve-scoped` CLI follows the same runtime path.

The latest diff/comment/TUI regression passed 55 cases: diff comments 4, native-adapter diff review 8 and TUI 43, including concurrent editor changes from the other development session. It includes an actual daemon/IPC/TUI confirmation flow and CLI output displaying the received rejection. An earlier focused run also passed library 17, intake 10 and recovery 5; the broader preceding run passed core 34 and directory inputs 7. The adapter fixtures exercise actual supervised CLI processes and include rejection, hash/schema contradictions, source/comment drift, both persistence boundaries, cancellation/restart, exact comment notifications and frozen confirmation. Final all-target Clippy with warnings denied, locked build, formatting and diff checks passed. Evidence: `/tmp/architect-independent-diff-verified.log`, `/tmp/architect-diff-ipc-verified.log`, `/tmp/architect-independent-diff-final.log`, `/tmp/architect-diff-ipc-clippy.log`, `/tmp/architect-diff-ipc-build.log`.

A real disposable task using the project's Antigravity routing completed through CLI 1.3.5 in observed native session `583e6efa-179a-452b-b4b8-ba6fb8a979ac`. Its candidate removed the `n == 0` branch from an integer division function whose original requirement preserves zero without panic. The independent response rejected the candidate with a concrete finding; exact diff/input hashes matched, original accepted input bytes remained unchanged, and no stage review or quality gate was created. Evidence: `/tmp/architect-real-diff-review.log`; retained private state is `/var/folders/d2/vtn09lpd2717th937yj7smhw0000gn/T/.tmpUoT9ET/state.db`. This accepts that small authenticated review scenario, not general model quality, whole-task completion or the full design.

At this slice's completion, directory admission and guided workspace preparation/activation remained open; the subsequent implementation above addresses their normal path. Complete integration/delivery, uncertain-preparation UI recovery, review recovery from retained completed turns and substantive provider/platform/task-family/autonomy acceptance remain open.

## Directory diffs and persistent candidate baseline — 2026-10-15

Design §8.6 now compares directory scopes by their captured members rather than treating the directory observation as a deleted JSON file and the repository as additions. A selected directory remains one task input. Text members have ordinary hunks and line counts; binary and literal link-target changes have explicit metadata descriptions with no invented text counts, in both TUI and CLI. Deleted scopes, directory-to-file/link changes, absolute in-workspace scopes and excluded private runtime state are covered. Directory diff reads use the same Git/nonignored source membership as source capture; link targets are never read.

New runs retain `diff_baseline` independently of the current `input_snapshot`. Contract acceptance and explicit task revision set the baseline; managed edits, reconciliation and workspace activation preserve it. IPC diff documents, TUI projection and diff-review scope calculation use that baseline. Legacy documents lacking it retain their previous snapshot fallback; an original baseline cannot be reconstructed reliably from an already advanced snapshot.

Managed edits update their explicitly pinned leaf and every overlapping captured directory using only the known before/after bytes. They do not reread unrelated current files into accepted state. Disjoint same-wave writes must reconstruct the complete current snapshot from authenticated native outputs and applied receipts, including directory aggregates. Source observation during recovery now accepts directory scopes as readable versions instead of classifying them as invalid file paths.

Focused validation passed 119 cases: directory inputs/diffs 7, diff comments 4, recovery 5, TUI 37, library 17, core 34, intake 10, managed workspace 3 and two native-adapter fixture scenarios for disjoint writers with and without a directory scope. The new directory scenario checks physical writes, aggregate/current consistency, retained original input bytes, database reopening and TUI diff projection. These are local fixture checks, not real-model task completion. Logs: `/tmp/architect-directory-version-regression.log`, `/tmp/architect-directory-baseline-core.log`, `/tmp/architect-directory-native-writes.log` and `/tmp/architect-disjoint-writes-regression.log`.

All-target Clippy with warnings denied, locked build, formatting and `git diff --check` passed. The checked binary remains in the isolated validation directory; coordination with the other session handling the live user run remains separate.

At this slice's completion, independently observed on-demand code review, directory admission to managed candidates, guided workspace preparation/activation and complete integration/delivery remained open. The subsequent diff-review implementation above supersedes the historical synthetic V04 path. No live user session restart or provider task was part of the directory-diff validation slice.

This audit compares the current worktree with `design.md`; it does not approve the design's technical proposals or declare MVP completion. The repository has one base commit (`418675d`), so the recent implementation is the uncommitted worktree, including new files. The original scope remains all established requirements, M0–M5 gates and product acceptance scenarios.

| Design area | Current evidence | Remaining work |
| --- | --- | --- |
| Local runtime, approvals and IPC (§6, §9–10, §14) | SQLite state/events/deduplication, private socket, supervised scope hashes, chunked reads, guardian processes, edit receipts and explicit reconciliation. Pause holds new provider/check/write admission; resume retains the same worker/attempt; cancel wakes waiting workers only after its commit. | Native session continuation after reconciliation; event subscriptions and control ownership. |
| Quality and requirement fidelity (§6.4–6.7, §7.5–7.6, §12) | Typed context/model/quality outputs, authenticated review and risk bindings, source invalidation, exact provider-session authorship and proof-context closure. Source audit retains 1146 fragments, 1443 interpretation units, 513 reviewed atoms and 474 remaining parent reviews. | Finish semantic requirements review and prove each product requirement independently. Structural completeness and synthetic reports cannot establish substantive correctness. |
| Team construction and scheduling (§7) | Built-in templates and standalone scheduler; native orchestration proposals, dedicated independent review, complete risk ownership and explicit authenticated installation now connect to the daemon and TUI. | Execute the accepted DAG through runtime-owned task sessions, artifacts, quality gates and recovery. The accepted DAG now has a runtime wave bridge; task sessions, artifacts and reviewed checks are durable. Source-edit revalidation, complete role artifacts and executor recovery, uncertain checkpoint recovery and complete task-role acceptance remain incomplete. Explicit verifier-only replacement after reconciliation is now available; its evidence is described below. Legacy progression stays blocked after installation. |
| CLI integrations (§9.4–9.6) | Codex, Claude, Gemini and agy adapters, structured transport fixtures, session ledger, sanitized diagnostics, TOML routing and capability checks. | Authenticated cross-provider/platform acceptance and exact-ID recovery under actual supported isolation. Existing live evidence only proves its recorded versions/tasks. |
| TUI and communication (§8, §10.4) | Ratatui, background IPC, terminal sanitization, authenticated point-to-point/broadcast outboxes, durable messaging/session order, consistent snapshots and orchestrator conversation with explicit turn/review approval. Reviewed team proposals are visible and separately accepted. | Team execution, active-turn hook transport, automatic intent-to-command translation and mode changes. Ambiguous legacy chronology is explicitly unknown. Missing pinned orchestration routes are refused. |
| Workspace and integration (§11) | Controlled in-place edits, pinned sources, workspace leases and disposable source-copy/worktree experiment drivers. | Managed default worktree lifecycle, dirty-worktree ownership, integrated candidates and general integration back to user workspaces. `RunSpec::validate` currently rejects configured managed worktrees. |
| Autonomy, task families and calibration (§6.2, §15–18) | Policy/configuration types and independent structural tests; family-specific quality contracts; completed scoped self-host evidence. | Connect autonomous progression to permitted effects; full software/research/document/analysis acceptance; shared-input baseline evaluations. The self-host driver's scoped approvals do not implement full autonomous mode. |

## Milestone assessment — 2026-10-14

The implementation is an advanced supervised prototype, not a completed MVP. No percentage is assigned: source fragments, passing tests and independently accepted product scenarios measure different things.

| Milestone from §16 | Assessment | What prevents closure |
| --- | --- | --- |
| M0 — CLI spike | Partial; adapters and capability/transport fixtures exist, with scoped historical live evidence. | Authenticated acceptance across required providers/platforms and isolation capabilities remains incomplete. |
| M1 — local runtime | Core implemented and regression tested: daemon, SQLite, IPC, leases, control, reattachment and no blind replay. | Full production acceptance, event subscriptions and controller ownership remain open. |
| M2 — supervised vertical slice | A scoped real CLI slice and self-host change have recorded evidence; the legacy fixture flow completes all six stages. | Historical evidence does not accept the current whole worktree or general task execution. |
| M2Q — quality contracts | Partial: typed models, requirement/plan checks, all 240 risks, independent reviews and source/evidence binding. | Complete semantic requirement interpretation, role-specific runtime execution and independent product acceptance remain open. |
| M3 — team and TUI | Partial: spatial graph, actual-step overlay, version-bound comments and reviewed reactions, communication, reviewed team installation, native waves, read-only source refresh, verifier replacement and supervised result-package acceptance. | Broader unknown-effect/check/scheduler recovery, full role/family execution, automatic intent handling and end-to-end product acceptance remain open; scoped executor restoration, native continuation/replacement and management recovery are connected. |
| M4 — full MVP | Partial foundations: CLI adapters, pinned TOML profiles and task-family types/contracts. | Managed worktree/integration lifecycle, autonomous and revised-scope recovery/progression, isolation and complete software/research/document/analysis scenarios remain open. |
| M5 — calibration | Scoped experiments and audit helpers exist. | Representative repeated comparisons against common-input baselines, cross-platform recovery/security acceptance and routing calibration remain open. |

The self-host acceptance is retained in `docs/selfhost/final-acceptance.json` and `root-transfer.json`. Historical intermediate failure reports remain historical; they must not be presented as the current final result. Earlier `implementation.md` validation counts describe their recorded run, not the present suite.

## Direct-message integration

The previously exposed TUI `send_message` action had no matching runtime command. The new integration reuses `CommunicationLedger`, `ContextPacket`, `SessionLedger` and SQLite transactions. No dependency or separate messaging service is needed.

The model is: authenticated local user → immutable queued message → exact next role invocation → reserved packet and delivery intent → observed transport send → committed native output and delivered state. A completed turn is not semantic acknowledgement of the message. `delivery_pending_unknown` records intent before process launch; `sent` requires an actual CLI send event; `delivered` requires the session ledger's validated completed output. Failure, cancellation or restart preserves uncertain delivery as `unknown`; it is excluded from future packets.

Invariants:

- User messages cannot supply a sender, grant permissions, revise requirements, approve stages or start provider work. `revise` remains the explicit scope-change path.
- Recipient IDs identify exact fixed-stage roles (`Discovery:executor`, `Discovery:verifier`, and the corresponding Planning/Execution/Validation/Integration/Delivery roles). An unallocated orchestrator or standalone team agent is refused.
- Packet selection reads the authoritative queue under the store lock. Messages arriving after selection remain queued for another invocation. Runtime checkpoints and attempt completion preserve concurrent queue updates.
- Packet reservation, message intent and session reservation share a database save. A failed save creates neither a delivery intent nor a reserved session and prevents launching the provider.
- The persisted packet hash belongs to the exact turn, recipient and scope. Bodies and sender/order references must match the accepted immutable records. The packet is appended as task data without replacing policy or prior context.
- Revision cancels unsent messages from the old scope. Cancellation also cancels unsent messages; history, bodies and prior confirmed deliveries remain inspectable.

Input classes: valid Unicode/multiline text is retained byte-for-byte; empty/whitespace-only text, NUL and oversized transport input are refused. Unknown/case-mismatched recipients are refused. Same key/same payload returns the existing result; same key/different payload is refused. Terminal runs refuse new messages. Missing/corrupt message bodies or wrong session/turn bindings fail before delivery. The transport's existing frame bound protects IPC; it is not an agent or task budget.

Verification covers queue persistence across restart, real TUI-backend IPC roundtrips, concurrent messages during a live check worker, exact executor/verifier routing, packet hashes, immutable history, revisions/cancellation and a SQLite failure at the reservation boundary. Native adapter tests capture actual Claude/Gemini mock-process stdin and compare the decoded prompt and packet. These local fixtures prove integration and receipt handling, not real-model understanding or provider quality. Authenticated message smoke tests and Linux validation of this change remain open.

Local validation after this integration:

- `scripts/cargo.sh test --all-targets --locked`: 296 passed, zero failed, seven ignored across 49 test binaries. Nested fixture subprocess results are excluded from this count. Six ignored tests require supplied historical snapshots; one explicitly records the open containment case for descendants using `setsid`.
- `scripts/cargo.sh clippy --all-targets --locked -- -D warnings`, `scripts/cargo.sh fmt --check`, `scripts/cargo.sh build --locked` and `git diff --check`: passed.
- All three requirements/audit/gap freshness checks and 19 Python unit tests: passed. The semantic interpretation gaps above remain open.
- Both fake demo paths: passed, including six-stage acceptance, independent fixture reviews, daemon restart/deduplication and seeded rejection waiting for repair approval.
- An earlier full run hit benchmark child timeouts while Clippy compiled concurrently. The isolated benchmark and final full suite passed with the original deadlines and assertions unchanged.
- Refreshed local dependency inventory: 218 locked packages, matching the current lockfile hash; three active packages still lack locally inventoried license source files. This is an inventory, not completed legal review or vulnerability scanning. The messaging change adds no dependency.

## Pause and session observation

The next integration fixes the pause boundary and session projection from §8, §9.3 and §14. It reuses the store mutex with a standard-library condition variable; no scheduler budget, polling deadline or dependency is introduced. Pause completes admitted effects and stops new provider reservations, checks and writes. The live worker retains its continuation and lease. Resume and cancellation serialize with admission through the same store lock; only a committed cancellation signals processes. A restart requires reconciliation of the lost continuation and preserves the user's pause.

| Contract | Verification |
| --- | --- |
| No new session while paused; completed executor output survives | `runtime::control` exercises observed executor completion followed by blocked verifier reservation, then resume in the same attempt. |
| Pause releases the store lock but retains workspace ownership | `tests/runtime_pause.rs` completes an independent sibling run while the paused workspace remains leased. |
| Cancel wakes a waiting worker; failed cancellation is inert | Tests verify lease release without a provider launch and inject a SQLite cancellation-commit failure while a real local check is alive. |
| Resume does not replay effects or ignore changed inputs | Baseline receipt IDs, attempt/approval counts and input bytes are checked across pause/resume. Changed inputs block before provider reservation. |
| Restart preserves pause and blocks replay | `tests/daemon.rs` kills an actual daemon at its durable waiting boundary, reattaches and checks that the completed check's effect still occurred exactly once. |
| Latest session is observed, history retained | `turn_order` is committed with reservation. Session tests cover failed saves, duplicate requests, malformed order, roundtrip and missing legacy fields. TUI tests reverse lexical identity order and check selected status, history and proof ownership. |
| Controls and document share a snapshot | An IPC regression changes the actual run between snapshot and read. The TUI rejects the mixed observation and refreshes through read-only operations. |

Legacy ledgers retain all earlier sessions; unknown chronology is shown explicitly rather than inferred from UUIDs. Provider completion means output availability, not successful independent task verification. The TUI backend no longer retains unused whole-run caches for previously selected runs. Native recovery, an orchestrator conversation, full team execution and the other open design requirements remain unchanged by these fixes.

Validation after the pause/session changes: the locked full Rust suite passed 307 tests with zero failures and the same seven ignored tests across 51 test binaries. Clippy for all targets with warnings denied, formatting, build and diff checks passed. The three requirements freshness audits and 19 Python unit tests passed; semantic completeness remains unproven. Both fake CLI demos passed: six-stage acceptance and seeded rejection requiring repair approval. No authenticated provider run or Linux validation is claimed for these changes.

## Orchestrator proposals and team installation

The daemon now accepts explicitly approved `orchestrate` requests after queuing a user message. It resolves the pinned orchestrator and dedicated orchestration-review profiles independently, reserves separate logical/native sessions, appends only the messages authorized for that round and retains raw completed output. A conversational answer receives common independent checks; a team proposal also requires all original criteria/plan items, orchestration checks and exactly 240 justified risk owners. The existing TeamPlan validator rejects forged history, cycles, missing pairs/competencies, scope expansion and ungated writes before review. Review acceptance authenticates current receipts, source hashes, native authorship and versioned contracts; structural completeness still cannot prove model judgment.

Team installation is a separate hash-bound user action. New messages, source drift, a later orchestration round, rejected review, altered raw output or missing native authorship prevent acceptance. Runtime resolves profiles and attaches only its actually observed orchestration sessions. All tasks remain planned. Installed teams disable legacy stage progression, preventing a run from completing while ignoring its accepted team. The next required implementation is execution of that plan through the standalone scheduler; this installation path does not claim scheduler execution or full automatic orchestration.

The CLI exposes `orchestrate` and `accept-team`, each requiring `--approve-scoped`. The TUI queues the main-chat message, exposes the scoped turn/review and team acceptance in its palette, displays exact proposal/review records under Options / plan and restores responses/session history after reattachment. Unsupported mode changes remain separate work. A missing explicit orchestrator configuration never selects a provider automatically.

Regression coverage uses real local CLI fixture processes and an actual daemon/socket: correct orchestrator/reviewer model routing, independent completed outputs, installation/restart, rejection of invented evidence/history, incomplete risks/cycles/write escalation, stale approval/new user context, source-changing malformed output and old contract versions. Fixture pass results are explicitly synthetic; authenticated model behavior is not established by them. Dialogue history grows without recursive request-context duplication. Each round binds its own exact input/output observations, preserving the distinction between earlier evidence and current review.

Orchestration preserves the primary attempt's active-session checkpoint and historical provider failure/diagnostic when answering a question; its separate sessions remain in the ledger. This keeps a conversation from replacing the identity needed by primary verifier recovery. Managed preflight probes now receive cancellation through each process-backed provider and the model-routing wrapper. A regression hangs the initial version probe with a same-group descendant, cancels the run and verifies lease release and absence of the descendant's delayed effect. Process-group escape containment remains an existing open limitation.

Validation of the final orchestration integration: 316 Rust tests passed, zero failed and seven remained ignored across 52 test binaries (nested harness reports excluded). The locked build, all-target Clippy with warnings denied, formatting and diff checks passed. All requirements freshness checks and 19 Python tests passed; semantic interpretation gaps remain open. Both legacy fake demos passed, including six-stage completion and rejection awaiting repair approval. New orchestration tests exercise real local fixture processes and daemon IPC, without an authenticated model call. Authenticated orchestration and Linux acceptance remain open.

## Concurrent session checkpoints and agent communication

The user's established communication requirements are now explicit in `design.md`: management consumes task reports, artifacts, verifier decisions, findings and addressed messages; the orchestrator does not need child transcripts. Every agent may address any current team member or broadcast to the other members. Hook transport is an extension of the same protocol, not a separate source of sender identity or permissions.

Native callbacks now load and mutate the latest session ledger under the store lock, committing each transition and its inbox delivery together. Stale worker/coordinator copies cannot replace that ledger through progress/final saves. Reservation order remains durable; an older completion does not steal the latest reserved session pointer. This fixes the session concurrency prerequisite for DAG execution. Artifact, quality and workspace state still require their task-specific scheduler bridge; the legacy whole-run checkpoint is not a safe parallel artifact merger.

All runtime-observed structured CLI turns advertise `agent_messages`. The sender is derived from the actually observed completed session/turn. A message has a typed direct or broadcast target, kind and text; it cannot supply a sender or masquerade as a user instruction. Broadcast freezes one separate queued record for each current other recipient. The complete outbox fanout, raw output, inbox delivery and publication receipt share one SQLite transaction. Invalid outboxes retain completed raw output and a rejection, admit no valid prefix and block false success. A failed transaction admits neither output nor messages. Deterministic turn/index/recipient IDs and recorded publication prevent duplicate fanout.

`send_message` accepts installed team agent IDs and `*` for a user broadcast. The TUI adds `b` to open the broadcast composer. Installing a team cancels unsent traffic for retired fixed-stage identities while retaining its history; existing identities keep their messages. An orchestration round's own authenticated outbox does not make its proposal stale. New user/other-task input still requires fresh review. Management context contains its own inbox and outgoing reports, not unrelated point-to-point traffic or child transcript bodies. The full local audit/session history remains available to the user.

Current transport is next-turn delivery through native CLI stdin and completed structured output. It does not install provider hooks, interrupt active work or claim live steering. Hook authentication, provider capability validation, active-turn transport and scheduling a reply when no next task turn exists remain open. The queue does not itself launch a new provider turn or approve work.

Regression tests force overlapping observed turns to finish in reverse order, inject an SQLite output-event failure, preserve a sibling's receipts across stale saves/restart, authenticate broadcast ownership and reject malformed/forged/foreign recipients atomically. Real local CLI fixture processes exercise orchestration outboxes, installation, targeted input isolation and persisted rejection. These are protocol/concurrency tests, not authenticated model judgments or full DAG acceptance.

## Historical integration order — 2026-10-14

1. Execute accepted team plans through the existing scheduler, with runtime-owned task sessions, authenticated gate evidence, source invalidation and durable checkpoints. Task prompts must use the shared inbox/outbox channel and report-based management without reading child transcripts. This completes the next part of the proposal/review/acceptance path; it does not replace the full product scope.
2. Complete conversational intent-to-command handling and remaining TUI controls, preserving explicit permissions and reviewed substantive management decisions.
3. Add managed workspace/integration lifecycle, then autonomous progression within its real capability boundary and exact-ID recovery.
4. Complete semantic requirement acceptance and the remaining product scenarios, then calibrate against shared-input baselines.

No passing narrow test or historical self-host experiment closes those remaining requirements.

The scheduler bridge must merge concurrent session/evidence checkpoints rather than overwrite cloned whole-run ledgers, use role-specific contracts and authenticate actual gate receipts. Task/source dependencies and write ownership must survive invalidation and cancellation. Running the old six-stage pipeline once for every team task would duplicate unrelated work and would not implement the accepted DAG.

## Graph, annotations and task execution continuation — 2026-10-14

The user clarified the agent panel requirements: a spatial node/edge graph in the style of MindNode/Neo4j, a corner overlay showing the currently executed step, and comments on every inspectable agent-panel element with orchestrator reactions. These requirements are normative in design §2 and §8. The source ledgers were regenerated without treating their structural freshness as semantic acceptance.

The panel now draws two-dimensional node boxes and relationship edges, follows selection through the viewport, navigates by direction, preserves coordinates across event reorder/status updates, groups by stage and handles disconnected cycles and large teams without recursion. The lower-right overlay derives activity from persisted task/session observations. Executor and verifier activity are projected separately; waiting verification is not shown as an active verifier. Terminal sanitization, ASCII/no-color operation and narrow layouts remain supported.

Version-bound annotations cover agents, tasks, relations, sessions, turns, artifacts, findings, messages, evidence, checks and gates. The daemon assigns the local user identity, persists the original element context and refuses changed targets. TUI comment composition is separate from chat drafts; comment target selection pins displayed references. Resolution/deletion retain historical bindings and reactions. Orchestration review v2 requires a linked answer, question or valid task proposal for every unresolved comment and independent COMMENT checks. Reviewed reactions commit with their round; unreviewed replies do not resolve comments or approve work. Comment context for sessions/turns retains identities and receipt references rather than child transcripts. Supervised turns still require their existing explicit scope approval; hook/live intent scheduling remains open. Completed/cancelled runs accept annotations and separately approved read-only dialogue, preserving task history, terminal status and historical task checklists. Each new dialogue uses the current independent review contract and cannot reopen task execution or install a replacement team. Installed management agents retain their previous native sessions in history and display the new observed reply/review session. The progress overlay distinguishes answering from independently reviewing the answer, including pause.

Audit of the initial task bridge found false-success defaults, filled-in criterion passes, discarded negative findings, hidden provider fallback, manufactured gate hashes, unchecked writes and swallowed SQLite checkpoint errors. Those paths were replaced with strict decoding, complete common/role/criterion/plan/risk/gate checks, actual receipt bindings, independently observed native authorship and exact configured routes. Gate assessments derive from persisted typed reviews, never a task's status alone. Native output including proposed edits is retained and hashed as the artifact. Task completion commits artifact/evidence/review data together with its scheduler checkpoint. Mediated edits use the existing pinned-path validation, compare-and-write and durable write-ahead receipts. Cancellation cannot be overwritten by wave completion; checkpoint failure cancels sibling turns and preserves reconciliation.

The bridge is still partial. It must implement full role-specific context/solution artifacts and runner plans, source/dependency refresh after legitimate edits, executor/uncertain-checkpoint recovery (verifier-only recovery is now implemented), supervised result acceptance at the appropriate stage boundary, policy-mode transitions without invalidating unrelated work, and authenticated acceptance beyond the synthetic protocol fixtures. In particular, a task cannot promote a changed source snapshot by relabeling old artifacts. Current conservative stale-source blocking must be replaced with actual independent refresh/revalidation, not waived.

Priority order remains:

1. P0: complete task proof/freshness/recovery and stage acceptance before allowing general DAG completion.
2. P1: finish intent/control scheduling, managed worktree/integration, local autonomous effects and exact-ID continuation.
3. P1: complete semantic review of the original requirements and software/research/document/analysis product scenarios.
4. P2: cross-platform/provider acceptance and representative baseline calibration.

Focused fixtures exercise malformed/contradictory reviews, missing evidence/typed reports, out-of-scope writes, SQLite checkpoint failures, native cancellation, inbox isolation, persistence and reviewed comment responses. They prove protocol/state behavior, not real-model judgment. Final validation for this continuation is recorded below after the current-state checks.

### Validation of this continuation

- Locked all-target Rust suite: 345 passed, zero failed, seven ignored across 55 outer test binaries; nested fixture subprocess reports are excluded. Six ignored tests require historical snapshots and one records the open process-group escape containment case.
- After the final design/catalog freshness update, the affected library/runtime, annotation and TUI targets passed 41 tests. The final TUI control-state projection then passed its 17 TUI/session tests. No test assertion, deadline or gate was relaxed.
- `scripts/cargo.sh clippy --all-targets --locked -- -D warnings`, locked build, formatting and diff checks passed on the final code.
- Python unittest discovery: 49 passed. All three source/requirements/audit/gap freshness checks passed: 1145 source fragments, 1438 interpretation units, 513 reviewed atoms and 473 remaining parent semantic reviews.
- Dependency inventory: 218 locked packages; three active packages still lack locally inventoried license source files. This continuation adds no dependency and does not claim a completed legal review or vulnerability scan.
- Native provider behavior in new tests comes from real local CLI fixture processes, including daemon/socket communication and cancellation. No new authenticated model run or Linux acceptance is claimed. Structural freshness and these local checks do not establish full design acceptance; the P0/P1 work above remains required.


## Task verifier recovery continuation — 2026-10-14

`retry-task-verification STATE_DIR RUN_ID TASK_ID --approve-scoped` replaces only the selected incomplete verifier turn after explicit workspace/session reconciliation. The IPC command is `retry_task_verification`; TUI snapshots advertise only eligible task/scope pairs and the command palette pins the exact task and scope through confirmation. The approval binds the full durable run, including current policy/configuration, native output history, artifact/evidence, pending messages and reconciliation state. Changed inputs, outdated scope, active work, ambiguous chronology, missing/authorship-mismatched output, stale or invented receipts and uncertain scheduler checkpoints refuse recovery.

The scheduler selects one verifier without scheduling unrelated executors, even in autonomous mode. It keeps the original executor attempt, artifact, repair history, source snapshot and edit receipts. Each explicitly approved replacement appends verification history and gets a fresh independently observed logical/native session. Prior failed sessions and raw outputs stay unchanged, including failures before the scheduler had bound an agent session. Incomplete prior verifier outputs are supplied to the replacement reviewer and do not waive any acceptance checks. Previously sent messages whose delivery is unknown are retained as unknown and excluded from the replacement packet. Idempotency and the approval transaction prevent duplicate launches; failed SQLite approval does not start a provider process.

A substantive rejection, failed criterion/check, unresolved finding (including a malformed findings field), synthetic report, mismatched artifact/input/checklist claim or invented evidence cannot be discarded through this command. An independently rejected task stays on its repair path. Malformed negative reports still require explicit adjudication/repair integration; this continuation does not turn them into positive reviews. Unparseable completed output is not silently interpreted as an empty report.

Daemon startup now also transitions interrupted team executor/verifier states to blocked while preserving artifacts and session history, requiring reconciliation before a replacement. Replacing an interrupted executor, resolving an uncertain scheduler checkpoint, refreshing dependencies after legitimate edits, full role-specific artifact/runner contracts and supervised stage-result acceptance remain open P0 work. General worktree/integration, autonomous effects, native exact-ID continuation, complete semantic/product acceptance and M5 calibration remain required by the original objective.

Focused validation: scheduler/team/TUI suites passed 64 tests, including real local native CLI fixture processes and an actual daemon/socket/CLI recovery path. The restart test exercises the persisted crash boundary; it does not claim authenticated provider resume or a killed-daemon acceptance run. All three requirements freshness checks passed; semantic review remains partial. Python discovery passed 49 tests. Full final Rust regression and lint validation are recorded after completion below.


The first final all-target invocation completed 43 outer binaries with 313 passes, six ignored and one installed-provider smoke failure. Claude's default executable changed during validation to `2.1.299`; the adapter correctly rejected it outside its pinned `2.1.296` contract. The retained `2.1.296` executable is available for a process-local PATH pin. No accepted version range, test assertion or global CLI symlink is changed to conceal this mismatch. The affected installed-provider smoke and all remaining regression targets passed with that process-local pin. This proves the recorded validated version, not compatibility with `2.1.299`.


The role-artifact binding described in this earlier P0 note is implemented by the continuation below; general DAG completion remains unproven: `ContextAudit` alone must not substitute for a `ContextModel`, and a planning audit must inspect the retained `SolutionModel` and independent simulations. The task bridge currently retains narrative/edit output and typed reviewer reports; the legacy provenance registration already supports model artifacts, but its stage-wide assumptions cannot be applied blindly to every Discovery task. Role-specific payloads, exact dependency artifacts and independently current model assessments must be connected before claiming general DAG gate completion. Legitimate edits also require separate pre-execution and post-edit source proof plus independent dependency revalidation; relabeling old source hashes is insufficient.


Final validation for verifier recovery:

- Complete Rust target coverage across the first all-target invocation and its remaining/affected targets: 353 passed, zero remaining failures, seven ignored across 55 outer binaries. Results are merged by target; the nested installed-provider subprocess is excluded. The initial default-PATH version rejection above remains recorded.
- After the final artifact-stage and receipt-origin binding checks, library/runtime, team-execution, TUI and session projection targets passed 48 tests. This includes source/stage/authorship tampering, stale scope, SQLite approval failure, multiple incomplete replacements, non-replayed uncertain messages, restart state and actual daemon/IPC/CLI recovery.
- All-target locked Clippy with warnings denied, locked build, formatting and diff checks passed.
- Python discovery passed 49 tests; all three requirement/audit/gap freshness checks passed with 473 parent semantic reviews still open. No dependency, policy gate, supported-version range or assertion was relaxed.
- Local CLI fixtures establish protocol/state behavior. No new authenticated model run, exact-ID native continuation acceptance or Linux acceptance is claimed; the full original design objective remains active.


## Native task model contracts continuation — 2026-10-14

Native task output now uses the explicit `task-models-v1` contract. Its immutable raw output includes nullable context/solution packages alongside the narrative, proposed edits and outbox. Missing nullable keys are rejected rather than filled in. Context roles require an actual sourced ContextModel and eight sourced context sections (intent, domain, behavior, dependencies, contracts, standards, environment and baseline), with material unknowns resolved against supplied sources. Section facts must belong to the retained model. Planner and OptionsAnalyst roles require actual distinct options, a scoped admissible selection, invariants and versioned scenario assessments. Every declared design class is accounted for, with nonempty assigned criterion coverage for passing cases; mandatory success, failure and counterexample cannot be N/A. Multiple cases per class are supported without a scenario-count cap, and signature indexes avoid quadratic comparison of large scenario sets.

The verifier receives the complete pinned packages, exact model hashes and scenario/section obligations. Its typed `model_review` binds the current packages, records independent sourced context observations and a counterexample, and supplies independently constructed scenario states/events/results. Exact copying of executor simulations, missing classes/criteria, stale versions, wrong model hashes, failed outcomes and planning-audit contradictions block acceptance. A ContextAudit alone cannot open context_ready, and a PlanningAudit alone cannot open solution_selected or plan_ready. Model packages and independent inspections persist within the atomic artifact/review/task checkpoints, alongside distinct actual native sessions and source receipts. Gate admission repeats model validation against authenticated current native output; it does not trust a stored task state or a historical gate label.

Verifier recovery also refuses contradictory model identities/selections, stale independent source facts, failed simulation outcomes and unresolved material unknowns. Those failures cannot disappear through a replacement review. The common checklist hash now includes the new task-model contract, so previously pinned runs require explicit revision instead of silently gaining new acceptance rules. Terminal historical records retain their read-only use.

This closes native role-model omissions and structural binding gaps; it does not establish the substance of real model judgment or complete M2Q/M3. Remaining P0 work includes source/effect proof across legitimate edits, independent dependency/model refresh, executor and uncertain-checkpoint recovery, actual task runner plans, broader role/family contracts and supervised stage-result acceptance. Model provenance across the legacy fixed-stage path still needs consistent integration with the richer task packages. The full worktree's semantic requirement and software/research/document/analysis acceptance, managed worktree/integration, autonomy, native exact-ID resume and calibration remain required by the original objective.

Focused evidence uses real local CLI fixture processes: current sourced context acceptance; omission/staleness/unknown/counterexample failures; actual options artifacts and distinct native reviewers; incomplete/misbound/copied simulations; durable model/review replay after reopening the store; and existing annotation/recovery/concurrency scenarios. These fixtures describe synthetic protocol/model examples and do not prove authenticated provider reasoning. Full final regression and lint results follow after completion.


Final validation of native task model contracts:

- Locked all-target Rust suite with no-fail-fast: 359 passed, zero failed, seven ignored across 56 outer binaries. Nested provider subprocess results are excluded. Actual installed-provider probes used the retained validated Claude 2.1.296 through a process-local PATH pin; no compatibility with 2.1.299 is claimed.
- The suite includes 22 team-execution integration cases and three model-contract counterexamples, including hundreds of cases in one class, conjunctive class/criterion coverage, legal N/A boundaries, required nullable transport fields, exact native artifacts/reviews and non-erased failed or contradictory model inspections.
- Python discovery: 49 passed. All three requirements/audit/gap freshness checks passed; the source ledger still has 473 parent semantic reviews remaining.
- All-target locked Clippy with warnings denied, locked build, formatting and diff checks passed on the final code.
- No dependency, gate, test assertion, time budget or supported CLI version range was relaxed. No new authenticated model or cross-platform product acceptance is claimed. The full design objective remains active.


## Managed task effects and read-only source refresh — 2026-10-14

Every native executor turn now retains a runtime-owned ExecutionBinding: exact original input bytes, source and reviewed-dependency hashes, observed logical/native session scope, proposed-output hash, write-ahead owned receipts and the actually observed after snapshot. The binding reaches Committed only together with the task's artifact/evidence checkpoint. Artifact source hashes remain the original execution hashes. Native authorship checks use those retained original inputs for the executor and current inputs for the verifier; historical effect receipts and outputs are not relabeled as current execution.

Mediated writes compare each destination against its captured expected bytes, require an unchanged physically observed workspace and admit changes to other inputs only when this wave has completed managed effect proofs. Two disjoint native writers can therefore share a wave without overwriting a stale whole snapshot. Unknown external edits, conflicting destination bytes, cancellation and policy/configuration changes block effect admission. SQLite intent commits precede filesystem replacements. A failed receipt commit preserves pending/unknown effects and requires reconciliation; no new wave is authorized to replay them.

Artifacts additionally bind exact independently reviewed parent artifacts/reviews. Source or dependency changes transition positive/unreviewed results to stale, preserving their artifacts, reviews, native history and effect receipts. Rejected results remain on their repair path. Stale gates cannot authorize downstream execution. The supervised source-refresh command is `refresh-task-sources STATE_DIR RUN_ID TASK_ID --approve-scoped` (IPC `refresh_task_sources`). Daemon snapshots and the TUI palette advertise eligible exact task/scope pairs. The refresh waits for independently current prerequisites/gates and authorizes one read-only executor turn. It archives the prior nonrejected result as a source-refresh attempt, supplies previous artifacts/effects and current sources, refuses all proposed edits, and retains the replacement as a new artifact/session. A subsequent verifier receives both source/effect history and refreshed models, performs the full role/criterion/risk/gate checks plus an explicit SOURCE_REFRESH check, and admits new gate evidence only after actual independent acceptance. Older artifacts and native turns remain unchanged. The progress projection labels source-refresh work explicitly.

Current successful runner receipts are now included in task-verifier context. Their IDs can establish runner-required gates; artifact-read, model claims and historical baseline evidence cannot substitute. The native writer recovery fixture uses an actual explicitly scoped local check of the resulting file before accepting the refreshed writer. Automatic role-specific runner-plan scheduling still remains P0 work.

This implements supervised managed-edit invalidation and read-only source/dependency refresh. It does not complete general DAG acceptance, autonomous refresh policy, effect/outcome recovery after uncertain SQLite checkpoints, interrupted-executor recovery, native exact-ID resume, family-specific artifact/runner contracts or stage-result approval. Legacy fixed-stage model/provenance integration and the semantic/product acceptance audit remain open. Default managed worktree/integration, cross-provider/platform acceptance and calibration are also still required by the full original objective. Source validity remains conservative over all inputs actually supplied to a task; narrowing context/dependency scope safely requires separate work.

Focused scheduler/runtime/TUI validation passed 78 tests, including real local native CLI processes, actual filesystem before/after effects, two disjoint writes in one wave, rejected repeated writes during refresh, immutable old input/effect/native history, source-refresh approval/receipt failures and actual daemon/socket/TUI/CLI access. No test assertion, deadline, gate or supported CLI version was waived. These native CLI fixtures establish protocol/state behavior rather than authenticated model judgment. Full final regression/lint results follow below.


Final validation of managed effects/source refresh:

- Locked all-target Rust suite with no-fail-fast: 367 passed, zero failed, seven ignored across 56 outer binaries. Nested installed-provider subprocess reports are excluded. The installed-Claude smoke used the retained 2.1.296 executable through a process-local PATH pin; supported versions and global CLI settings were unchanged.
- Focused scheduler/runtime/TUI suites passed 78 tests, including 29 team-execution cases and real local filesystem/native process/daemon/source-refresh scenarios. The writer refresh acceptance uses an actual successful current local runner receipt, not a fabricated pass or an artifact-read substitute.
- Python discovery passed 49 tests; all three requirements/audit/gap freshness checks passed. Semantic review still has 473 parent interpretations open.
- All-target locked Clippy with warnings denied, locked build, formatting and diff checks passed on the final code.
- No dependency, deadline, assertion, quality gate or supported-version restriction was relaxed. These fixtures do not establish authenticated model judgment, native exact-ID recovery or cross-platform/product completion. The full original design objective remains active.


## Approved task check plans and observed execution — 2026-10-14

Each task can now declare a check_plan with stable check ID, exact argv, assigned criterion/gate coverage and a rationale. Bootstrap uses the explicit RunSpec check command for runner-required roles when supplied; model proposals may specify more appropriate commands. Native proposal/installation validation refuses missing required gate/criterion coverage. The independent orchestration contract requires one TASK_CHECK assessment for every proposed command, with explicit inspection of argv, effects/policy, resource assumptions and meaningful assertions/oracle. Existing scoped team-wave approval binds the installed plan, including every command byte; executor output cannot replace this plan or turn a claimed tool run into an execution request. The common checklist hash includes task-check-plan-v1 and old active checkpoints require explicit revision.

Before a native task verifier starts, runtime executes the assigned plan against the exact current task artifact and pinned source snapshot. Local-check policy is enforced. A durable task_check_intent binds task/attempt/artifact identity, the plan, source/configuration hashes, current local-user authorization and input hash before process launch. Actual supervised-process output, exit status, before/after input binding and evidence are committed as task_check_finished. A successful process is only performed evidence; independent criterion/assertion quality still requires review. RUNNER checks must cite their exact invocation receipt and agree with its actual result. Runner-required gates must cite successful checks explicitly assigned to that gate in the approved task plan.

Completed matching checks, including failures, are retained without automatic replay. A verifier-only format recovery reuses the same completed successful invocation rather than launching its command again. A failed mandatory check cannot be erased through verifier replacement. Failed output is available to an independent rejection; a lying positive review is blocked. Input mutation, cancelled execution, process failure or uncertain SQLite completion preserves Intent/Unknown and reconciliation. Such an invocation cannot silently rerun after restart or reattachment.

Check admission serializes checks sharing uncontrolled resources within one daemon, including the existing fixed-stage and supplemental check paths. Waiting admission observes cancellation without waiting for the active check to finish. Already admitted checks follow existing pause/cancel semantics. This is conservative coordination, not an OS sandbox or cross-daemon/external-process isolation guarantee. The progress overlay now shows the actual pending check ID/task while the native verifier is waiting, rather than showing an unstarted verifier as active.

Current coverage is automatic checks of a retained current artifact before its task verification. Remaining P0/P1 work includes independently prepared baseline-before-write checks, richer role-specific execution environment/resource plans, result-stage approval and complete family acceptance, recovery/adjudication of unknown check/effect outcomes, interrupted-executor recovery, autonomous progression and native exact-ID continuation. Managed worktree/integration, semantic/product requirement acceptance, real-provider/platform acceptance and calibration remain required by the original objective. No complete-MVP claim follows from this integration.

Focused protocol/state evidence includes actual local subprocess exit/output, exact receipt reuse during verifier recovery, observed failure followed by independent rejection, blocked fake pass labels, input mutation, SQLite intent/receipt failures, cancellation without a verifier launch, missing runner plan before installation and truthful TUI check progress. The local CLI fixtures describe synthetic model judgments; authenticated model correctness remains unproven. Final full regression/lint results follow below.


Final validation of approved task checks:

- Locked all-target Rust suite with no-fail-fast: 375 passed, zero failed, seven ignored across 56 outer binaries. Nested installed-provider subprocess reports are excluded. Actual installed-Claude probes used the retained validated 2.1.296 executable via a process-local PATH pin.
- After the final Clippy-only conditional cleanup, the affected native orchestration/installation target passed its 10 integration cases again. No semantic condition or assertion was weakened.
- Actual task-check cases cover durable exact command/receipt reuse, required failure, blocked lying pass, changed input, SQLite intent/completion failure, cancellation with no native verifier launch, required-plan rejection before installation, interruptible shared-resource waiting and truthful check progress. Existing source-refresh/writer acceptance now obtains its successful local check automatically from the approved task plan.
- Python discovery passed 49 tests; all three requirements/audit/gap freshness checks passed. Semantic review still has 473 parent interpretations open.
- All-target locked Clippy with warnings denied, locked build, formatting and diff checks passed after the conditional cleanup.
- No dependency, deadline, runner/quality gate, test assertion or supported CLI version was relaxed. Real model judgment and full baseline/environment/stage/family/product acceptance remain unproven. The full original design objective stays active.

## Task baseline before native execution — 2026-10-14

Reviewed task check plans now distinguish `before_execution` from `before_verification`; old serialized plans retain the latter default and expected exit zero. An explicit nonnegative `expected_exit_code` may describe a failing baseline reproduction only in the former phase. Only before-verification checks count toward required positive runner-gate and criterion coverage. The orchestration reviewer sees the exact phase, command, exit expectation, resource assumptions and coverage rationale; scoped team approval binds these bytes.

The scheduler persists its selected wave before runtime serially observes every writing/executing task's planned baseline, then starts native workers. No sibling worker can change inputs before another baseline in that wave is captured. Intent and actual output receipts retain the original input snapshot, source/configuration identity, task/attempt, authorization, command and before/after hashes. A baseline has no fabricated task artifact ID. Actual process success remains false for an expected nonzero exit; `BASELINE:<id>` passes only when the declared reproduction outcome was observed. A signal, unknown code, truncated output, changed pinned input or uncommitted completion cannot satisfy this expectation. Known unexpected outcomes block that executor before its native session starts; uncertain observation stops the wave and requires reconciliation without replay.

Independent task review authenticates historical baseline receipts against the original completed native execution/input binding. The baseline cannot replace a current positive check or a successful receipt for a required runner gate. Read-only source refresh cites the preserved original baseline rather than running a reproduction against the already changed solution; a repair begins a new attempt's baseline. Verifier-only recovery may reuse an authenticated expected failed baseline, but cannot reconstruct a missing one or hide a mismatch. The master checklist includes `task-baseline-v1`.

The signal fixture exposed an existing supervisor defect: signalled leaders were converted to exit one. After group cleanup the guardian now reproduces the actual termination signal, and check receipts include it separately from nullable exit status. The wave error path also retains `checkpoint_uncertain` durably instead of losing the scheduler's flag during final worker cleanup. The TUI labels actual pending baseline checks and shows selected executors waiting until the baseline barrier finishes.

This implements scoped, reviewed baseline execution, not general environment isolation or substantive model acceptance. Checks are declared by the approved plan; the role contracts still need an explicit bugfix classification and enforcement of design §12.2.5 (original reproduction or an independently justified alternative when reproduction is unavailable), tied to the post-change behavioral evidence. Optional baseline capability alone does not close that requirement. P0/P1 priorities remain role-specific execution environments/resource declarations, adjudication/recovery of unknown checks/effects and interrupted executors, supervised stage-result approval, full task-family acceptance, managed worktree/integration, intent/control scheduling, autonomy and native exact-ID continuation. Semantic requirement acceptance, real-provider/platform acceptance and representative calibration remain part of the full original goal.

Validation is recorded below after the final regression and lint run. Synthetic native-model fixtures test protocol/state/effect behavior; they do not establish real-model judgment or complete MVP acceptance.

Final baseline validation:

- Locked all-target Rust suite with no-fail-fast: 386 passed, zero failed, seven ignored across 56 outer binaries (nested installed-provider reports excluded). Installed-Claude probes used the retained validated 2.1.296 binary through a process-local PATH pin.
- The final TUI-only correction for mixed executor/verifier waves passed all 21 TUI/session tests after the full suite. Final all-target Clippy with warnings denied and locked build passed.
- Python discovery passed 49 tests; all three requirements/audit/gap freshness checks passed. The 473 outstanding parent semantic interpretations remain open.
- Baseline cases observe expected failure before actual executor admission, known unexpected success/failure and signal rejection, before/after input hashes, original-input authentication, missing/corrupt receipt refusal, unknown SQLite/cancellation outcomes with no worker launch or replay, reuse during verifier replacement, preservation across read-only refresh and whole-wave capture before two disjoint writers. A positive review supported only by historical reproduction is blocked. The supervised-process fixture independently checks nullable exit status and the actual termination signal.
- No dependency, deadline, test assertion, quality gate or supported CLI version was relaxed. Authenticated model judgment, Linux/full-provider acceptance, bugfix classification/alternative-proof enforcement, environment isolation and the remaining stage/family/product requirements are still unproven. The full original objective remains active.

## Native software change classification and bugfix proofs — 2026-10-14

Software implementer, refactorer, test-implementer and integrator tasks, and any software task declaring write scopes, now require an explicit `change_contract` before native proposal acceptance/installation and worker admission. Candidate templates retain null rather than inventing a default feature classification. The native proposer supplies a sourced classification (`feature`, `bugfix`, `refactor`, `maintenance`, `mixed`) and rationale. `TASK_CHANGE` and one `BUGFIX_PLAN` per defect require independent proposal review against the original request, source bytes, assigned criteria, commands and oracle; relabeling a fix to waive proof is a review defect. The scoped team authorization includes these exact contract bytes.

Each bugfix declares an ID, original defect, assigned criteria, oracle, before proof and designated post-change check. A bugfix-only task must account for every assigned criterion; mixed changes preserve the identified defect obligations alongside existing full positive runner/criterion coverage. The original-state check must be `before_execution`; the designated post-change check must be `before_verification`, expect zero and cover the defect's criteria. Reproduction declares the observed command outcome rather than equating an exit code with substantive correctness: a measurement can exit zero and still expose a defect. If reproduction is unavailable, the plan must state the concrete reason, assigned sources and alternative method and schedule an actual original-state investigation. Missing execution or a known unexpected/unknown outcome never supplies a default pass.

Acceptance now requires a typed `quality.change_review` bound to the exact contract hash/classification, actual current source-snapshot observation and the observed native verifier/artifact. Each defect has an independent original-result assessment, oracle inspection, counterexample, exact before/after invocation receipt IDs and (when applicable) independent evaluation of the reproduction limitation and alternative proof. `CHANGE:classification` cites its source evidence; `BUGFIX:<id>` cites both designated invocation receipts. Historical input/receipt authentication is retained across read-only refresh; the post-change proof must belong to the actual current artifact/source. A narrative claim or positive process label cannot supply these missing bindings.

Verifier-only recovery can fill omissions with a new independently approved read-only review, but rejects prior claims of another contract/classification, invented/duplicate defect identities, reassigned original/post-change receipt IDs and stale or failed post-change evidence. Original outputs and effects are retained. The master checklist includes `task-change-proof-v1`. The legacy fixed-stage path has no approved native task change contract and refuses a fabricated native change-review package; equivalent change classification/before-after proof enforcement for that path remains open. The substantive truth of classification, oracle and alternative reasoning still requires real independent model/product acceptance, beyond protocol fixtures.

Source hashing now derives canonical hashes directly from the selected current or retained original input map and immutable spec sources. Receipt/execution authentication no longer clones the complete session/event/artifact ledger solely to hash historical inputs. It retains the same source identities, serialized source bytes and input/configuration checks.

Remaining priorities preserve the full original scope: legacy model/change-proof integration; role-specific environment/resource plans and isolation; recovery/adjudication of unknown check/effect outcomes and interrupted executors; supervised stage-result approval; complete family/product acceptance; managed worktree/integration; intent/control scheduling, autonomous effects and native exact-ID continuation; semantic requirements acceptance, real-provider/platform evidence and representative calibration. No full-MVP or complete-design claim follows from this integration.

Final validation is recorded below after the current-state regression. Native-model fixtures are synthetic judgments; actual local command/file/receipt/daemon behavior is observed separately.

Final change-proof validation:

- Locked all-target Rust suite with no-fail-fast: 391 passed, zero failed, seven ignored across 56 outer binaries; nested installed-provider reports are excluded. Native team execution passed all 46 cases. Actual installed-Claude probes used the retained supported 2.1.296 executable through the process-local PATH pin.
- Actual before/after file/command cases cover failed reproduction, original measurement returning exit zero, observed unavailable entrypoint with source-based alternative proof, independent current acceptance after read-only refresh, exact retained original receipts and one-shot baseline execution. Missing classifications/plans, unbound/missing defect reviews, swapped post-change receipts and unassessed alternatives stop approval or acceptance. Contradictory contract/receipt claims cannot obtain verifier-only replacement. The fixed-stage test refuses an unapproved native change-review claim.
- Locked build, all-target Clippy with warnings denied, formatting and diff checks passed. Python discovery passed 49 tests; all three requirements/audit/gap freshness checks passed. Semantic review still has 473 parent interpretations open.
- No dependency, deadline, assertion, required gate or supported CLI version was relaxed. The fixtures prove protocol and observed local execution, not actual model judgment, full classification correctness, full family/product acceptance or Linux acceptance.
- The next isolation/recovery audit must verify guardian lease inheritance in parallel task threads: `LeaseScope` uses thread-local state, while the inspected `work_team` entry currently initializes it on the coordinator. Targeted parent-death/cleanup evidence for task-worker checks is required before asserting that those guardians retain workspace ownership after daemon death. This is an open P0 item, alongside the remaining requirements above.

## Parallel task guardian lease inheritance — 2026-10-14

The task-wave coordinator previously installed `LeaseScope` only on its own thread. Scheduler workers run on newly spawned threads, whose thread-local lease list was empty. Their CLI/check guardians therefore did not inherit the workspace descriptors: killing the daemon could release workspace ownership while a task process group still awaited cleanup.

`work_team` now retains the lease descriptor list for the wave lifetime and installs a scope inside every executor/verifier worker before provider preflight, native turns or local checks. The coordinator's scope still protects pre-wave baselines. The underlying lease files remain owned until scoped workers join; each guardian duplicates its own references before launching the external process. Tools still do not inherit the lease descriptors.

A real-daemon regression stops an owned guardian, kills the daemon and proves that a competing runtime cannot acquire the workspace while cleanup is suspended. It exercises task executors, native verifiers and before-verification local checks; symlink aliases and nested workspaces are also refused. After resuming the guardian, the group is terminated, its direct child is reaped, descendant heartbeat writes stop and ownership becomes available. Restart preserves blocked/reconciliation state and unknown check intent without launching another native turn or rerunning the command. The test reproduced premature release before the fix and passed all three boundaries after it.

This closes the inspected thread-local inheritance gap, not general OS containment or resource isolation. Process-group escape containment, shared external resources, environment plans, unknown-effect adjudication and interrupted-executor recovery remain required, alongside the full original design scope. No stage/family/product or real-provider/platform acceptance is inferred from this lease regression.

Final current-state validation is recorded below after the regression run.

Lease validation and remaining regression concern:

- The targeted real-daemon test failed before the fix with premature workspace release, then passed executor, verifier and local-check boundaries after the fix. The final full native-team target passed all 47 tests, including retained alias/nested locks, completed cleanup, blocked startup and no native/check replay.
- Locked all-target regression: 390 passed, two failed, seven ignored across 56 outer binaries. Both failures were timing bounds in the existing fixed-stage path: `core::supervised_full_cycle_requires_each_start_and_result_approval` and `drive_follow::follows_completed_run_without_new_approvals_or_runs`. This run is not a green full-regression result.
- A serial rerun of both affected modules passed all five driver-follow tests; the core full-cycle case still timed out without changing its five-second bound. Added diagnostics identify the stage and actual session states. Independent reruns timed out at Delivery with completed sessions and at Validation with pending verifier delivery. Process samples show full-run JSON decoding under the store mutex and native prompt/session checkpoint work; host process observations also showed unrelated workloads using over 1300% CPU. These observations do not establish a single implementation cause or justify hiding the failure. Runtime/prompt/storage performance and a fresh green full regression remain required.
- Python discovery passed 49 tests and all three requirement/audit/gap freshness checks passed; 473 parent semantic interpretations remain open. Locked build, final all-target Clippy with warnings denied, formatting and diff checks passed. No test assertion, deadline, quality gate, dependency or supported provider version was relaxed.
- No general OS containment, Linux acceptance, substantive model/family/product acceptance or complete-MVP claim is made. The full original design goal remains active; the unresolved regression and other P0/P1 requirements remain work to complete.

## Authoritative decoded snapshot reuse — 2026-10-14

The preceding core timing failure was reproduced without relaxing its five-second bound. Process samples showed repeated full `Run` JSON deserialization under the store mutex during polling and session checkpoints, along with prompt/checkpoint work. Host CPU contention remains visible; the samples do not establish that it is the only source of latency.

`Store::get` now retains only the last decoded snapshot. Every lookup still reads the actual database state and compares its complete bytes; it does not trust an unchanged `state_hash`, revision, event cursor or externally supplied metadata. Equal bytes reuse a clone of the retained value rather than decoding the full JSON again. Different bytes are decoded before replacing the retained snapshot; invalid JSON and missing rows still return errors. Returned values are independent copies. A successful save updates the retained snapshot only after the state/event/idempotency transaction commits; a failed save cannot install an uncommitted value or response.

Focused storage regressions exercise external edits with an unchanged digest, caller-local mutations, invalid JSON, deleted rows, transaction rollback and reopened committed state. All 34 core tests, including the unchanged supervised six-stage acceptance test, passed in the focused serial run after the change. Full default-parallel regression is recorded below after it finishes. This does not remove prompt/storage performance or the remaining design work from scope.

The default-parallel rerun still exposed timing pressure and also revealed two distinct races: a BSD test listener's accepted stream could remain nonblocking, and worker registry removal could precede workspace-lease release. The IPC fixture now matches production's blocking connection, bounded I/O timeouts and buffered complete JSON frame; its stale-snapshot/read-only assertions are unchanged. Completion paths for team waves, orchestration, supplemental checks and fixed-stage execution/verification now release workspace ownership before publishing the worker as idle while holding the store lock shared by dispatch. Cleanup and ownership checks remain enforced; clients need not race a final descriptor drop after the worker disappears.

After these fixes, all 12 library, 34 core, two storage and five IPC transport cases passed in focused runs. The final full regression uses two test-harness threads to reduce contention on this shared host; tests that create concurrent runtime work still exercise their actual internal concurrency. No assertion, time bound, required check or product concurrency capability is removed. The default-parallel timing failures are retained as historical evidence rather than relabeled as passes.

Final snapshot/completion validation:

- Locked all-target Rust regression with no-fail-fast and two test-harness threads: 394 passed, zero failed, seven ignored across 57 outer binaries; nested installed-provider reports are excluded. The native team target passed all 47 cases. Installed-Claude probes used the retained supported 2.1.296 executable via process-local PATH.
- The unchanged supervised six-stage test, completed-run follow, stale-control IPC and both immediate task-recovery cases passed. The source snapshot cache tests prove external edit/deletion/corruption visibility and transactional rollback; cleanup/guardian tests still retain workspace ownership through actual process cleanup and refuse automatic replay.
- Locked build, final all-target Clippy with warnings denied, formatting and diff checks passed. Python discovery passed 49 tests; requirements/audit/gap freshness checks passed. Semantic interpretation remains partial with 473 parent reviews outstanding.
- This is a green controlled-concurrency regression, not proof of arbitrary-load latency or a relabeling of the retained default-parallel failures. No test assertion, timeout, quality gate, dependency or supported CLI version was relaxed. Broader runtime/prompt/storage performance and representative calibration remain open, as do legacy change-proof integration, environment/resource isolation, unknown-effect/executor recovery, stage-result approvals and the remaining full original design requirements. The original objective stays active.

## Supervised native stage-result acceptance — 2026-10-14

Installed supervised teams now distinguish independently verified task results from explicit user acceptance of the logical-stage package. `accept_team_stage` binds the run/spec/team revision, current checklist/configuration/source hashes, exact selected task records/attempts, retained artifact IDs/hashes, independent review hashes and dependency bindings. Native completed author/verifier identities, original execution/effect lineage, exact original/current native evidence receipts, all expected task/check-plan/quality checks and criterion coverage are revalidated before offering or accepting a package. Stale or corrupted evidence cannot obtain a new approval merely from a `Verified` scheduler state.

Acceptance is a durable user action; it does not launch provider work or approve the next wave. Cross-stage task dependencies and owned gate results require current user acceptance in supervised mode. Same-stage internal work remains available through its separately approved waves. Explicit task subsets support independent branches and milestones without imposing a whole-project waterfall. Adding a result acceptance invalidates the previously displayed wave scope, so an approval of a previously empty wave cannot silently authorize newly ready work. Unchanged result packages survive unrelated wave counters and retain their history across daemon restart. Source/configuration/task/result/dependency drift invalidates affected approval bindings. Source freshness remains conservative over the actual supplied source scope.

The daemon/IPC exposes bounded stage counts/scopes and paginated `stage-results` documents bound to the exact run document hash; the TUI palette opens a scrollable, frozen result card with result text, edits, criteria and exact artifact/review identities. Refresh does not substitute new results or scope into an open confirmation. CLI `accept-team-stage STATE_DIR RUN_ID STAGE --approve-scoped` explicitly accepts the current stage package; IPC accepts the exact listed task IDs or an explicitly selected subset. The workspace lease stays held through the acceptance transaction. Failed SQLite commit preserves the prior run and approvals with no provider work.

Final native completion requires all independently current task results, every supervised result acceptance and every applicable exact qualified quality gate. The previous short gate alias could allow design evidence to satisfy a missing delivery phase of the same gate; final checks now require each exact phase and positive current receipts. Gate/result authentication is evaluated once per task within each immutable control observation instead of repeated for every gate. This is scoped recomputation, not a persistent cache or waiver of native validation.

The `team-stage-result-v1` checklist marker requires explicit revision of older active runs. Existing synthetic team fixtures now simulate the user by sending real result-acceptance commands; separate manual tests prove that the runtime itself never auto-accepts these results. One initial regression correctly failed its obsolete post-review `Ready` assertion; it now asserts `AwaitingResult` and explicitly accepts before expecting the next stage to start. The first scheduler test omitted its mandatory session-start receipt; it now uses the established receipt-emitting fixture, preserving the runtime's receipt requirement.

Validation of the final state is recorded below once the full regression finishes. Focused stage/scheduler/TUI cases already passed: manual cross-stage blocking, partial package acceptance, stale/forged result rejection, failed approval commit, real daemon/IPC/CLI acceptance and restart, and frozen confirmation content. These local protocol fixtures do not establish authentic model judgment, full task-family acceptance or completion of M3/MVP.

Next priorities retain the full original scope: P0 interrupted-executor and uncertain check/effect recovery/adjudication; P1 legacy model/change-proof integration, explicit execution environment/resource plans and isolation, complete role/family/product acceptance, managed worktree/integration, intent/control scheduling, autonomy and native exact-ID continuation; semantic review of all remaining requirements and real-provider/platform evidence; representative M5 calibration. No whole-design completion is claimed.

Stage-result validation checkpoints:

- Full locked all-target regression before the final pagination/UI-stage-label follow-up: 400 passed, zero failed, seven ignored across 57 outer binaries. This is not relabeled as the result of the later transport changes.
- After the pagination change, focused stage targets passed seven cases, including real daemon/IPC/CLI acceptance, exact document/run-hash binding, refusal of pagination against an obsolete package hash, partial acceptance and restart, malformed/native receipt rejection and frozen result cards. The serializer regression constructs a package larger than the 4 MiB frame limit while its control summary stays below 1 KiB; detailed data is available through the bounded existing chunk reader.
- Final library, IPC transport, TUI and session projection targets passed 41 cases. The confirmation test also proves that the displayed stage is the selected result package's stage rather than the run's legacy stage cursor.
- Final all-target Clippy with warnings denied, locked build, formatting and diff checks passed. Python discovery passed 49 tests; all three requirements freshness checks passed with 473 parent semantic reviews still open.
- A final locked all-target regression of the complete current state is running with two test-harness threads and the retained supported Claude 2.1.296 executable through process-local PATH. Its result must be recorded after the exact live job finishes. No deadline, assertion, quality gate, provider range or dependency was relaxed.

The next recovery audit also needs to cover the existing `Reconcile` branch's workspace-lease lifetime through its SQLite commit. Its lease is currently scoped inside the command arm, unlike the new result acceptance's outer lease. Unknown task-execution/check records and uncertain scheduler checkpoints remain retained but lack complete adjudication/replacement paths; observing workspace files is not evidence that every external effect is known. These are concrete remaining P0 observations, alongside all previously listed design requirements.


## Reconciliation commit ownership and completed-executor restoration — 2026-10-14

The final stage-result all-target run finished with 400 passing Rust cases, one failed case and seven ignored across 57 outer binaries. Its stage/task/native targets passed. The failure was `verifier_selfhost_benchmark::independent_benchmark_counterexamples_and_measurement_checks`: the nested per-child RSS/accounting cases returned nonzero for both platform accounting variants with empty stderr. An unchanged focused rerun passed both variants. The assertion now retains stdout as well as stderr so another failure includes the actual structured failure/child observations; no benchmark deadline or oracle was relaxed. This full regression is not recorded as green.

The reconciliation audit reproduced a workspace-ownership gap at the actual SQLite write boundary. `Reconcile` acquired its lease inside the command arm, then released it before `Store::save`. A competing runtime/state directory could therefore acquire the same workspace while the reconciliation transaction was waiting or committing. The command now retains the outer lease through the complete state/event/idempotency transaction. A library regression holds a real SQLite writer lock, observes the dispatch connection's actual busy callback, tests competing workspace acquisition, then releases the writer and reaps the dispatcher before asserting. It failed before the fix and passed afterward for both success and injected event-commit failure. Rollback retains the prior state/events/request response; workspace bytes are never replayed.

`restore_task_result` implements the specific completed-executor checkpoint case: the native turn and output already committed and its execution record is durably `Applied`, but the scheduler outcome/artifact transaction failed. After explicit workspace reconciliation, a separate hash-bound user action authenticates the exact owner/session/turn, provider/configuration/stage/input scope, original source/model/effect lineage, current dependency bindings and each retained owned after-write state. Eligibility also validates the original baseline/check context on an isolated prepared state; missing or unknown original baseline cannot be fabricated. The action atomically retains the same native raw output as the artifact, commits its known execution binding and records a user restoration receipt/approval. It launches no provider, local check or filesystem effect and preserves the original executor attempt, native ledgers and outboxes. It does not produce a positive review, quality gate or stage acceptance.

Scheduler uncertainty is cleared only when every interrupted executor has retained authenticated output/effects, no task remains running/verifying, and there are no unresolved active turns or unknown task checks. Otherwise it remains a barrier. Current source/dependency invalidation is applied to the restored artifact and existing tasks; a completed writer's original result can therefore remain stale and require independently reviewed read-only source refresh. Restoration cannot label pre-change sources as current or discard a substantive rejection.

The TUI offers frozen output/effect cards for eligible tasks. CLI `restore-task-result STATE_DIR RUN_ID TASK_ID --approve-scoped` and IPC `restore_task_result` require the same explicit scope. Snapshot controls expose only a small eligible count, while paginated `task-recovery` documents carry the scopes bound to the exact run document hash. Failed mutation commit preserves the pre-restoration state and known native output without another invocation. Restart preserves the restoration history and artifact.

Focused fixtures exercise a real failed executor-result SQLite checkpoint, then recover through the runtime/daemon/IPC/CLI. They verify exact output/session/turn/attempt retention, idempotence, no repeated baseline/native/write effects, separate actual verification, stale-source refusal to progress, original managed-effect receipts, corrupted/native-unobserved/ambiguous input and scope rejection, missing/unknown baselines, stale physical inputs, transaction rollback and restart. These are local protocol fixtures; they do not establish the substantive correctness of model judgment or complete general executor recovery.

Validation of the current continuation will be recorded after the remaining checks. Python discovery currently passes 49 cases and all source/audit/gap freshness checks pass; 473 parent semantic interpretations remain open. The full original objective stays active. Remaining P0 includes incomplete native exact-ID continuation/replacement with an explicit handoff, observation/adjudication of `Intent` or conflicting/unapplied effects, unknown check outcomes and scheduler uncertainty without a completed recoverable executor. Remaining legacy contracts, role/environment/resource isolation, complete family/product acceptance, managed worktree/integration, intent/control, autonomy, semantic/provider/platform acceptance and M5 calibration retain their original scope.

Restoration/commit validation checkpoints:

- The reconciliation write-lock regression failed before the lease-lifetime change and passed unchanged afterward, testing actual SQLite busy/commit and rollback with a competing workspace lease.
- Five final native restoration cases passed: missing executor checkpoint with original baseline retained once, corrupted/ambiguous/uncertain result and failed commit refusal, actual managed writer restoration with mandatory source refresh, real daemon/TUI/CLI authorization and restart, and preserving incomplete sibling/session outcomes instead of clearing global uncertainty.
- Final library/recovery/IPC/TUI/session/independent-benchmark targets passed 50 cases with zero failures. The frozen restoration-card case passed. Python discovery passed all 49 cases and all three requirements freshness audits passed with 473 parent semantic reviews still open.
- Final all-target Clippy with warnings denied, locked build, formatting and diff checks passed. No deadline, benchmark oracle, native gate, supported provider version or dependency was relaxed.
- A new locked all-target regression of this exact final implementation is running with two test-harness threads and process-local supported-Claude PATH. Record its terminal result before claiming a green full regression. The preceding stage-result full run remains 400 passed, one failed, seven ignored; a focused benchmark rerun does not erase that recorded failure.


## Observed after-state for interrupted mediated writes — 2026-10-14

The preceding completed-executor restoration full regression finished green: 408 Rust cases passed, zero failed, seven ignored across 57 outer binaries, with two test-harness threads and the retained supported Claude 2.1.296 PATH. This includes the independently rerun benchmark's actual full-regression result; the older 400/1/7 stage-result failure remains historical and is not erased. This 408-case run predates the following effect-observation changes.

The remaining write boundary can leave a native completed proposal and durable `Intent` ownership record while a file already contains the approved after-state and the edit receipt transaction failed. Ordinary generic reconciliation previously reset the team/revision when observed inputs differed from its saved snapshot. New explicit `reconcile_task_effects` acknowledges this scoped observed state without replaying a native turn, check or filesystem replacement. Eligibility requires one exact current task/attempt intent, independently recorded completed native output and original scope, current pinned configuration and dependencies, validated role/model output, exact proposed edit/intent hashes and permitted owned paths. Every proposed edit must already match its after-hash. Other changed pinned inputs require authenticated same-wave completed managed effects; arbitrary unrelated changes, missing/unapplied/conflicting files, ambiguous/native-unobserved intent, altered scope or absent ownership refuse this acknowledgement.

The durable `ObservedApplied` state is separate from normal `Applied`. Its observation receipt preserves the original pending/applied receipt states, immutable original inputs/dependencies/native output and observed snapshot. A scope-bound local-user approval and exact observation evidence authenticate this origin. Neither the record nor its UI claims which process performed the matching write. Result restoration retains the observation origin after the execution becomes `Committed`; losing its receipt, user approval or evidence refuses proof consumption. Original source hashes remain original. Current inputs adopt observed bytes and previous results become stale; only actual independently reviewed source refresh can make new evidence current. No quality pass or stage acceptance is created by observation.

The operation holds workspace ownership through its state/event/idempotency transaction, does not launch provider work, and performs no writes. Unknown active turns, remaining intent records, pending edits or unknown task checks retain the reconciliation barrier. Scheduler uncertainty is preserved for subsequent result recovery. The scoped action is available in CLI, daemon IPC and the TUI's frozen intent card. Snapshot controls contain a count; paginated `task-recovery` documents bind full scopes to the exact run document hash.

The first focused run passed the refusal/rollback, real daemon/TUI/CLI/restart and frozen-card cases, but one case incorrectly assumed every effect receipt failure also sets scheduler checkpoint uncertainty. It now asserts exact preservation of the pre-existing flag, rather than manufacturing or clearing it. The final focused rerun and broad checks will be recorded below when terminal. Tests use actual failing SQLite edit-receipt transactions, real native fixture processes and physical files; they remain structural/local protocol evidence, not authentic model judgment.

Remaining P0 work keeps the full scope: acknowledgement/recovery of unapplied or conflicting effects, simultaneous unrecorded sibling effects needing complete cohort reconciliation, unknown check outcomes, incomplete native exact-ID resume/replacement with explicit handoff, and scheduler uncertainty without a known completed outcome. The native/legacy role contracts, environment/resource isolation, complete family/product scenarios, managed worktree/integration, intent/control, autonomy, semantic/provider/platform acceptance and M5 calibration remain required. All 473 outstanding parent semantic interpretations remain open.

Observed-effect validation checkpoints:

- Final focused observed targets: five native-task cases passed (including two existing observed-check/baseline regressions and three new effect cases), and two TUI cases passed. The new cases cover a physical after-write/SQLite receipt failure, retained original intent/source hashes, observation-origin receipt/approval/evidence refusal, no repeated native/check/write effects, separate artifact restoration and stale-source handling, unapplied/conflicting/unowned/stale states, failed transaction, actual daemon/TUI/CLI authorization and restart, and pinned confirmation.
- Final library/recovery/IPC/TUI/session targets passed 49 cases with zero failures. Python discovery passed 49 cases. Source/audit/gap freshness checks passed with 473 parent semantic interpretations still open.
- All-target Clippy with warnings denied, locked build, formatting and diff checks passed. No assertion, deadline, quality gate, dependency or supported provider version was relaxed.
- A new locked all-target regression of the observed-effect implementation is running with two test-harness threads and retained supported-Claude PATH. Its exact terminal result remains to be recorded. The prior 408/0/7 full result applies to the previous restoration implementation, not to these subsequent effect changes.


## Complete selected effect cohorts — 2026-10-14

Single-task after-state acknowledgement rejects changes belonging to an unselected uncertain writer. If multiple original wave members lost receipt checkpoints, this prevents individual acknowledgements from silently adopting each other's files. New `reconcile_wave_effects` explicitly selects the whole required cohort and binds its exact native execution IDs and immutable per-member observation scopes. Task IDs must be nonempty/unique, native intents independently valid, paths disjoint, original wave identical, and every proposed edit already present as its exact after-state. Changed inputs outside the selected cohort require previously authenticated same-wave completed effects. Unapplied or conflicting members refuse the cohort without attempting their writes.

All selected observation records, original receipts, evidence and one cohort approval commit in one state/event/idempotency transaction while retaining workspace ownership. Per-member proof retains canonical membership and the common scope, plus each member's own immutable receipt scope. Proof consumption rechecks the common approval, complete member set, each native proposal and original/observed input binding, outcome, original/current receipt hashes and exact retained observation evidence. Missing/corrupted peer proof invalidates the remaining members' cohort proof. The original single-task receipt's optional absent cohort field is omitted on serialization, preserving its existing observation-evidence bytes.

The TUI offers a frozen card listing every selected task and original intent. CLI `reconcile-wave-effects STATE_DIR RUN_ID WAVE_ATTEMPT --approve-scoped` uses the current complete package; daemon IPC accepts explicit task IDs/scopes. Snapshot controls expose only a cohort count and the hash-bound paginated `task-recovery` document carries full packages. Acknowledgement does not invoke a provider, check or write, nor produce independent verification or stage acceptance. Result restoration can retain each original completed output afterward; current-source invalidation and other unresolved sibling/check outcomes remain enforced.

The new fault-model fixtures retain actual native completed outputs and physical writes from two disjoint actors, then restore the saved intent states before their receipt/outcome checkpoints. This does not claim a live daemon crash at a simultaneously failing SQLite boundary: mediated writes currently serialize store access. It exercises the otherwise unrepresentable combination of two missing durable receipts without inventing native output or physical after-state. Cases cover incomplete/duplicate membership, canonical ordering, complete acknowledgement, idempotence, exact peer-proof rejection, restoring both original results without replay, conflict and whole-transaction rollback, daemon/TUI/CLI authorization and restart, and frozen complete-cohort confirmation. Verification results will be recorded after the current full-regression job and focused cohort checks finish.

The original design remains the completion target. Remaining recovery work includes unapplied/conflicting outcomes, unknown checks and native exact-ID continuation/replacement; all legacy contracts, role/family/environment isolation, managed worktree/integration, intent/control/autonomy, 473 semantic reviews, provider/platform acceptance and representative calibration remain open. No product or whole-MVP acceptance follows from this cohort integration.


Cohort validation checkpoints:

- The previous observed-single-task implementation's final locked all-target regression completed: 412 Rust cases passed, zero failed, seven ignored across 57 outer binaries, using two test-harness threads and the supported Claude 2.1.296 PATH. This result predates cohort integration.
- Three native cohort cases passed: exact complete/canonical membership and restoring both retained real outputs without replay; refusing a conflicting member and rolling back the entire transaction; real daemon/TUI/CLI explicit authorization, complete displayed scope and restart. The remaining control/full-current checks are in progress; a filtered TUI invocation ran zero cases, so the full TUI target is required before crediting its new frozen-cohort test.
- Python discovery passed all 49 cases; source/audit/gap freshness checks passed with 473 parent semantic interpretations still open. No runtime test assertion, time bound, native quality gate, provider version range or dependency was relaxed.


## Chat-first task intake and terminal send repair

The reported `antislop ui`, `i`, prompt, `Ctrl+S` sequence previously did nothing when no run was attached: the key handler silently required a nonempty run ID. Existing message submission also only queued a message. This exposed a missing intake workflow, rather than a terminal shortcut problem.

The frontend now starts or connects to its daemon, opens a new conversation when no run ID is supplied, and creates a durable intake from the first prompt. The original prompt remains the objective and original requirement text. `Ctrl+S` authorizes a read-only orchestration turn and its separate independent review during intake; subsequent interview replies do the same. Empty sends, pending submission, failed creation and queue/start rejection have explicit notices. Drafts survive failed sends and edits made while a receipt is pending. Existing agent messages retain their next-turn queue semantics and executable-run stage approvals remain separate. Importing an existing specification is optional, not a prerequisite.

Intake has a distinct persisted flag and provisional interview contract. It cannot approve execution, install a team, run project checks, change mode or grant task writes. It uses the project’s orchestrator configuration when supplied, or the product’s read-only native Codex conversation bootstrap. No model/provider API was introduced. Task-family execution routes and managed-worktree setup are not prerequisites for this conversation-only stage; ordinary executable specs retain those checks. There is no execution-team template during the interview. Existing non-intake serialization omits the false flag to preserve prior retained JSON bindings.

The user’s clarification is recorded in design §8.2: the orchestrator interviews the user and decides agent roles/configurations, while roles and system prompts are predefined. **At that checkpoint, the complete post-interview transition was still open; the later “Interview proposals, reviewed routing and team handoff” section records its implementation.** The initial chat patch made the conversation testable; it does not implement an independently reviewed executable-spec/configuration proposal and UI acceptance, or claim that writing a manual SPEC.json is the intended product flow. That missing transition is the next priority before returning to broader recovery work.

Validation: 65 Rust cases passed across configuration, intake, IPC, orchestration, TUI and session projection. They include actual PTY input `i`, prompt and byte `0x13` to the real frontend, actual daemon IPC, completed mock-native orchestrator/reviewer processes, a second conversation reply, retained original text and refusal to execute a provisional intake. These CLI processes exercise transport and control behavior; their synthetic reports do not establish authentic model interview quality. Python discovery passed 49 cases; all-target clippy with warnings denied and source/audit/gap freshness checks passed. Two intermediate native checks caught the premature execution-template dependency and a legitimate paginated document-change race; both were corrected without increasing timeouts or weakening acceptance. Full current regression will be recorded separately after completion.


Final intake checkpoints:

- The locked all-target regression completed with 427 passed, 0 failed and 7 ignored across 61 reported targets. This includes the cohort changes and initial chat-intake implementation. It predates the final draft-receipt handling, intake presentation label and document-cursor precedence fix below; it is not relabeled as a second all-target run of those final changes.
- The latest focused configuration/intake/IPC/storage/orchestration/TUI/session targets passed 69 cases with 0 failures. This includes the actual PTY shortcut, two native interview replies and a real SQLite refusal of the first message commit. The first fault hook used the wrong event name; correcting it to `message_queued_next_turn` caused the intended rollback, retained one intake with an unaccepted draft and permitted a retry on that same ID. No assertion or timeout was relaxed.
- The two-turn reader exposed a real run-chunk race: a new document could be shorter or have a different UTF-8 boundary at the old cursor, so storage rejected the offset before IPC checked its expected hash. The bounded read now checks the expected saved digest before interpreting that cursor. A deterministic shrinking-document case refuses the old digest as changed, while the same invalid offset with the current digest remains rejected. The native polling test retries only the explicit document-change response within its unchanged deadline.
- Final all-target clippy with warnings denied, locked build, formatting and diff checks passed. Python discovery remains 49 passed; source/audit/gap checks preserve 1146 fragments, 1443 interpretation units, 513 reviewed atoms and 474 remaining parent reviews. The design remains partially implemented, including the post-interview configuration/specification transition described above.


## Interview proposals, reviewed routing and team handoff

The next UI gap is now connected: a native orchestration response may contain `intake_proposal_json` with a task definition, unpinned configuration and rationale. Runtime owns workspace/provider resolution and computes the pin; the model cannot invent hashes, provider binaries, homes, isolation capabilities or an autonomy change. The contract replaces the provisional INTAKE/INTERVIEW placeholders, retains the exact original chat prompt and declares executable criteria, plan items, input/editable files and exact check argv. It validates all required task and orchestration routes. The original interview and its messages remain in history.

Proposed input bytes are captured separately as `runtime:intake-sources`. Dedicated independent review covers contract obligations, routes/configuration, local permissions and those actual source observations. Source review must cite that receipt; intake authentication additionally binds the exact reconstructed verifier prompt/schema to the actual native turn. Changing file bytes, adding a user message, forging receipt/round data or losing the native binding refuses acceptance. This is review of a proposed task, not proof that any project work or quality gate succeeded.

`AcceptIntake` persists the accepted spec/configuration at the next revision while holding the workspace lease through commit. A failed SQLite commit leaves the original interview, configuration, approvals and observed output intact. Idempotent retry does not repeat native work. CLI `accept-intake STATE RUN ROUND --approve-scoped` accepts metadata only. TUI `a` opens a frozen scrolling card for requirements, profiles, permissions, files, exact argv, source hashes and independent review; Enter also explicitly requests the following read-only team-planning turn. The next reviewed team card shows effective agent configurations and the full DAG/gates. Installing that team keeps tasks planned and requires separate wave approval for execution. Chat-originated conversations retain `Ctrl+S` read-only reply behavior after leaving intake.

Focused actual-process fixtures cover contract/configuration approval, original prompt preservation, separate native review and source receipts; rejecting invented binary/omitted scope/rejected review; changed files/new messages/forged native source input; complete daemon/TUI backend contract-to-team installation, persisted selected routing across restart and no task/check/file effects; and real commit failure plus idempotent retry. The completed mock CLI records also prove that subsequent planning invokes the orchestrator-selected model rather than the original one. Their synthetic judgments prove control/transport behavior, not authentic model interview or planning quality. Validation totals will follow the final checks.

Remaining limits are explicit: provider choices come from the initially declared configuration; the default conversational bootstrap currently offers Codex. Automatic installed-provider discovery, full managed-worktree execution, broader control-intent translation, role/environment/resource isolation and substantive provider/platform/task-family acceptance remain unfinished. The full design goal is still active.


Interview-transition validation checkpoints:

- The initial focused control suite passed 76 cases. A separate real PTY test then exercised first prompt/Ctrl+S, contract-card approval and team-card approval without a specification form. The selected model was observed in actual mock-CLI argv, not only in stored profile metadata. The model/protocol judgments remain synthetic fixtures.
- Locked full regression reported 435 passed, 1 failed and 7 ignored across 61 targets. Its sole failure was the newly added two-card PTY harness: fixed delays between Esc and a allowed both bytes to remain queued while the UI was rendering, so the terminal parser could treat them as an Alt combination and leave the editor active. The failure and log are retained as evidence; the run is not declared green.
- The harness now waits for the actual hidden-cursor frame after leaving the editor and for the actual confirmation card before Enter. It alternates actual terminal sizes for both redraws rather than reusing an unchanged-size buffer. No timeout, acceptance assertion or provider version constraint was relaxed. The latest configuration/intake/IPC/storage/orchestration/comments/TUI/session suite passed 77 cases with zero failures, including the corrected PTY case alongside concurrent native orchestration cases. Full all-target clippy with warnings denied, locked build, formatting and diff checks passed. Python discovery passed 49 cases; source/audit/gap checks passed with 474 parent semantic reviews still open.
- Next priority: discover and expose usable installed CLI capabilities and available profile choices for orchestrator decisions, then continue supervised progression, workspace isolation and native recovery work. Read-only local version observations are Codex 0.162.1, Claude 2.1.299 and agy 1.3.4; version observation alone is not capability, authentication or task-family acceptance. Existing supported-version gates remain unchanged.


## Runtime CLI discovery for interview routing

The intake orchestration worker now enumerates the four supported CLI kinds and the explicit project configuration. It keeps explicit executable/home/isolation/effort declarations and resolves automatic candidates from absolute PATH entries to real executable paths. It never replaces a missing or rejected configured choice. Every kind receives a catalogue entry; compatibility comes from the real adapter probe, while missing executables, rejected versions/startup settings and timed-out probes remain visible. Each owned probe has a 10-second discovery budget with cancellation forwarded from the runtime token. Guardian/lease handling and admission between probes preserve pause/cancel behavior. The render/input thread and mutation response do not wait for probes.

The catalogue is persisted on its orchestration round and included in the actual immutable native request context before the input receipt and session are recorded. A proposed additional provider must match a compatible observed entry exactly; changing the catalogue alone without its captured context refuses approval. Every required predefined task/management role must resolve to an observed compatible CLI. Authentication and model availability are unverified, not implied by a version/help probe. The initial orchestrator is not silently changed. Historical catalogue observations remain visible after contract acceptance; new native invocations retain their own preflight checks.

This exposed a Codex probe defect: it previously returned capabilities for any reported version, although turns enforced the pinned version. Probe and turn now share the same existing 0.162.1 validation; the supported version range was not widened. Capability fixtures prove configured/missing choices are preserved, PATH candidates require executability, canonical paths are recorded, actual versions are retained, unsupported versions reject, probe budget stops owned work, and user cancellation stays distinct. A real daemon fixture starts with only declared Claude, discovers a mock Codex solely from its PATH, supplies that observation to the orchestrator and accepts the selected provider/model without task/file execution; forged catalogue binaries refuse. The corner overlay reports actual queued/running CLI discovery even before native sessions exist, and respects pause.

The focused suite passed 92 outer cases across 8 targets: agy, configuration, intake, orchestration, discovery, provider contracts, TUI and session projection. A raw row sum was 93 because one provider test launches a test subprocess whose result also appears in stdout; it is not counted twice. Intermediate path assertions used macOS's noncanonical /var alias and the version assertion omitted the native codex-cli prefix; expected observations now match the actual canonical paths/version string. The unsupported-version counterexample failed before the production probe gate was corrected. Clippy with warnings denied, locked build and formatting/diff checks passed. Full regression completed successfully: 439 passed, zero failed, seven ignored across 59 outer Cargo targets (`/tmp/architect-discovery-full.log`), counting nested test subprocess output only within its containing target. Python discovery passed all 49 tests; requirements, audit and gap freshness checks passed. The full suite used the previously supported Claude 2.1.296 executable without widening its version gate.

Remaining requirements include native model catalogue/authentication observations where supported, explicit bootstrap setup on machines lacking the configured orchestrator, comprehensive CLI/platform acceptance, managed worktrees, isolation and broader control/autonomy/recovery/family execution. Metadata probes and synthetic model reports do not establish substantive model quality or complete the design.


## Explicit initial CLI for a new chat

The initial orchestrator can now be selected with `antislop ui [STATE_DIR] --orchestrator codex|claude|gemini|agy` when there is no project configuration. This selects the conversational orchestrator and its separate native reviewer, not the execution team. Default Codex behavior remains available. Parsing refuses duplicate/unknown options, missing values, extra positional arguments and attempts to change an attached run. A project `architect.toml` remains authoritative; an explicit bootstrap choice conflicts rather than overriding it. An existing unreadable/non-file project configuration must not trigger an implicit bootstrap fallback.

All four bootstraps share the discovery defaults for native effort and read-only/tool-free isolation, preserve supervised mode and grant no local writes/checks/commits. The initial native model remains the CLI default. Runtime discovery/version admission still applies; authentication and model availability are not inferred. Tests cover the actual daemon/CLI interview and separate review with explicit Claude without a project configuration, and refusal to silently replace explicitly selected unsupported Codex with the compatible Claude. These model reports are synthetic protocol fixtures.

The graphical workspace proposal is separately documented in `docs/ui-redesign.md` and is awaiting the user's graphical-versus-terminal direction and design approval. Existing frontend appearance is not changed by this initial-CLI implementation. Remaining priorities keep the full design scope: bootstrap/auth/model observations, guided supervised execution, managed worktree/integration, role/environment isolation, incomplete/unknown-effect recovery and exact native continuation, full task-family execution and substantive provider/platform/calibration evidence, plus the remaining semantic requirement review. Final focused validation passed 97 cases across eight outer Cargo targets with no failures; the nested provider-test subprocess is not counted twice. All-target Clippy with warnings denied, locked build, formatting/diff checks and all 49 Python tests passed. Requirement/audit/gap and dependency-inventory freshness checks passed; 474 parent semantic reviews and three active dependencies lacking license source files remain explicit. The locked all-target bootstrap regression completed: 444 passed, zero failed, seven ignored across 59 outer Cargo targets in `/tmp/architect-bootstrap-full.log`, using the previously supported Claude 2.1.296 executable. This snapshot predates the subsequent exact native task-continuation implementation.


## Exact native task continuation and daemon restart

`ResumeTaskSession` authorizes one selected executor or verifier continuation after workspace/effect reconciliation. It binds the full current run, task, role, original managed/native session, previous turn and sources/configuration. Fresh native capability/version admission must match the original session scope. Runtime sends the exact UUID through the adapter's resume path, reserves a new turn in the same managed session and retains the old failed/stopped/delivery-unknown turn unchanged. Existing unknown inbox delivery is not automatically resent. Native IDs without an observation, completed turns, source/configuration/role drift, unknown task checks, existing executor effects/output and uncertain scheduler checkpoints refuse continuation. Verifier recovery preserves the authenticated executor artifact and all existing review/check requirements; chronology must place the verifier after that pinned executor. Executor continuation must follow any previous attempt boundary.

An explicit `Resuming` scheduler state preserves the original executor attempt without relaxing the invariant for ordinary planned attempts. Continuation history records its approval and launching run attempt. The selected wave executes alone; executor completion leaves independent verification pending. Quality or stage acceptance is not implied. CLI `resume-task-session STATE RUN TASK --approve-scoped`, hash-bound paginated recovery packages and a frozen TUI confirmation card expose the exact continuation scope. Metadata approval is committed before work; failed database commit rolls it back without native execution, and an idempotency acknowledgment does not resume twice.

A real daemon/CLI restart fixture exposed stale-socket startup: the frontend previously returned connection refused after daemon termination. It now retries startup only for a refused/missing connection to a private owned socket. It does not unlink that path; the daemon's existing exclusive lock and path checks govern removal. Startup waits tolerate the old refused socket until the new listener is ready. Unexpected files, listening-server application rejection and protocol mismatch remain refusals.

Intermediate native tests exposed the incorrect first transition through `Planned`; the separate continuation state fixed the production scheduler rather than loosening its replay guard. Stale-socket fixtures initially had non-private socket/state permissions; those fixtures now match the production contract, without weakening ownership/mode validation. Final focused checks passed 71 distinct cases: 68 configuration/intake/IPC/session/TUI cases across six targets and three native continuation cases covering executor/verifier restart, scope/source/chronology/retained-output refusal, rollback/idempotence and actual daemon/UI projection/CLI restart. The final five session cases were rerun after the last retained-output guard and are not counted twice. Filtered native invocations reported zero session cases; only the unfiltered session runs are credited. All-target Clippy with warnings denied, formatting/diff checks, 49 Python cases and requirement/audit/gap/dependency freshness checks passed. Final locked build passed. All-target regression completed: 451 passed, zero failed, seven ignored across 59 outer Cargo targets in `/tmp/architect-native-resume-full.log`, with the previously supported Claude 2.1.296 executable. This snapshot predates the subsequent replacement-handoff changes. Previous full regression (444 passed) predates this slice and is not credited as its full validation.

This establishes controlled native task continuation, not the complete restart requirement. Automatic autonomous recovery, management/orchestration sessions, explicit replacement handoffs when an ID is lost/unsupported or scope changed, unknown-effect/check/scheduler adjudication, managed worktrees and environment isolation remain open. Mock model judgments establish protocol/control behavior, not substantive task-family, model or platform acceptance. The full original design objective remains active.


## Explicit task-session replacement with versioned handoff

`ReplaceTaskSession` installs one scope-bound replacement for an interrupted executor or verifier, including a task whose native identity was never observed. It creates a new managed/native session instead of impersonating a resume. The task's current attempt stays pinned; old sessions/turns are unchanged and independent verification/stage acceptance remains required. It shares the native continuation guards for reconciliation, current sources/configuration, check/effect uncertainty, current-attempt chronology and retained executor authorship. A completed output is not replayed through replacement. Scope changes still require explicit contract revision rather than rebinding stale claims.

The `task-handoff-v1` packet captures the original executable contract and task constraints, source hashes, dependency results, prior task sessions and structured reports, applicable artifacts/check/effect receipts, provenance/quality/risk/comment state and historical message delivery states. Historical bodies must match their original content hashes. Queued message bodies stay in the normal inbox packet; handing off history does not fabricate delivery or acknowledgement. Handoff bodies are not recursively embedded in later handoffs or duplicated in the ordinary task prompt: prior handoffs retain their IDs/hashes in task context and their full bodies in durable history.

Approval binds the current run and handoff digest. Reservation authenticates the stored packet against current source/configuration versions and the separate replacement approval before native work. Failed SQLite approval rolls back the complete original state; idempotent acknowledgement does not launch another session. IPC recovery uses hash-bound pagination with counts and packet validation, the TUI freezes the complete handoff card and the CLI requires `replace-task-session STATE RUN TASK --approve-scoped`. An actual InvalidSession response does not trigger a new native session; it removes exact resume eligibility and leaves explicit replacement available.

New actual-process fixtures cover unobserved executor identity and incomplete verifier replacement, distinct native IDs/argv, exact handoff bytes received by the CLI, original attempt/history/artifact preservation, historical unknown delivery, source/scheduler drift, forged packet data, SQLite rollback/idempotence and daemon/TUI/CLI restart. These remain synthetic model judgments. Focused validation passed 76 distinct cases: 69 configuration/intake/IPC/session/TUI cases across six targets, four replacement/handoff cases and three exact-resume regression cases in the native task target. The frozen replacement-card case is included in the full TUI count and is not counted twice. Final source-bound packet checks refuse changed operation, changed handoff content with a recomputed digest, corrupted historical bodies and stale previous-session identity. Python passed all 49 cases; source/audit/gap/dependency freshness and formatting/diff checks passed. Final all-target Clippy with warnings denied and locked build passed. The locked all-target regression completed: 456 passed, zero failed, seven ignored across 59 outer Cargo targets in `/tmp/architect-replacement-full.log`, with the previously supported Claude 2.1.296 executable. This snapshot predates management-round recovery. The preceding full 451-pass regression predates this slice.

Remaining scope includes replacement across explicitly revised task/source/configuration contracts, orchestration/management sessions, automatic autonomous recovery and unknown check/effect/scheduler adjudication; managed worktrees/isolation, full family/provider/platform acceptance and semantic requirement/calibration work remain required. The graphical appearance proposal still awaits the user's direction and approval. The full original objective remains active.


## Recovery of management interviews and plan reviews

Management recovery now targets the last failed/interrupted orchestration round without requiring a new queued message. It preserves the original round/context, input/output/source observations and original dialogue authorization. Exact resume requires an observed current native ID; explicit replacement creates a distinct managed/native session with a versioned handoff; restore reattaches a durably completed native review without a provider turn. A completed authenticated executor response is retained when the verifier alone failed, including the boundary where its native output committed before the round-output checkpoint. The dedicated independent reviewer still evaluates the original response and required receipts.

Fresh native reservation authenticates the recovery approval and handoff, preserves previous incomplete turns, requires exact version/scope matching for native resume and never conflates replacement with the old session. Intake review authentication reconstructs the actual decorated recovery prompt and inbox packet, so captured source/configuration claims cannot be rebound merely by altering stored observations. Existing intake proposed source bytes must still match before recovery. Comment reactions are applied idempotently for the original round. New messages/obligations, changed comments/sources/spec/configuration or superseded/substantively rejected rounds refuse old-scope recovery.

Hash-bound paginated `orchestration-recovery` packages and frozen TUI cards expose resume, replacement and retained-result restore separately. CLI verbs require `STATE RUN ROUND --approve-scoped`. Recovered dialogue does not accept a task contract, install a plan, approve stage gates or execute project effects. Database approval failure rolls back the prior state; idempotent acknowledgement does not invoke native work again.

Six focused native/UI cases passed: executor and verifier exact/replacement recovery across runtime reopen; completed reviewer checkpoint restoration without native replay; source/new-obligation/approval rollback refusal; intake source retention and actual native-input review binding; real daemon/TUI/CLI recovery and restart; frozen management confirmation. Model judgments are synthetic protocol fixtures, not model/platform acceptance. Broader focused validation passed 114 distinct cases across nine outer targets: configuration, intake, orchestration, native provider/agy contracts, IPC, sessions and TUI/session projection. All-target Clippy with warnings denied, formatting/diff checks, all 49 Python cases and source/audit/gap/dependency freshness checks passed. Final locked build passed. The all-target regression terminated with 457 passed, 5 failed and 7 ignored across 59 outer Cargo targets in `/tmp/architect-management-recovery-full.log`, using the previously supported Claude 2.1.296 executable. The five failures are in selfhost benchmark/probe fixtures; the probe report records a child startup timeout. This is a failed full regression, not complete validation. Targeted diagnosis is pending and no timeout or assertion is relaxed. The preceding 456-pass full regression predates this slice.

Automatic autonomous recovery, management recovery after explicitly revised scopes, unknown task check/effect/scheduler adjudication, managed worktrees/integration and stronger environment isolation remain open, alongside complete role/family/provider/platform acceptance, semantic requirement review and representative calibration. The graphical proposal awaits user direction/approval. The full original objective stays active.


## Captured task-check outcome recovery

Runner output now has a separate durable checkpoint before the final task-check result write. `RestoreTaskCheckResult` reuses the exact captured output/hash, argv, original intent/authorization and observed after-inputs. It retains success or failure without replay or user-declared acceptance; missing captures, source/contract drift and unauthenticated authorship refuse recovery. A restored current check leaves independent verification pending only when no previous verifier exists and all current checks are known. Earlier verifier history still requires native recovery.

Baseline capture also retains the scheduler's pre-admission team and native/execution ledger hashes. It can settle that exact scheduler barrier after reconciliation and restoration of every captured check, with no native workers started. Unrelated uncertainty, missing barrier data, changed ledger/team and actual source mutation remain blocked. Expected failing baselines remain historical original-state observations, retain original inputs and cannot establish a current positive quality gate. Tasks stay blocked until explicit continuation.

CLI `restore-task-check-result STATE RUN INVOCATION --approve-scoped`, paginated hash-bound IPC and a frozen TUI card expose the original result; backend validates the full package against its captured Run. Initial focused validation passed four native recovery cases and one TUI card case, including actual success/failure, baseline retention, rollback/idempotence, missing/corrupt/source-drift refusal and daemon/UI/CLI restart. The expanded baseline-barrier/capture/checklist counterexamples and final projection are verified in the final targeted suite. A restored writer baseline also remains authenticated across actual source changes and independent read-only refresh/review. All 49 Python tests and requirement/audit/gap/dependency freshness checks passed. Earlier management full regression failed five selfhost fixtures. A targeted serial rerun passed all six selfhost benchmark/probe cases without changing production code, deadlines or assertions (`/tmp/architect-check-recovery-selfhost.log`). This supports a transient startup/load cause but does not erase the failed full run; a new complete regression remains required. The full objective remains active.


The expanded baseline checkpoint counterexample found that, after explicit workspace reconciliation, `StepTeam` could acknowledge a wave whose scheduler checkpoint was still uncertain; its asynchronous worker rejected it later. Admission now refuses that uncertainty before recording approval or starting work, and the wave scope includes the uncertainty flag. The actual negative checkpoint tests passed after this production fix.

Broader control validation terminated with 169 passed and two failures across eight targets. One PTY harness matched raw terminal bytes rather than rendered cells and missed a word split by cursor/style updates. The harness now reconstructs visible ASCII cells while retaining the original deadlines, fresh-frame waits and actual contract/team assertions. The other failure recorded `unsafe IPC socket` during daemon startup: bind briefly published the final path before chmod. The daemon now prepares the listener under a same-length temporary name inside the private state directory, sets mode 0600 and atomically publishes it under the held state lock. Ownership/mode validation remains strict. Deterministic socket tests cover private-before-publication visibility, actual post-rename connection and refusal to replace an unexpected file. Final expanded validation passed 84 distinct cases across socket publication, captured check/baseline recovery, checkpoint failures, daemon/IPC, management dialogue, TUI and selfhost benchmark/probe targets. The 25 management cases and corrected PTY case are counted once each; the earlier failed control run is retained as historical failure. The PTY fresh-frame assertion permits the visible cursor while the composer is focused and still requires the hidden cursor after Esc. No deadline or product acceptance assertion was weakened. All-target Clippy with warnings denied, locked build, formatting/diff checks and requirement/audit/gap/dependency freshness checks passed. Python discovery passed 49 cases. The subsequent locked all-target regression used `/tmp/architect-check-recovery-full.log`, an isolated build directory to avoid another development session rewriting test binaries, and the previously supported Claude 2.1.296. That complete regression terminated with 467 passed, 5 failed and 7 ignored across 59 outer targets. The five failures are only selfhost benchmark/probe fixtures whose child observations record timeout without stdout or a child-start marker. The result is failed full validation; it is retained in the log and is not replaced by targeted pass counts. A native fixture correction is in progress without changes to deadlines, assertions or production runner behavior.


## Source observations for managed workspaces

`workspace::snapshot` and CLI `workspace-snapshot DIRECTORY [ADDITIONAL_PATH ...]` now observe the source boundary required by design §11.1. The snapshot retains repository/common directory, HEAD/branch (including unborn/detached), project prefix, all applicable index stages, current binary-compatible file hashes/modes, missing tracked files and nonignored untracked paths. Explicit additional paths can include ignored files. NUL-separated Git paths preserve tabs/newlines; unsupported path encoding fails explicitly, including an index imported from another platform. Source file bodies and symlink target bodies are omitted from the report; file permissions and ownership are retained. Symlink target bytes are hashed without following them; ancestor symlinks and special files refuse observation. Gitlinks and unenumerated directories such as untracked nested repositories remain explicit unresolved content and cannot receive an inclusion scope.

The reader uses supervised Git processes, clean environment/configuration overrides and optional-lock/lazy-fetch suppression. Raw blob hashing disables filters; regular input streams with SHA-256 and Git hashing share the same bytes, keeping file-body memory bounded. Two equal observations and later exact revalidation detect source drift; external writer isolation still requires the managed environment. The scoped working-copy/deletion inclusion digest binds the complete observed state and explicit selected paths. A persistent workspace ID registry, durable preparation intent/result, actual candidate/task worktrees, environment/resource allocation, integration verification and separate delivery operations remain required; configured execution in worktree mode remains gated pending that lifecycle. See `workspace-lifecycle.md`.

Eight actual Git/filesystem/CLI cases passed, covering staged/unstaged/deleted/untracked/ignored content, index/ref preservation, literal names/binary data, unborn/detached/linked/nested workspaces, callback refusal, SHA-256 Git objects, submodule/conflict/hash/encoding refusal and source drift. macOS rejects creating the invalid byte filename directly; the counterexample inserts an actual non-UTF-8 Git index entry, then requires observation refusal. No production encoding rule was weakened.

The repeated full-regression startup failures in selfhost fixtures are addressed by compiled native Rust dummy CLIs. Their supplied argv behavior, mismatch/mutation counterexamples, individual one/two-second deadlines, observed child-start requirement, supervised descendant cleanup and raw measurement assertions remain intact. The production benchmark/probe scripts and runner are unchanged. All six original selfhost benchmark/probe cases passed with the native fixtures. They validate runner/protocol behavior, not representative product performance or task-family acceptance. The broader focused suite passed 16 distinct cases: eight source snapshot cases, six selfhost benchmark/probe cases and two independent benchmark/guardian cases. All 49 Python tests, all-target Clippy with warnings denied, locked build and requirement/audit/gap/dependency freshness passed. Final permission/ownership fields, all-target Clippy with warnings denied, locked build and formatting/diff checks passed. The reader was also exercised against the actual current repository: 215 files, 192 local changes, no unresolved gitlinks/directories, and unchanged source index bytes/mtime. The observation is retained as historical local evidence in `/tmp/architect-source-workspace-snapshot.json`; subsequent source edits require revalidation. The new locked all-target regression is running in `/tmp/architect-workspace-snapshot-full.log` with the previously supported Claude 2.1.296 and isolated build directory. No terminal result is claimed. The previous 467-pass/five-failure regression remains historical failed evidence.


## Interview waiting state and chat guidance (2026-10-14)

The actual local chat records exposed a premature V08 blocker: the independent reviewer required an observation of its own native identity before its response existed. Its blocker then entered later interview context. Provisional intake replies now finish as `awaiting_user` after authenticating the actual completed orchestrator turn. They start no verifier, install no contract/team and approve no execution. A completed intake proposal still needs its independent native reviewer, all required checks and explicit user acceptance. Native identity separation remains enforced by runtime after response delivery; the V08 prompt asks for independent inspection/reasoning rather than an impossible future receipt.

The chat and graph progress overlay show the waiting state and answer instructions. A newly received question opens the answer editor when the user is viewing chat, without taking over an agent-detail view, a search or a comment draft. Enter or `i` reopens the answer editor; Ctrl+S submits a free-form answer. A successful send clears waiting locally; failure retains the draft and waiting state. Persisted running state or a subsequently queued user response also removes the waiting indicator. Chat-origin conversations continue to invoke orchestration after contract acceptance.

Focused validation: 76 cases passed across intake, managed workspace, orchestration runtime, TUI and session projection. The guided regression exercises question → user answer → proposal → independent review → contract acceptance, asserts no reviewer while awaiting the answer, and checks that repeated snapshots produce no native turns. Existing forged/rejected review and source/authorship refusal cases pass. The final TUI-only run passed 34 cases after adding automatic editor focus; all-target Clippy with warnings denied passed before that final UI focus change. A real PTY `i`/prompt/Ctrl+S smoke with Codex CLI 0.162.1 produced Polish clarifying questions, `awaiting_user`, no verifier session and no review. Evidence: `/tmp/architect-live-interview.log`, `/tmp/architect-interview-guided.log`. This proves the interview entry path, not end-to-end completion of a real project task.


## User-selected Codex/Antigravity routing (2026-10-14)

Local `architect.toml` now declares Codex only for orchestration and Antigravity (`agy`, `gemini-3.8-flash-high`, deep/high) for every executor and verifier. It uses supervised in-place execution with local writes/checks enabled and local commits disabled. These settings define available local actions; task/wave acceptance remains explicit. Optional `role_providers` constraints are preserved exactly across intake proposals and validated against every profile. Empty/default constraints are omitted during serialization, preserving existing pins. Compatibility discovery inspects CLIs independently of synthetic probe-role restrictions while leaving the actual run's routing and pin unchanged.

A real local probe first caught the default blank native status-line shape; permitting only that non-executable shape fixed startup auditing without editing global CLI settings. The installed binary then changed from 1.3.4 to 1.3.5 during validation. The adapter now admits both exact releases, retains its existing finish-only agent and protocol restrictions, and rejects unknown releases. Real 1.3.5 inference through the configured review profile completed with the requested structured output. A separate real PTY/UI conversation loaded the project's Codex/agy profiles and role limits, observed a compatible agy catalogue entry, and stopped at Codex's Polish clarifying questions without starting a verifier.

Focused routing validation passed 96 cases (config 18, intake 9, orchestration 28, agy 7, TUI 34). A subsequent concurrent orchestration run hit two unchanged 15-second fixture deadlines; the final serial repeat passed all 28 cases, with 8 agy and 5 discovery cases also passing. After exact 1.3.5 support was added, the agy/discovery repeat passed 9 and 5 cases. Native smoke output: `REAL_AGY_CONFIGURED_REVIEW_PROFILE_OK`; PTY record: `/tmp/architect-live-routing-135.log`. These transport and interview results do not claim full real-task completion or general model/platform acceptance. Existing conversations retain their previous configuration pins; new conversations load the local project configuration.


## Directory scope and guided approvals (2026-10-14)

The user's actual test `1f857f6f…` failed because the orchestrator supplied the whole repository's absolute directory as an input and the old reader accepted only relative files. Directory scopes now remain single inputs: an absolute path within the workspace normalizes to a relative scope, and `.` denotes the complete repository. The internal observation records text bodies, binary byte counts/hashes, literal symlink targets and missing tracked paths; it never follows a link outside the scope. Git-backed membership covers tracked and nonignored untracked source; native Git administration and recognized runtime state remain excluded explicitly. Two matching captures bind contents and membership. The directory reader uses repository membership queries and local hashing rather than per-file Git object subprocesses. Outside-workspace selections and parent traversal refuse admission.

Invalid source selections are rejected before independent review rather than being classified as uncertain project effects. Startup can repair the specific legacy directory-path failure only when completed orchestrator authorship authenticates, the proposed directory is valid, and no reviewer, task artifacts, edits, provider failure were admitted. The response remains historical and no native process starts; an explicit missing-review approval is offered when no newer user input supersedes it. A newer queued user update instead offers a fresh scoped conversation turn, without duplicate submission. Tampering with completed output refuses that repair.

Chat `a` now guides reviewed contract/team acceptance, current independently verified stage-result acceptance and the exact ready scheduler wave. Wave cards freeze selected tasks, actual profiles, workspace/policy, dependency bindings, write scopes and the operation's declared checks; existing authenticated check receipts remain reused. Running/paused/uncertain states do not offer a new wave. A missing initial review with a retained completed proposal is offered separately, preserving the executor output.

The actual Codex/agy disposable task reached a complete contract but its first independent review failed on an external-tool/subagent protocol event. A separate fresh diagnostic turn with the same retained proposal completed and its captured event shapes were ordinary user_input/agent_response/finish with no tool/subagent fields; the first failure is not fully explained and no policy guard was loosened. These results do not prove full real-task completion. The source changes and guided approval behavior continue the full original objective.

Validation: directory contract acceptance plus legacy checkpoint recovery passed two native fixture cases, including retention without replay and forged-output refusal. Guided execution/review/stage acceptance passed one actual scheduler fixture. Directory and TUI focused tests passed, with a clean run retained in `/tmp/architect-directory-clean.log`. A broader orchestration run failed six unchanged observation deadlines under concurrent shared-workspace testing (`/tmp/architect-directory-input.log`); it is retained as failed evidence, not replaced by narrow pass counts. Final source/scheduler/UI validation and Clippy are tracked in `/tmp/architect-directory-validation.log` and `/tmp/architect-directory-final-clippy.log`. Managed-worktree directory admission/delivery and provider context scaling still require follow-up; no completion claim is made for the full design.

Final focused evidence: two directory contract/recovery native cases passed, including forged-output refusal; the latest TUI run passed 37 cases including queued-message guidance, directory input tests passed two cases, snapshot tests passed eight and managed-workspace tests passed three. The guided-wave scheduler fixture passed one case. All-target Clippy with warnings denied and the locked companion build passed. An audit against a private copy of the actual user state proved that the legacy false uncertainty clears while a newer queued user message prevents replay/reuse of the old proposal; the next action is a fresh scoped conversation turn. The real repository remains one directory input: 222 source entries and approximately 11.8 MB of encoded source content were captured; provider context scaling remains a material follow-up. No native task was launched by the state repair.

Live reattachment also exposed a separate intake UI timeout: snapshot preparation and the TUI eagerly calculated a whole-repository diff against the provisional empty baseline. Both paths now skip diff during intake, where no executable task inputs or diff-review action have been accepted. The regression checks the absent diff-review scope and explanatory UI text. The final intake/TUI repeat passed 10 and 37 cases. The previous two live snapshot attempts timed out while list requests succeeded; no daemon restart was used to label those attempts successful. The repaired existing user task is awaiting_start with reconciliation_required=false and its newest message retained in the next-turn queue. No native conversation or task was replayed by the repair.

The final no-diff intake fix passed all-target Clippy and locked build. The local binary and idle daemon were updated; an intervening newly started user conversation was allowed to finish before restart. Real backend projection then confirmed `USER_UI_QUEUED_CONVERSATION_READY` for the repaired existing task, including its `a` action for the already queued message. Both old and newly started conversations were retained.
