# Pinned context pages

Native model-format rejections are separately persisted as typed step/reason
observations in a turn's optional `native_rejections`. They contain no failed
arguments or reasoning and are not rejected task artifacts or delivery receipts.
Flat historical turn archives preserve them exactly. Empty observations omit the
field, retaining original legacy serialized bytes and hashes. Completion still
requires all original context/finish/schema gates after any rejected call.

The context reader provides exact, immutable UTF-8 pages from a turn-bound bundle. Explicit `pinned_context` profiles for Antigravity 1.3.5/1.4.0 and Codex 0.162.1 connect it to ordinary runtime turns, durable SQLite checkpoints and native response validation. Existing Antigravity `tool_free` configurations retain their finish-only profile and 192,000-byte prompt preflight; existing Codex `read_only` configurations retain inline input and its scalar-count preflight. No provider silently changes its isolation profile. Complete large-repository retrieval/compaction and end-to-end task acceptance remain separate work.

## Codex and interview source access

An explicitly reviewed `[providers.codex]` configuration may select `isolation = "pinned_context"`. The adapter still ignores user configuration/rules and disables project documents, native shell, apps, hooks, plugins and subagents. It adds only the owned stdio reader through per-invocation `-c mcp_servers=...`, with `enabled_tools=["read_context"]`, required startup and an explicit response budget. The official [MCP configuration](https://learn.chatgpt.com/docs/extend/mcp?surface=cli) and [configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference) document these settings. Authentication/home/model/effort remain unchanged. This profile is not a proven general tool-free allowlist or an OS sandbox. Exact-ID native resume for this profile is explicitly unsupported until validated; the normal separately authorized replacement path preserves prior turns.

Codex context transport has its own durable `codex-pinned-context-v1` marker. Fresh Antigravity turns use `agy-pinned-context-v2`; retained `agy-pinned-context-v1` inputs keep their exact original rendering and hashes. Session validation rejects a transport/provider mismatch. Codex tool events must carry matching started/completed item identities, the exact server/tool and unchanged arguments. The full echoed MCP page must match the immutable reader and an actual committed backend receipt. Missing bodies, foreign tools, fabricated bytes, duplicate completions and uncommitted pages cannot establish an observation. The adapter retains a final JSON response as rejected if required page coverage is incomplete.

New interview rounds retain the exact pre-contract observation in `observed_interview_sources`, with its hash in the immutable request context. Reader bundles expose these as `interview-file:<selector>` before a task's input selectors exist. The bounded interview prompt advertises that access when the orchestrator selects `pinned_context`. Old rounds with an already frozen metadata-only packet are not silently backfilled/replayed. Source drift still invalidates an answer before contract review.

New captures retain an `input-observation-v1` record binding each selected root's observed `file` or `directory` kind to its exact body hash, UTF-8 byte length and complete snapshot hash. The record survives SQLite reopening, interview/proposal review, activation, mediated effects and reconciliation. Every captured directory scope, including a selected subdirectory, exposes `directory-index` and `directory-member` documents with original parent reference/hash, member path, type metadata and exact text hash. The entire original directory envelope remains available. No live body is reread and no input selector or edit scope is expanded. Links are never followed; binary, missing and link entries have metadata only. An ordinary file containing identical directory-envelope JSON remains one opaque file and cannot create descendant instructions or edit scope.

New project-instruction indexes use `project-instructions-v2` and bind the type observation as well as original body hashes. Discovery uses captured kinds, so replacing a live path after capture cannot change the historical index. A file/directory replacement invalidates currentness even when the captured bytes are identical. Reconciliation retains the original pins and requires explicit input revision for that change. Mediated text effects advance snapshot and observation atomically; separately pinned diff-baseline kinds continue to describe the original baseline. Diff reads normalize absolute selectors, represent links without following them and exclude private runtime directories.

Older records without type metadata retain their original encodings and prepared native inputs. Only known workspace-root directory captures expose legacy member views; ambiguous non-root directory-shaped bodies cannot authorize new work without explicit revision. Historical records are not silently backfilled. An unambiguous legacy source can acquire a type binding for a new mediated version while its original execution record remains unchanged. Direct non-UTF-8 file selectors still require selecting the containing directory for binary metadata. These records do not establish binary-content inspection, an OS sandbox, complete contextual-source storage or compaction.

A genuine configured Codex orchestrator passed the actual runtime/SQLite/reattachment probe in 28.50 seconds: native session `01a13dcb-25da-7f43-9600-2493fb34950c`, six committed/native-observed responses and 72,113 complete source/index bytes, including both unpredictable sentinels. Fixture: `/var/folders/d2/vtn09lpd2717th937yj7smhw0000gn/T/antislop-native-context-runtime-o6IagF`; log: `/tmp/architect-codex-context-native.log`. Its disposable diagnostic run was explicitly cancelled after the completed native turn. This proves transport and persistence, not an independently accepted project task, semantic repository inspection or native resume. Reproduce the ignored runtime probe with `ANTISLOP_CONTEXT_PROBE_PROVIDER=codex` and an explicit `ANTISLOP_CONTEXT_PROBE_CONFIG`.

## Runtime integration

Select the profile explicitly for Antigravity CLI 1.3.5 or 1.4.0 in the task's reviewed configuration:

```toml
[providers.agy]
binary = "/absolute/path/to/agy"
isolation = "pinned_context"
native_effort = true
```

Keep the existing effort mappings, model and role routes. The profile does not work on the 1.3.4 adapter contract and never downgrades itself or changes model/effort. Configuration changes require the ordinary revision/approval process for an existing run; the implementation does not rewrite a running task or user configuration. The probe reports that this profile is not tool-free, and makes no OS-containment claim.

Antigravity v2 supplies an executable first index call through `call_mcp_tool`, with the exact `ServerName`, `ToolName`, bundle/document hashes, offset and page size. The owned agent instructions use the same call guidance. The immutable index is the reader catalog: source labels and inline-registry aliases are not MCP server names or resource URIs. Agents resolve inline aliases from the full original prepared context and request pages using listed document hashes. `list_resources`, `read_resource`, native tool discovery and other tools remain outside the reader contract. Missing material means missing proof. This instruction change does not add tools or waive native event/receipt checks. Native v1 and Codex v1 rendering remain unchanged for existing records.

Before native delivery, the provider commits the complete logical request and its scope as `turn.context`, including the exact hash of its deterministic short native prompt. The original prompt/schema hashes remain unchanged. The short message points to the complete source/index and requires every byte before answering. A fresh controlled profile/cwd registers only the owned reader through audited local MCP inheritance. No project/global MCP server is added. Its private Unix socket belongs to one invocation; the `__context-connect` subprocess only forwards MCP frames to that socket.

Each page waits for the main daemon's `ContextRead` callback to commit its receipt in the canonical run/turn SQLite transaction. Only then is the response released. A failed callback aborts that response. The adapter separately matches native MCP start/completion, conversation ID, server/tool name, exact arguments and actual committed response before committing `ContextObserved`. Echoed response bodies must match exactly. CLI 1.3.5 omits large bodies from `step_update`; in that observed variant, a completed call must still match an actual unobserved backend receipt for exactly those arguments and bytes. A completion without a backend read cannot create delivery evidence. Unknown tools, changed arguments, forged bodies and repeated completed step IDs are refused.

Acceptance requires complete coverage of the index and full prepared prompt by native-observed responses. Overlapping reads count once. This is transport coverage, not semantic inspection or task correctness. A genuine schema-bound final response with missing pages is retained as `turn.rejected_output` and cannot complete the turn. Normal session/turn scope, sources, configuration, approval, independence and quality checks still apply. Replacement handoffs retain prior turns and their context/rejected outputs; unsupported exact-ID Antigravity resume remains unsupported, with no automatic replay.

New pinned-reader management recoveries use `orchestration-handoff-v2`. The mandatory prompt includes an exact `orchestration-turn:<id>` reference, SHA-256, byte length and runtime-authored historical status instead of embedding the previous turn's entire context again. The active recovery bundle exposes the full original serialized turn under that hash, including its original context, receipts and rejected output. Original content must match on preparation, retrieval and later input authentication; missing or changed records fail closed. Historical delivery/completion flags do not grant current proof or permissions. Inline profiles and retained v1 handoffs keep their original bytes. This is reference-based handoff, not native compaction or a persistent deduplicated blob store.

New orchestration review inputs use format `inline-exact-utf8-v9`, including accepted/proposed type observations. Explicit pinned-reader profiles can receive proposed-source manifests with exact source/document references; complete captured originals and their observations remain separate reader documents. A manifest proves membership/metadata, not semantic inspection. Inline profiles receive complete bodies, and retained v7/v8/unmarked inputs preserve their earlier packet bytes for native input authentication. Current sources and independent review obligations still apply.

Fresh orchestration contexts and the same task's executor/reviewer contexts receive a qualified projection of prior rejected reports, including their original hash, author/scope and findings. The private `agent_messages` field is excluded; its proposed outbox was never delivered by a rejected turn. The original raw report remains in the turn and authorized handoffs. These projections are historical, unaccepted task data; each substantive concern must be reassessed against current sources, and cannot supply a missing proof or permission.

The graph overlay projects unique observed/total bytes for concurrent readers. It hides active delivery when paused and returns to the actual review/task phase after delivery completes. Contexts and receipts remain available after reconnection through the normal run document. Ephemeral sockets and known profile files are closed/removed; unexpected files are left untouched.

## Lossless task-history archives

Model citations additionally resolve `directory-member` aliases only from known
captured directory types (or the existing unambiguous legacy root capture).
The body/hash must match that original member exactly; ordinary JSON files do
not create members. `runner:` citations must match actual completed task-check
output with its current input/configuration/approval bindings. Expected failing
baselines remain legitimate historical observations, not task-success evidence.
These aliases do not expand canonical input selectors or change original scope
hashes. Provenance rechecks those bindings and expires when captures or receipts
change. Historical handoff/archive documents are not implicitly promoted into
current model sources.

Each model/provenance validation borrows one immutable run and builds canonical
source hashes once. Member hashes are decoded lazily once; check observations are
memoized by exact `(source_ref, output_hash)`, so one valid result cannot bless a
different result under the same label. Directory envelopes are borrowed rather
than copied for each lookup. This resolver is process-local derived data without
serialization or reuse across mutable run versions. Original native bundles,
source bytes, approval hashes and fresh admission/source checks remain unchanged.
Canonical runs still store full inline bodies; persistent CAS/compaction and
complete source-kind storage remain open.

New pinned-reader replacements use `task-handoff-v2`. Each `prior_turn_refs`
entry binds a `task-history-turn:<id>` archive hash/length, original serialized
turn hash/length, session identity and runtime-authored historical state/coverage.
The archive retains original turn metadata and a context manifest with the
original source references, transport scope, receipts and native-observed reads.
Source text is exposed once per content hash through flat
`task-history-content:<sha256>` aliases. Identical bodies share the reader's
document entry; repeated replacements do not recursively wrap old bundles into
new archive bodies. Legacy prompts already containing nested history retain
their original bytes as addressed documents.

Preparation compares each reference with the exact original canonical turn and
owning task/session. Reconstruction verifies original hashes and reproduces the
entire serialized turn byte-for-byte. Missing/corrupt documents, mixed versions,
foreign turns and conflicting aliases are refused. Historical completion,
partial delivery, rejection and failure remain historical; they establish no
current proof, acknowledgement, tool access or permission. Reader retrieval
does not read live paths or expand input selectors.

Tool-free replacements and retained v1 handoffs keep their complete inline
history and exact earlier hashes. Newly accepted teams and approved v2 task
replacements select `task-prompt-v2`, which states the exact response schema and
nested artifact wire shape without changing typed output validation. Existing
turn inputs are never rewritten. Canonical runs still store inline bundles;
persistent deduplicated storage, management-history flattening, retention and
compaction remain separate work.

## Recovery projection delivery

`RestoreTaskResult` also supports a completed read-only native answer which has
no execution/effect checkpoint yet. It requires the latest exact completed turn,
zero edits/write scopes, current sources and task/agent/specification/dependency
bindings, the original task view and settled checks. Fresh executor turns persist
`task-turn-input-v1` before native delivery, independently of whether the profile
uses the reader. Earlier reader turns can authenticate the complete original
runtime prompt; old inline turns with no task input record remain unavailable.
Original legacy bytes are not backfilled. Explicit admission creates a distinct
read-only receipt with the exact turn and scoped local approval, leaving the
artifact awaiting independent verification. The model, checks, files and Git
commits are not replayed, and no quality/stage/delivery approval is implied.

The daemon prepares management recovery documents in the background against the exact captured canonical run. Snapshot control reports `orchestration_recovery_state` (`preparing`, `ready`, `unavailable`), a package count and `orchestration_recovery_document_hash`. While preparing, the chat displays the validation phase and offers no inferred recovery approval. Direct cold document reads return a preparation error; poll snapshots before reading the document.

Each prepared document retains its original `run_document_hash` and complete packages. Initial publication rechecks live source captures. Continuation reads reuse immutable bytes and verify the current canonical run binding; changed canonical state cannot reuse the old document even if database hash metadata was not updated. The frontend verifies the complete document hash and authenticates the packages together. Actual admission independently rebuilds the scoped package and checks current sources; no cache grants permission or converts historical reports into current proof. Derived documents are process-local and rebuilt after restart. Slow source capture, synchronous action admission, local frontend authentication and durable storage/retention remain separate performance work.

Prepared recovery documents additionally hold an internal authenticated set bound to the complete canonical run and exact package bytes. It has no deserializer and never crosses IPC as authority. An explicit approval can reuse it only after runtime compares the current complete run, rechecks live inputs/proposed sources and verifies the requested round, kind and scope. Worker/overlap/lease checks and atomic approval persistence remain mandatory. Cold IPC mutations return `no mutation admitted` while read-only preparation runs; the CLI waits for its observed document's preparation before sending a scoped command. Already committed idempotency keys still return their recorded response without replay.

Frontend refreshes also reuse a locally authenticated set only for the exact captured run and package bytes, with fresh live-source validation. The proof belongs to one immutable decoded document; changing original document bytes creates a new owner and drops that proof. New daemon snapshots verify the hash of the complete actual SQLite bytes before setting `document_hash_verified`; the client may then reuse its already downloaded version under that exact hash. Older daemons still require a full hash-pinned download each refresh. Changed or foreign packages must pass complete native-prefix authentication again. These internal proofs are process-local derived data and are rebuilt after restart. They cannot convert historical state or untrusted package JSON into permission. Canonical serialization in the daemon, filesystem capture, initial authentication, full UI projection and durable storage remain material costs.

## Contract

Reader construction is cached in memory for a transport. The reader and transport now own the same immutable `Arc<Bundle>` allocation. Reader/transport clones share source bodies. A cached reader can be reused only while it owns that exact allocation. While attested, the bundle cannot be uniquely mutated; copy-on-write creates a distinct owner which must rebuild and validate its reader. After eviction a uniquely mutable old owner no longer has a matching cache attestation. Even a same-length change with the old document key is rejected. No public hash or serialized flag authorizes reuse. The cache is omitted from serialization/equality and cannot be supplied as an accepted serialized field. Deserialization creates a new bundle owner and empty cache. Native input, serialized bundle/page hashes, coverage and per-response SQLite acknowledgements remain unchanged. External `Reader::from_bytes` still binds the exact supplied encoding, including noncanonical whitespace. This removes repeated full source copies and full-value equality on reader/transport clones and cache hits; it does not remove canonical snapshot serialization, persistent duplicate storage or large-project compaction work.

`pinned-context-v1` binds the run ID, specification revision, logical turn ID, input hash, complete prepared-prompt hash and schema hash. Documents are addressed by their original SHA-256, with byte lengths and original source references. Identical bodies share storage while retaining every source reference. Duplicate source references, changed bodies/lengths, invalid scope hashes and unknown fields are refused. Source text is opaque: JSON-looking values, instructions and wrapper-shaped data cannot create references or change authority.

Runtime bundles also register the already captured `Run.sources()` bodies under their original `file:`, `requirement:`, `plan:` and objective references. This performs no new filesystem read and does not expand a directory selector in the specification. Captured directories remain available both as full original envelopes and, when their captured kind is known, as indexed original text members. New interview rounds expose their complete pre-contract captures; older metadata-only rounds cannot claim source inspection from those packets. Complete all-source-kind registration, durable deduplicated blob storage, compaction and substantive repository/task acceptance remain open.

The source index is itself a paged document. Each `read_context` call supplies the exact bundle hash, a listed document/index hash, a UTF-8 byte offset and a maximum page size (4–32,768 bytes). Responses carry the exact bytes, continuation offset, complete document length, chunk hash and scope-bound receipt. A page boundary never splits a UTF-8 character. Integer-valued JSON numbers such as `512.0` are accepted without rounding; fractional, negative, string and inexact floating-point counts are refused. This matters for MCP clients that encode arguments through protobuf numeric values.

The stdio server exposes one tool and no file/path, URL, environment, command, write, delegation or subscription operation. It loads only the operator-supplied bundle after verifying its exact file hash, private ownership/permissions and regular-file type. Symlinks, multiple hardlinks and special files are refused; FIFO inspection is nonblocking. The source file need not remain present or unchanged because reads use the saved snapshot, never the live filesystem.

The internal process entry point is:

```text
antislop __context BUNDLE_FILE EXPECTED_FILE_HASH NEW_RECEIPT_FILE
```

Bundle/receipt parent directories must be private and owned. A receipt file is created exclusively with mode 0600; an existing file is never overwritten. Its directory entry is synchronized, then every receipt is synchronized before attempting to return source bytes. A failed receipt commit aborts the server before that response. A receipt proves which pinned byte response was prepared, not successful native delivery, semantic inspection, task correctness or current-source freshness. Retrying an immutable read is safe but can create another response receipt; this is not an exactly-once provider guarantee.

The file-based service above remains an explicit standalone diagnostic interface. Ordinary `pinned_context` turns instead use the daemon-owned bridge and canonical turn checkpoints described above. Original references cannot be replaced by unavailable links or summaries. Compaction and semantic completeness remain separate obligations. Each raw page currently returns at most 16 KiB even if a larger maximum is requested, leaving space for JSON escaping/receipt metadata in native tool responses.

## Transport and dependency decision

The server implements the small [MCP 2025-06-18 stdio transport](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports), [initialization lifecycle](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle) and [tool interface](https://modelcontextprotocol.io/specification/2025-06-18/server/tools), using the existing bounded, duplicate-key-rejecting JSON transport. This is not a general MCP SDK or HTTP server. The official [Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk) was considered: it provides a Tokio-based general client/server runtime, while this synchronous subprocess needs initialization, ping, static tool discovery and one read-only call. Reusing the existing synchronous framing avoids introducing an asynchronous runtime/dependency graph for that narrow surface. No Cargo dependency or license inventory changed. A broader MCP implementation should reassess this decision rather than extend a bespoke general framework.

## Native evidence and limits — 2026-10-15

The integrated provider subsequently passed an explicit native test through the actual `Runtime::observed_turn` implementation and SQLite session checkpoints, not only the standalone reader. It delivered all 528,484 prepared-source bytes and the 631-byte index in 39 committed/native-observed responses, returned both random sentinels, and retained the same context/output after normal runtime reattachment. Native session: `460ebe96-ab7b-47fb-9675-5188f26dd330`; elapsed: 142.11 seconds; log: `/tmp/architect-native-context-runtime-complete.log`; retained fixture: `/var/folders/d2/vtn09lpd2717th937yj7smhw0000gn/T/antislop-native-context-runtime-eLjWiV`. The diagnostic run was deliberately cancelled after its completed native turn; this is not complete task/DAG/quality/delivery acceptance. An ignored unit test, `runtime::turns::tests::native_context_adapter_uses_actual_runtime_checkpoints_and_survives_reattachment`, reproduces that path with an explicit configuration and native usage.

Two earlier integrated attempts failed because large completed native tool frames omitted `tool_info.output`. The adapter now authenticates such completions against actual committed backend responses, while still checking every supplied echo exactly. A native completion without a real backend read is rejected. No missing-body frame creates a source observation by itself. The passing runtime experiment above exercised this actual CLI 1.3.5 behavior.

The documented [Antigravity text-block input](https://antigravity.google/docs/cli/headless/) does not bypass the observed input ceiling: a genuine CLI 1.3.5 experiment joined 238,425 bytes of text blocks and retained a truncation marker after byte 192,000, removing 46,846 bytes. The final sentinel was missing; the experiment failed. Native session: `7121a9e3-fcd3-4cea-98d5-c0094c544b5d`; log: `/tmp/architect-agy-block-probe-canonical.log`.

A subsequent explicit reader experiment used the configured `gemini-3.8-flash-high`/high route, an audited home, a disposable controlled profile/cwd and the local server above. It completed in 29.40 seconds with clean exit and both random sentinels. Three actual synchronized response receipts authenticated the index (bytes 0–616), source bytes 119,000–119,512 and source bytes 237,914–238,426. The model's short input did not include either sentinel. The 238,426-byte source exceeded the ordinary prompt ceiling. Native session: `287bb365-1688-4e9d-9107-60a8175a5bda`; retained fixture: `/private/var/folders/d2/vtn09lpd2717th937yj7smhw0000gn/T/antislop-agy-block-probe-jjlsY3`; log: `/tmp/architect-pinned-context-native-integer.log`. The original probe report's source counter excluded the one newline joining its blocks; byte receipts establish the complete joined length above, and the driver now reports that length explicitly.

Earlier probe attempts failed executor construction or repeatedly refused integral numeric arguments; one was explicitly stopped after observed refusals, and all owned processes are terminal. The reader now accepts exact integral JSON numbers and the experiment stops on surfaced tool errors. The successful disposable profile inherits only its audited local MCP configuration, keeps `tools: [finish]` and uses the CLI's inherited MCP mapping; listing `call_mcp_tool` as an ordinary explicit component did not work in the initial probe. This does not establish an OS sandbox or change the ordinary provider's tool-free capability claim. The separate [Antigravity MCP documentation](https://antigravity.google/docs/mcp/) describes local stdio configurations.

These are genuine native context-transport observations; the integrated experiment additionally covers ordinary runtime receipt authentication and reattachment. Neither establishes substantive source inspection or a passing Antislop task. Binary-source representation, complete full-repository/interview retrieval, compaction, all supported versions/platforms and end-to-end run acceptance remain open.

The two explicit native probes in `tests/agy_context_live.rs` are ignored in ordinary tests. They require an explicit configuration path and consume native CLI usage:

```sh
ANTISLOP_CONTEXT_PROBE_CONFIG=/absolute/path/architect.toml \
  scripts/cargo.sh test --locked --test agy_context_live native_scoped_context -- --ignored --nocapture
```
