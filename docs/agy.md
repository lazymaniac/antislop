# Antigravity CLI adapter

The canonical provider is `agy`; the validated CLI versions are **1.3.4, 1.3.5 and 1.4.0**. The adapter uses the CLI and its own authentication. It does not call a model API directly, copy credentials, change global settings, or expose the task workspace as the native CLI cwd.

On 2026-10-16 the installed CLI began reporting **1.4.0**. The initial owned
replacement was refused before a new turn. Before changing admission, an owned
source candidate and exact copied binary were tested through the actual adapter:
192,000-byte inline input retained both unpredictable UTF-8-edge sentinels;
224,768-byte prepared input used 22 committed/native-observed reader responses,
covering all 226,077 required index/prompt bytes. Both ledgers passed reopening
and validation from SQLite. The combined probe took 162.78 seconds; copied and
installed binary SHA-256 matched
`a4502aaf310b2fd6eb657fcfec2fcc31a690fbbd0c2d14c8eefbb556409024ce`.
Report: `/var/folders/d2/vtn09lpd2717th937yj7smhw0000gn/T/antislop-agy-version-Bdea2Z/report.json`.
The probe does not establish project quality, complete task delivery, OS
containment or native resume. Admission now includes that exact version;
1.4.1, prerelease spellings and other unknown versions still fail without a
downgrade or fallback.

