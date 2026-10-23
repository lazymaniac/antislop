# Source size diagnostic

Run the diagnostic from the repository root with:

```sh
cargo run --locked --bin diagnostic-size-check -- --root .
```

It scans every Rust file under `src`, including unreferenced files and inactive cfg branches. It reports files over 299 physical lines and callable declarations over 29 lines. Callable spans include attached outer attributes, documentation and leading comments, signatures, bodies, interior comments and blank lines. The line count treats LF and CRLF alike and does not count a final newline as an extra line.

External modules resolve every explicit `#[path]` override, including conditional alternatives. A source file stays in production scope if any production path reaches it.

A nonzero exit means either a size violation or an incomplete classification. In particular, `cfg_attr`, ambiguous or unresolved module paths, unresolved includes, parse failures and item-position macros fail closed. Do not use the inventory as a passing gate while those errors exist. This checker is diagnostic only; it is not enabled in CI.

No generated-source exclusions are configured. A generated path may be excluded only after its exact path and generator provenance receive independent review.

The AST parser is `syn` with source mapping for comments and blank lines. Run its black-box fixtures with:

```sh
python3 tests/size_check/check.py
```

The fixture runner retains per-case exit codes and matched diagnostics in
`tests/size_check/validation-results.txt`. It also captures the current
worktree diagnostic output in `tests/size_check/diagnostic-run.txt` so the
measured violations and classification errors can be reviewed independently.
Formatting, binary compilation and Clippy output are retained in
`tests/size_check/tool-validation.txt`.
