# M0 integration evidence

Historical: these probes were made for the old runtime, since deleted. The lean core's transports and the CLI versions it was measured on are in [lean-core.md](../lean-core.md#design-16-build-stages).

Provider probes were observed on macOS on 2026-10-13. Runtime tests also passed in a Linux Podman container; see `linux-runtime.json`. The previously stopped VM was stopped again after testing. Capability advertisements and successful protocol initialization do not prove that a model turn, resume, steering or cancellation works.

| Provider | Version | Observed | Open |
| --- | --- | --- | --- |
| Codex | 0.162.1 | App-server initialize/thread-start with read-only network-disabled sandbox; exec structured output, exact-ID resume retaining 37 → 42 context, live process cancellation | App-server full lifecycle/approval enforcement; Linux isolation; fine-grained provider error categories |
| Claude Code | 2.1.296 | Tool-free stream-json, structured output, exact-ID resume retaining 37 → 42 | Live cancellation, steering, approvals and Linux isolation |
| Gemini CLI | 0.63.0 | Isolated package-cache invocation, version/help, ACP initialization advertising loadSession | No local Gemini auth settings or credential environment detected; isolated headless turn exits 41. Resume/cancel/model execution remain unverified. |

Transport choices: restricted Codex exec JSONL for the first implemented slice; app-server as the richer follow-up once configuration/permission enforcement is proven. Claude stream-json is the candidate second adapter. Gemini ACP is the preferred candidate over headless because it advertises session loading; a real authenticated contract test must confirm it before acceptance.

`probe.json` is a repeatable baseline discovery report. Separate `*-session.json` reports record real model/session tests. `gemini-acp.json` records only initialization; its advertised capabilities remain unverified behavior. Gemini was fetched into `/tmp/architect-gemini-cache`; no global installation or credential copy was performed.

The live bugfix fixture reproduces incorrect negative magnitudes, keeps tests immutable, permits only `lib.rs` edits, and checks positive, zero, negative and i32::MIN inputs. The test driver sends normal supervised stage approvals. Its optional `--repair` flag supplies explicit repair approvals only within this disposable fixture.

Early live attempts exposed and rejected:

- A fabricated/nonmatching context source reference.
- Future test outcomes incorrectly classified as unknown inputs required before planning. The context contract now distinguishes missing planning inputs from pending validation.
- Missing baseline receipts in the verifier context: the executor could see actual runner evidence that the verifier could not. Both now receive relevant runner receipts; stale input snapshots remain rejected as current evidence. A runtime-generated before/after comparison supplies original source evidence for preservation checks.

These observed failures are useful negative evidence; they are not a successful end-to-end result. Only a completed `live-demo.json` with passing final tests establishes the accepted real slice.
