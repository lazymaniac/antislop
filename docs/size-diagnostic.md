# Source size diagnostic

Run the diagnostic from the repository root with:

```sh
CARGO_TARGET_DIR=.antislop/target cargo run --locked --bin diagnostic-size-check -- --root .
```

It scans every Rust file under `src`, including unreferenced files and inactive cfg branches, plus Rust files reached through module paths outside `src`. This includes `tools/size_check/checker.rs`, which is linked from `src/bin/diagnostic-size-check.rs`. A file is excluded only when test-only paths reach it and no production path reaches it. Test-only callable declarations are excluded from callable limits; a mixed production file still counts every physical line, including its test code.

The checker reports files over 299 physical lines and callable declarations over 29 lines. A callable span starts at its earliest attached outer attribute, documentation line or leading comment, or at the signature when no prefix is attached. Contiguous comment and attribute prefixes attach. A blank line attaches only when the preceding line is itself a recognized prefix; a run of blank lines stops attachment. Inner doc comments are not outer callable prefixes. Spans end at the closing brace or terminating semicolon and include every physical line in between: attached prefixes, intervening blanks, multiline signatures, braces and interior comments. Comment-looking text in strings is not a prefix. LF and CRLF are counted as one physical line each, and a final newline does not create an extra line.

External modules resolve every explicit `#[path]` override, including conditional alternatives. Collected files, module edges and the source root are normalized to the same relative path identity, so the documented `--root .` invocation classifies the same graph as an absolute-root invocation. Normalization removes `.` and collapses a parent component only when a preceding normal path component exists; repeated leading `..` components remain intact. A source file stays in production scope if any production path reaches it.

A nonzero exit means either a size violation or an incomplete classification. In particular, `cfg_attr`, ambiguous or unresolved module paths, unresolved includes, parse failures and item-position macros fail closed. Do not use the inventory as a passing gate while those errors exist. This checker is diagnostic only; it is not enabled in CI.

No generated-source exclusions are configured. A generated path may be excluded only after its exact path and generator provenance receive independent review.

The AST parser is `syn` with source mapping for comments and blank lines. Run its black-box fixtures with:

```sh
python3 tests/size_check/check.py
```

The fixture runner builds under `.antislop/target`; it creates each synthetic source tree under `.antislop` and removes it after the checker invocation. It records each case's exit code, matched diagnostics, and fixture-tree cleanup in `tests/size_check/validation-results.txt`. The earlier system-temporary-directory behavior and its observed paths are retained in `tests/size_check/fixture-artifact-pre-fix-reproduction.txt`. The runner captures the current worktree diagnostic in `tests/size_check/diagnostic-run.txt`. The initial inventory and raw pre-change output are retained in `tests/size_check/baseline-inventory.md` and `tests/size_check/initial-diagnostic-run.txt`; the separately measured linked checker evidence is in `tests/size_check/initial-checker-diagnostic-run.txt`. The inventory is not a passing gate while it lists violations or classification errors. This diagnostic remains manual and is not enabled in CI.
