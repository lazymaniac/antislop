# Protocol regression fixtures

`codex-exec.jsonl` is a synthetic regression transcript matching the observed Codex 0.162.1 exec envelope. Session/item IDs and usage are invented fixture values, not captured user sessions or measurements. It covers session identity, unknown event tolerance, Unicode, final messages and completion. Fragmented/invalid/oversized transport cases are generated in the Rust tests.