Version 1.4.0 introduces terminal OSC 7501 status reporting according to its
[official release notes](https://github.com/google-antigravity/antigravity-cli/releases/tag/1.4.0).
Every adapter version/help/turn invocation explicitly sets
`AGY_CLI_OSC7501=off`; arbitrary inherited `AGY_*` overrides remain excluded.
This keeps status sequences and pending-command details out of the NDJSON
transport without rewriting global settings. `tests/agy_version_live.rs` is the
opt-in exact-binary probe; normal tests do not use authenticated providers.

For a legacy spec, set `"provider": "agy"` and optionally `"model": "gemini-3.8-flash-low"`. For role-specific routing, declare the provider and assign executor/verifier profiles explicitly in the pinned configuration:

```toml
[providers.agy]
binary = "agy"
isolation = "tool_free"
native_effort = true
# managed_home = "/absolute/private/cli-home"  # optional, separately authenticated

[providers.agy.effort_map]
light = "low"
standard = "medium"
deep = "high"

[profiles.agy-review]
provider = "agy"
model = "gemini-3.8-flash-low"
effort = "standard"
work_kinds = ["context", "planning", "implementation", "testing", "integration", "delivery"]
roles = ["verifier"]
```

Set each desired routing entry's `verifier` to `agy-review`. Executor profiles can use the same provider with a separate role assignment; the runtime reserves fresh logical/native sessions for independent reviews. `examples/architect.toml` includes an optional provider declaration without changing its existing role assignments. `agy models` lists native model slugs; unsupported model/effort combinations remain CLI failures.

## Native profile and startup audit

The adapter creates a private controlled cwd and an exact owned `.agents/agents/antislop-stdio.md` with `tools: [finish]`, `excludeDefaultComponents: true`, `inheritCustomizations: false`, `inheritMcp: false`, `subagent: false`, `commandExecutionPolicy: "off"`, and empty MCP/skill/plugin declarations. `finish` is the native structured-output component; excluding it produced successful native turns without usable `structured_output` in the initial probes. The installed binary exposes `FinishToolConfig.GetResultJsonSchemaString`, and the actual `finish` profile returned the schema-constrained object.

This is a CLI configuration boundary, **not OS containment**. `--sandbox` is passed but no broad filesystem, network or detached-process isolation guarantee is inferred. External tool and subagent events are rejected; planner text and reasoning never become the artifact. Process groups and the guardian lifeline provide cancellation cleanup, with the existing detached-process limitation.

Every probe and turn audits the selected CLI home read-only. Nonempty global hooks, MCP configurations, plugin/agent/skill/rule/workflow directories, unsafe ownership/symlinks, or unsupported settings fail closed. The current supported ordinary settings are model, feedback, tool permission, trusted-workspace and verbosity preferences; optional documented authentication and strict deny-all settings are supported. Arbitrary editor, startup-script, custom endpoint and environment overrides are rejected. The environment inherits only ordinary process variables plus the CLI's documented `GEMINI_API_KEY`; arbitrary `AGY_*`, `NODE_OPTIONS`, ADC paths and endpoint overrides are excluded.

Without `managed_home`, the adapter uses the audited existing authenticated `HOME`. A managed home is optional and must already exist with safe ownership. Authenticate it interactively through `HOME=/absolute/private/cli-home agy`; the adapter never signs in or copies a credential cache for the user. A clean managed-home probe without copied credentials failed authentication before inference, while the existing authenticated home worked.

The native `init.tools` array in 1.3.4 is emitted before custom executor component resolution and lists the global registry. It is not treated as the effective selected toolset. A deliberately unknown component failed resolution after that same init catalog. Positive-control write and empty-component probes changed the owned sentinel behavior; neither absence of tool calls nor the catalog alone is used as an isolation proof. Internal `manage_task` list attempts seen during malformed-output probes are also rejected by the adapter.

## Protocol and evidence

Two owned native ContextAnalyst attempts submitted invalid task JSON with a
flattened context and no required `context.model`. The second failed at
`finish_component` after full required context delivery. Those candidates remain
diagnostics, not artifacts. Fresh accepted teams and explicitly approved
referenced task replacements now select versioned `task-prompt-v2` instructions:
the complete response schema plus explicit nested model/sections shape. This
does not relax stream ordering, finish-component identity, JSON validation or
role-specific semantic requirements. Retained legacy input bytes remain valid.

The following owned native attempt passed finish/schema/completion with all
required reader coverage. Runtime then found a citation-registry mismatch for
original captured directory members and actual baseline-check output. Those
exact source bindings now validate without changing canonical selectors or
approval hashes. The completed output was subsequently recovered through a
scoped read-only admission using its original runtime prompt, without repeating
the native turn or checks. It remains awaiting independent task review. Native
transport success and recovery do not establish semantic acceptance or delivery.

The separately selected `isolation = "pinned_context"` profile supports the validated 1.3.5 and 1.4.0 native MCP contracts. It routes immutable pages through a private daemon-owned bridge with commit-before-response callbacks, retains logical/native input bindings and checks actual native read completions against committed byte receipts. Every index/prepared-prompt byte must be delivered before acceptance; incomplete final output is retained as rejected data. This profile is explicitly not advertised as tool-free. Models, effort, policies and existing configurations remain pinned. See [context pages](context.md) for the protocol, UI and recovery behavior. The existing `tool_free` profile keeps the single-message behavior below.

New reader turns use `agy-pinned-context-v2`. Their short native prompt gives a concrete first `call_mcp_tool` invocation with `ServerName="antislop-context"`, `ToolName="read_context"` and the exact index page arguments. The owned agent profile repeats that routing contract. The index supplies document hashes; original source labels and inline-registry aliases are not other MCP servers or resource URIs. Native `list_resources`/`read_resource` and other discovery tools remain unsupported. Retained v1 prompts/hashes and Codex transport rendering remain exact. A read-only compatibility probe authenticated all nine completed reports from the owned genuine fixture after this version change; that compatibility check does not establish new model quality or complete task acceptance.

The complete prepared prompt has a conservative 192,000-byte UTF-8 preflight, including communication and recovery additions. Oversized input returns a redacted `Protocol/input_too_large` failure before home inspection, CLI probes, native launch or delivery callbacks. It never silently removes source bodies. A retained native review exposed actual input truncation at byte 192,000, removing 822,771 bytes and causing 26 honest blocked checks. Executable fixtures verify byte counting, preflight ordering and exact boundary delivery to the mock CLI; they do not establish genuine model acceptance at the boundary.

New orchestration rounds pin `review_input_format` in their immutable context. Antigravity reviews use the lossless inline UTF-8 registry: original sources, candidate, receipts, hashes and literal wrapper-shaped user data round-trip exactly. Earlier unmarked rounds preserve their original request format for authentication. Factoring alone did not fit the historical approximately 360-KB-per-part proposal into the ordinary prompt ceiling. Explicit pinned-context profiles now retrieve the complete original context through pages. Complete native task/quality/delivery acceptance, durable storage and compaction remain open.

The adapter sends one NDJSON `user` message through stdin and closes the pipe. It accepts a single observed conversation UUID, a completed native `finish`, matching echoed schema, `SUCCESS`, structured output and clean exit. Duplicate/late events, another native UUID, raw-text fallback, missing finish/schema/output, malformed JSON and external tool/subagent events fail. Unknown delivery is reserved durably before launch; callbacks must succeed before progression. A cancellation or protocol failure preserves observed identity and possible partial delivery.

The selected finish component has two supported event encodings: a bare `step_type="finish"`, and a `step_type="tool"` with both `tool_name` and `tool_info.name` exactly `"finish"`. The latter was observed during the owned CLI 1.3.5 review after complete context delivery. Conflicting identities, subagent metadata, component errors and malformed parameter shapes are rejected. ACTIVE is not completed finish. Component arguments, including native `toolAction`/`toolSummary` metadata, are never treated as the artifact; acceptance still requires the later full terminal structured result, exact session/schema, complete context and clean process exit. Tests cover both profiles and missing/foreign/error/active-only/nonzero-exit cases. This is the already configured output component, not permission to execute arbitrary tools.

Rejected tool/subagent events include only known protocol identifiers and field-presence flags in their diagnostic. Unknown names are redacted; arguments, agent text and subagent bodies are excluded. This aids diagnosis without admitting another tool or weakening the profile. Older retained generic errors do not establish the exact offending component. Unknown native step types remain rejected. Their diagnostic includes a bounded known label (otherwise redacted), ACTIVE/DONE state, numeric step index, a SHA-256 fingerprint of the type and field-presence flags; step text and system/error payload bodies are excluded. Scoped metadata-only inspection of two owned failed conversations found `list_resources` as each last recorded call, to the same non-owned server argument. Those stored calls suggest the missing native-frame trigger; the old stdout frame/state were not retained and cannot be reconstructed from that observation. No other native conversations or reasoning text were inspected.

Stream-guard errors additionally retain a fixed `phase` and exact allowlisted `boundary` category
for ordering, conversation/finish/schema and reader-call/receipt bindings. Catalog,
native step-state and finish-component normalization guard failures now
also have fixed allowlisted categories. Their original state, component payload
and error text are not copied. These categories do not change rejection rules.
Unknown callback/storage errors retain their fixed phase with `boundary=unclassified`;
their original text is not copied.
The categories do not relax any guard or retroactively identify old generic
failures. The owned review that stopped after 74 committed/native-observed reads
and ten completed reports returned the older generic ordering/structure error;
its missing exact stdout frame cannot be reconstructed from the SQLite numeric
step metadata. A read-only probe validated the ten-report prefix while confirming
that turn's required context was still incomplete (126,574/298,190 UTF-8 bytes).

A native `error_message` step normally makes the turn inadmissible. The adapter immediately stops and reaps its owned process group. Output-token exhaustion is classified as `Protocol` with the fixed diagnosis `output_token_limit_exceeded`; raw error text is not persisted. This path preserves observed identity and possible delivery and never emits a completed turn. Executable fixtures verify prompt return and descendant cleanup; genuine large-plan review remains an open acceptance case.

The first owned task-review wave failed in all three native sessions after full
required input delivery. Their retained error-message payloads contained
`Malformed function call`; no valid structured review was produced. New known
error code/message payloads now get the separate fixed `Protocol` diagnosis
`malformed_function_call`. Attempted arguments remain private and are never
parsed into actions or artifacts. Historical generic failures are not rewritten.
The explicitly selected ContextAnalyst verifier also stopped after 844.11 seconds
with all 355,766 required bytes observed, 91 reader responses and no accepted
review or correction event. Its original runtime diagnosis remained
`Process/unknown`; a read-only examination of its exact owned native error record
confirmed the malformed-call marker. The discarded NDJSON carrier is unknown,
so this does not prove which public field the CLI emitted.
The adapter now also recognizes a string `text_delta` on an `error_message`
step, after non-null explicit error carriers. It never diagnoses an ordinary
agent response as an error. Conflicting carriers and a non-string delta cannot
enable correction. Unhandled failures retain only fixed allowlisted field/type
metadata (`error_shape`), without raw keys, text or attempted arguments, so an
absent native error body can be distinguished from an unsupported carrier.
These fixtures do not establish that this was the actual discarded frame shape.
For exactly CLI 1.4.0 there is now a narrow exception: a DONE native
error-message envelope with the known malformed-call marker can remain in its
same supervised process. It requires an observed current UUID, completed send,
an advancing stable step index, no completed finish, no outstanding reader call
or unobserved committed response, and no conflicting error code, known fatal
prefix or explicit non-retryable flag. Generic nested diagnostic matches do not
enable this path. A typed `NativeOutputRejected` event must durably commit before
reading onward; it stores only the native step and fixed reason. It does not
claim that a retry occurred or completed. All subsequent steps must advance
past the rejected step. The original successful terminal result, new completed
finish, echoed schema, full context coverage and clean exit remain mandatory.
There is no resend, new conversation, argument repair, inferred artifact or
automatic replacement. A second narrow 1.4.0 case permits a reasonless DONE error-message envelope:
only known metadata fields, absent/null error carriers, a valid optional duration
and object-shaped optional usage. It records `UnclassifiedNativeStep`, without
attributing a model-format cause. All the same identity, index, reader and
terminal guards apply. Unknown fields and malformed metadata still fail.
This follows an owned native attempt whose task diagnostic recorded every known
error carrier as absent; its 117 committed/native-observed reads produced no
accepted review. Genuine continuation for this new case remains unproven.
Other versions/errors retain the stop behavior. A failed
event commit cancels and reaps the process and descendants. Progress shows the
committed rejection count while awaiting a valid result; history retains these
observations without changing legacy turn hashes. Genuine same-session CLI
correction acceptance remains under evaluation; fixture success is not that
proof.

Only terminal usage is emitted, marked cumulative for the native session; per-step counters are ignored to avoid double counting. **Native resume is currently unsupported by this adapter.** The installed CLI advertises `--conversation UUID`, but the actual adapter resume probe emitted an unvalidated native step and failed closed. The capability is disabled and valid UUID resume requests are rejected before probes or delivery; aliases are also rejected. There is no `--continue`, guessed fallback or automatic replay of an uncertain runtime turn.

Task recovery now also checks the resolved adapter/profile's implementation
before offering or admitting exact-ID resume. For Antigravity, the guided UI
offers a distinctly new session with the source-bound retained handoff after
explicit checkpoint reconciliation and approval. It preserves old failed turns,
completed sibling artifacts and checks; it does not pretend to resume the old
native UUID or approve an invalid finish candidate. Actual CLI/version preflight
still runs before launching any newly approved turn.

The sanitized fixture `fixtures/protocol/agy-success.jsonl` retains the first successful native schema/output/usage, replacing UUID/cwd and abbreviating the registry while omitting planner text. That manual transport smoke completed in 6.648 seconds with `{ "ok": true }`, 795 input and 136 output tokens. The **actual adapter** subsequently completed a fresh turn after its own startup audit and protocol validation: 7.180 seconds, 795 input/29 output tokens, five durable-callback events including one usage and one completion, and no stderr. The structured output SHA-256 was `4062edaf750fb8074e7e83e0c9028c94e32468a8b6f1614774328ef045150f93`. Its exact-ID resume probe failed on an unknown native step, so resume remains disabled. These are protocol smokes, not task-quality acceptance; no task sources or runtime Run ledger were involved.

The public Go CLI help text is written to stderr. A bounded help-only capture handles this explicitly; normal turn stderr still passes only through static diagnostic classification and is never retained raw.

Validation includes offline executable fixtures for schema/finish/session ordering, literal UUID resume, cumulative usage, startup/custom-profile tampering, callback durability, cancellation and descendants, plus independent review fixtures. No paid native calls are part of the automated test suite.

Official references: [headless protocol](https://antigravity.google/docs/cli/headless/), [custom agents](https://antigravity.google/docs/subagents/), [component exclusion changelog](https://www.antigravity.google/docs/changelog?tab=cli), [permissions](https://antigravity.google/docs/permissions), [CLI authentication](https://antigravity.google/docs/cli/install/).

The local 1.3.5 adapter smoke used the configured `gemini-3.8-flash-high`/high profile and completed a fresh finish-only structured turn. The adapter admits explicitly validated versions, never a general version range; unknown versions still fail before inference. The CLI default blank `statusLine` object is accepted only with empty type/command and a boolean enabled flag. Executable status-line customizations remain rejected, and no global CLI settings are rewritten. Official release notes: [1.3.5](https://github.com/google-antigravity/antigravity-cli/releases/tag/1.3.5).
