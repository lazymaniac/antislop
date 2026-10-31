# Source size diagnostic

Run the diagnostic from the repository root with:

```sh
CARGO_TARGET_DIR=.antislop/target cargo run --locked --bin diagnostic-size-check -- --root .
```

It scans every Rust file under `src`, including unreferenced files and inactive cfg branches, plus Rust files reached through module paths outside `src`. This includes `tools/size_check/checker.rs`, which is linked from `src/bin/diagnostic-size-check.rs`. A file is excluded only when test-only paths reach it and no production path reaches it. Test-only callable declarations and embedded test-only lines are excluded from callable counts; a mixed production file still counts every physical line, including its test code.

The checker reports files over 300 physical lines and callable declarations over 30 lines. A callable span starts at the first signature line and ends at its closing brace or terminating semicolon. It includes every physical line in that range: multiline signatures, braces, blank lines, and comments. Test-only lines embedded in that range are excluded from the callable count. Attributes, documentation, and comments before the signature do not count. Comment-looking text in strings is not a prefix. LF and CRLF are counted as one physical line each, and a final newline does not create an extra line.

External modules resolve every explicit `#[path]` override, including conditional alternatives. Collected files, module edges and the source root are normalized to the same relative path identity, so the documented `--root .` invocation classifies the same graph as an absolute-root invocation. Normalization removes `.` and collapses a parent component only when a preceding normal path component exists; repeated leading `..` components remain intact. A source file stays in production scope if any production path reaches it.

A nonzero exit means either a size violation or an incomplete classification. In particular, `cfg_attr`, ambiguous or unresolved module paths, unresolved includes, parse failures and item-position macros fail closed. Do not use the inventory as a passing gate while those errors exist. This checker is diagnostic only; it is not enabled in CI.

No generated-source exclusions are configured. A generated path may be excluded only after its exact path and generator provenance receive independent review.

The AST parser is `syn` with source mapping for comments and blank lines. Run its black-box fixtures with:

```sh
python3 tests/size_check/check.py
```

The fixture runner builds under `.antislop/target`; it creates each synthetic source tree under `.antislop` and removes it after the checker invocation. It records current results under `.antislop/size-check/`, keeping runtime output out of source fixtures. Historical pre-change evidence remains in `tests/size_check/`. The inventory is not a passing gate while it lists violations or classification errors. This diagnostic remains manual and is not enabled in CI.
