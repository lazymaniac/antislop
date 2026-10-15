# Antigravity CLI adapter

The canonical provider is `agy`; the validated CLI versions are **1.3.4 and 1.3.5**. The adapter uses the CLI and its own authentication. It does not call a model API directly, copy credentials, change global settings, or expose the task workspace as the native CLI cwd.

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

The separately selected `isolation = "pinned_context"` profile supports the validated 1.3.5 native MCP contract. It routes immutable pages through a private daemon-owned bridge with commit-before-response callbacks, retains logical/native input bindings and checks actual native read completions against committed byte receipts. Every index/prepared-prompt byte must be delivered before acceptance; incomplete final output is retained as rejected data. This profile is explicitly not advertised as tool-free. Models, effort, policies and existing configurations remain pinned. See [context pages](context.md) for the protocol, UI and recovery behavior. The existing `tool_free` profile keeps the single-message behavior below.

The complete prepared prompt has a conservative 192,000-byte UTF-8 preflight, including communication and recovery additions. Oversized input returns a redacted `Protocol/input_too_large` failure before home inspection, CLI probes, native launch or delivery callbacks. It never silently removes source bodies. A retained native review exposed actual input truncation at byte 192,000, removing 822,771 bytes and causing 26 honest blocked checks. Executable fixtures verify byte counting, preflight ordering and exact boundary delivery to the mock CLI; they do not establish genuine model acceptance at the boundary.

New orchestration rounds pin `review_input_format` in their immutable context. Antigravity reviews use the existing lossless inline UTF-8 registry: original sources, candidate, receipts, hashes and literal wrapper-shaped user data round-trip exactly. Earlier unmarked rounds preserve their original request format for authentication. This removes repeated bodies but does not make arbitrarily large context fit: the retained proposal still measures about 360 KB per part. Complete source/context paging remains open, and no new native review was launched with that oversized packet.

The adapter sends one NDJSON `user` message through stdin and closes the pipe. It accepts a single observed conversation UUID, a completed native `finish`, matching echoed schema, `SUCCESS`, structured output and clean exit. Duplicate/late events, another native UUID, raw-text fallback, missing finish/schema/output, malformed JSON and external tool/subagent events fail. Unknown delivery is reserved durably before launch; callbacks must succeed before progression. A cancellation or protocol failure preserves observed identity and possible partial delivery.

Rejected tool/subagent events include only known protocol identifiers and field-presence flags in their diagnostic. Unknown names are redacted; arguments, agent text and subagent bodies are excluded. This aids diagnosis without admitting another tool or weakening the profile. Older retained generic errors do not establish the exact offending component. Unknown native step types remain rejected. Their diagnostic includes only a bounded known label (otherwise redacted), a SHA-256 fingerprint of the type and field-presence flags; step text and system/error payload bodies are excluded.

A native `error_message` step makes the turn inadmissible. The adapter immediately stops and reaps its owned process group rather than waiting through internal CLI retries. Output-token exhaustion is classified as `Protocol` with the fixed diagnosis `output_token_limit_exceeded`; raw error text is not persisted. This path preserves observed identity and possible delivery and never emits a completed turn. An executable fixture verifies prompt return and descendant cleanup; genuine large-plan review remains an open acceptance case.

Only terminal usage is emitted, marked cumulative for the native session; per-step counters are ignored to avoid double counting. **Native resume is currently unsupported by this adapter.** The installed CLI advertises `--conversation UUID`, but the actual adapter resume probe emitted an unvalidated native step and failed closed. The capability is disabled and valid UUID resume requests are rejected before probes or delivery; aliases are also rejected. There is no `--continue`, guessed fallback or automatic replay of an uncertain runtime turn.

The sanitized fixture `fixtures/protocol/agy-success.jsonl` retains the first successful native schema/output/usage, replacing UUID/cwd and abbreviating the registry while omitting planner text. That manual transport smoke completed in 6.648 seconds with `{ "ok": true }`, 795 input and 136 output tokens. The **actual adapter** subsequently completed a fresh turn after its own startup audit and protocol validation: 7.180 seconds, 795 input/29 output tokens, five durable-callback events including one usage and one completion, and no stderr. The structured output SHA-256 was `4062edaf750fb8074e7e83e0c9028c94e32468a8b6f1614774328ef045150f93`. Its exact-ID resume probe failed on an unknown native step, so resume remains disabled. These are protocol smokes, not task-quality acceptance; no task sources or runtime Run ledger were involved.

The public Go CLI help text is written to stderr. A bounded help-only capture handles this explicitly; normal turn stderr still passes only through static diagnostic classification and is never retained raw.

Validation includes offline executable fixtures for schema/finish/session ordering, literal UUID resume, cumulative usage, startup/custom-profile tampering, callback durability, cancellation and descendants, plus independent review fixtures. No paid native calls are part of the automated test suite.

Official references: [headless protocol](https://antigravity.google/docs/cli/headless/), [custom agents](https://antigravity.google/docs/subagents/), [component exclusion changelog](https://www.antigravity.google/docs/changelog?tab=cli), [permissions](https://antigravity.google/docs/permissions), [CLI authentication](https://antigravity.google/docs/cli/install/).

The local 1.3.5 adapter smoke used the configured `gemini-3.8-flash-high`/high profile and completed a fresh finish-only structured turn. The adapter accepts these two exact versions, not a general version range; other versions still fail before inference. The CLI default blank `statusLine` object is accepted only with empty type/command and a boolean enabled flag. Executable status-line customizations remain rejected, and no global CLI settings are rewritten. Official release notes: [1.3.5](https://github.com/google-antigravity/antigravity-cli/releases/tag/1.3.5).
