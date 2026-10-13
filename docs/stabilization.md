# Runtime stabilization and quality provenance

The original design remains authoritative. The following changes strengthen the existing vertical slice; they do not claim completion of the entire design.

## IPC protocol 2

Protocol 1 is explicitly refused rather than silently changing response shapes. JSON object keys must be unique at every nesting level, including escaped spellings of the same key. The CLI and daemon use the same decoder.

Mutation responses and snapshots contain summaries, counts, hashes and document references. They do not copy source, artifact, evidence or report bodies. `list` returns up to 100 summaries and a `next_cursor`; use `list_page` to continue. Event polling retains its durable sequence cursor.

A `read` operation fetches a bounded UTF-8 chunk of `run`, `spec`, `artifact:ID` or `evidence:ID`. The first chunk returns the document SHA-256; every subsequent request must supply it as `expected_hash`. An intervening mutation fails the read explicitly. Readers restart the read-only download instead of combining revisions or replaying a mutation. Run chunks are read directly from SQLite's stored JSON bytes with a transactionally maintained hash; pagination does not deserialize the whole run for every chunk.

```
architect snapshot STATE RUN
architect export STATE RUN
architect export STATE RUN artifact:ARTIFACT_ID
architect list STATE [CURSOR]
```

SQLite schema 2 adds the document hash and migrates schema 1 transactionally without discarding run, event or deduplication history. Frame limits are transport protection, not a budget or agent limit.

## External processes and workspace leases

A separate internal guardian owns each check/provider process group. A private lifeline closes even after daemon SIGKILL. The guardian kills descendants before reaping the group leader, avoiding reuse of a stored PID. It also cleans remaining descendants after the leader exits, so inherited output pipes cannot keep a completed command blocked indefinitely. Cancellation applies to provider version probes as well as model turns.

Workspace leases are OS locks on canonical path ancestors: parents conflict with descendants, while independent sibling workspaces may proceed. A guardian inherits the complete lease set, retaining it until its process cleanup finishes after daemon death. Tool processes do not inherit the lease descriptors. Missing supervisor binaries fail closed.

Process groups do not contain deliberately escaping programs that use `setsid` or change groups; these require an OS isolation boundary. No arbitrary-project sandbox guarantee is claimed.

## Partial edits and explicit reconciliation

Before writing a batch the runtime validates the entire edit scope and persists receipts containing every path and before/after hash. Each successful rename updates the source snapshot and persists its checkpoint. A crash in the gap is classified from actual bytes on restart as applied, unapplied or conflicting; it never causes automatic write replay.

An interrupted attempt retains `reconciliation_required`. Repair/revision and overlapping managed runs cannot proceed until effects are reconciled and process leases are released. The snapshot exposes a fresh observed-input hash and a reconciliation scope hash. The explicit command binds both:

```json
{"version":2,"request_id":"reconcile-1","idempotency_key":"reconcile-1","operation":{"type":"mutate","command":{"type":"reconcile","run_id":"RUN","scope_hash":"RECONCILE_SCOPE","input_hash":"OBSERVED_INPUT_HASH"}}}
```

Reconciliation records observed receipt states, preserves files and history, and starts fresh Discovery when inputs changed or the batch was incomplete. A stale observed hash is rejected. Cancellation remains terminal; reconciliation may clear its outstanding effects without reviving the run. Missing/unreadable pinned input files must be restored or handled by a future explicit workspace lifecycle operation; reconciliation does not silently invent their content.

## Provenance and source review

Source hashes feed fact, context, solution, simulation, artifact, review and risk nodes. Source changes invalidate dependent nodes transitively; cached presentation flags cannot authorize acceptance. Independent revalidation must enumerate every required stale node and cite actual current source-read receipts. Risk coverage binds exact risk/check IDs, reports, artifacts and evidence; a flat `pass` cannot satisfy Delivery.

The requirements source ledger preserves all 1123 nonempty source records. The atomic interpretation ledger and separately implemented source-first audit retain original clauses and qualifiers. Their reports distinguish established user decisions, proposals, code, research and open decisions. Source coverage and regression anchors do not establish full semantic completeness: remaining parent interpretations are explicitly listed in `requirements-audit.json` and linked to implementation packages in `requirements-gaps.json`.

## Verification

Regression coverage includes raw-socket duplicate keys, large Unicode snapshots and consistent document downloads, database migration, interrupted multi-file writes, external-edit conflicts, nested workspace collisions, daemon SIGKILL, descendants/cancellation and stale or forged provenance/assessment references. Fake model reports remain explicitly synthetic. Historical live exports under `m0/` are evidence of the older observed run, not a claim that every new contract has passed a live model test.
