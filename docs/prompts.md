# Prompts

Prompts are generated from the current run; they are not a collection of standalone Markdown templates.

| Part | Source |
| --- | --- |
| Executor instructions and context | [`execution_request`](../src/providers/prompts.rs) |
| Independent verifier instructions and expected checks | [`verification_request`](../src/providers/prompts.rs) |
| Shared quality instructions | `quality_instructions` in the same module |
| Inline context encoding and reconstruction instructions | `render_context` in the same module |
| Required response JSON schemas | `execution_schema`, `stage_quality_schema` and the verifier schema in the same module |
| Machine-owned workflow, session and policy context | [`orchestration.rs`](../src/orchestration.rs) |
| Shared agent inbox/outbox instructions and response field | [`PreparedTurn::with_context`](../src/providers/mod.rs), [`outbox.rs`](../src/communication/outbox.rs) |
| Additional immutable communication packet | [`ContextPacket::append_to`](../src/communication.rs) |
| Antigravity agent definition | `AGENT_SOURCE` in [`agy.rs`](../src/providers/agy.rs) |
| Risk catalog used in verifier context | [`fixtures/risks.json`](../fixtures/risks.json) |
| Orchestrator answer/team proposal and dedicated review | [`planning.rs`](../src/orchestration/planning.rs): `executor_request`, `verifier_request`, `REVIEW_CONTRACT_VERSION` |

Run specifications, pinned files, artifacts, evidence and previous reviews supply the remaining task data. Fixed-stage invocations use the shared executor/verifier generation; orchestration uses its own schemas and the same native receipt transports. Native CLI internal system instructions are owned by those tools and are not fully defined in this repository.

Orchestration stores its exact immutable request context, raw native outputs, receipt IDs, product checklist version and orchestration review-contract version in each round. `team_plan_json` is a complete JSON-encoded TeamPlan string, decoded with duplicate-key rejection and the existing typed fresh-plan validator. This reuses the full domain contract instead of maintaining a second plan representation. Runtime assigns profiles and observed session identities. Prior dialogue includes exact prior answers/reviews and source/version metadata; it does not recursively embed earlier request contexts. Those contexts remain in the local round records. The verifier receives only this round's current source/output read receipts and the actually observed executor identity. V01 requires the input receipt; V04 requires the output receipt. A plan review also accounts for every original criterion, plan item, orchestration facet and risk owner. Risk ownership and planning review do not establish execution quality or complete a product requirement.

The shared native-turn preparation adds the `agent_messages` array and exact known agent IDs to every runtime-observed executor, verifier and orchestration prompt/schema. The outbox carries typed direct/broadcast targets, kinds and text. Runtime derives the sender from the observed turn and commits recipient fanout with completed output. It is not a tool grant, independent review or live-delivery claim. Native hook transport is not yet implemented. The management context is deliberately report/message based: child transcripts and unrelated private inboxes are excluded; requesting missing facts through an addressed question is the expected flow.

## Exact inputs from the live self-host test

These reconstructed pre-send inputs match the hashes stored in the session ledger:

- Validation: `/tmp/antislop-v7.1-independent-presend-prompt.txt` and `/tmp/antislop-v7.1-independent-presend-schema.json`.
- Integration: `/tmp/antislop-v8-independent-presend-prompt.txt` and `/tmp/antislop-v8-independent-presend-schema.json`.
- Delivery attempt 10: `/tmp/antislop-v8-independent-delivery-presend-prompt.txt` and `/tmp/antislop-v8-independent-delivery-presend-schema.json`.
- Delivery attempt 11: `/tmp/antislop-v9-independent-presend-prompt.txt` and `/tmp/antislop-v9-independent-presend-schema.json`.
- Delivery attempt 12: `/tmp/antislop-v10-independent-presend-prompt.txt` and `/tmp/antislop-v10-independent-presend-schema.json`.

A durable local copy of the audited prompts, matching pre-send snapshots and audit helpers is in `.antislop/proofs/c77f6677-f800-483d-bc8a-f04d43b89b10/prompts/` (ignored by Git). For example, `antislop-v10-independent-presend-prompt.txt` is the exact ledger-bound Delivery12 verifier input. This directory contains audited reconstructions, not every historical prompt.

The corresponding permanent audit metadata is in [`docs/selfhost`](selfhost/README.md). The `/tmp` files are temporary. The ledger currently stores prompt/schema hashes and durable provider outputs, rather than every complete prompt body. Consequently these audited reconstructions are not a complete archive of every historical input. A current renderer also cannot recreate an old input byte-for-byte without the matching historical renderer and pre-send state.

Chat intake and reviewed task/configuration proposals use `runtime/intake.rs`, the immutable interview context in `runtime/orchestration.rs` and the response/review contracts in `orchestration/planning.rs`. `intake_proposal_json` carries a task definition, unpinned configuration and rationale; runtime owns the workspace and computes the pin. The reviewer receives the exact proposed source observations and must cite the `runtime:intake-sources` receipt. New contract acceptance binds its actual native review input and starts no project task or check. Predefined role prompts remain runtime-owned when the subsequently reviewed team is installed.
