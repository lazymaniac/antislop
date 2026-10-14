# Self-host benchmark audit

The explicitly approved supplemental Check completed successfully: receipt `5157e37e-c9d8-465e-a7ae-c6fb3567af72`, revision 1, actual Execution artifact `e674a4e765abdfbb245932e8bdfb541aae4b80759638b0640d26f4df1992a23e`. All 450 child measurements completed with exit/status and output assertions satisfied. The checker exited zero, emitted 83,210 stdout bytes, empty stderr, and no truncation. This is benchmark evidence, **not full quality acceptance or a completed self-host run**.

The receipt's output SHA-256 is `cd7f8dff7ba73bc84db97fcf79e4ea8cb950f377c43bf58267f4fa79765363db`; it was independently recomputed from the persisted wrapper output. Pinned input hash remained `ab12b49db6afdc59a86ea1d280b961651b6ed63640833c699849ff39c807f297`. The parsed observations file equals the actual runner stdout JSON. Evidence is retained in `/tmp/antislop-selfhost-00wizn49/after-benchmark-run.json` and `/tmp/antislop-selfhost-00wizn49/supplemental-benchmark-observations.json`.

Actual baseline binary SHA-256: `facdb7b1f0d30bd60afae8e7b5f5969736dfe83d05ed0a86a5b3ec3a5e1f0726`; current binary: `9417f7e56dc656bd29d11f492cdb1541f82e8c382dbcd29f530333fc64be64f4`. The 105-file baseline manifest SHA-256 is `d377b62248d090e7f311fbd229af16d3d858b1454e54abf9777a15e7fea4a911`. Current `src/main.rs` is `1832819d2fbf83698375d586deb22466fe3fa261975f115989b6043e9b8c37c4`.

For normal current `--version`, observed wall-time p50/p95/p99/max were 2.968/3.358/3.403/3.418 ms; median per-child peak RSS was 2,048,000 bytes. These are process-level measurements on this host, not isolated branch costs or guaranteed latency bounds. Median paired current/baseline wall-time ratios were 0.987 for help and 1.011/0.983/1.000/1.015 for unknown argv lengths 32/256/2048/8192. No statistical significance or performance pass threshold is inferred.

`scripts/selfhost_benchmark.py` embeds the entire program and baseline manifest bytes in argv. It pins canonical baseline/current binaries by SHA-256, verifies the complete baseline source manifest, permits only `src/main.rs` to differ in the current source, and repeats source/binary checks after measurement. It builds nothing and invokes no shell, provider or package manager. Compiler identity and flags require the retained baseline/current build provenance; matching source/binary hashes alone cannot establish those facts.

The paired workloads are `--help` and unknown arguments of 32, 256, 2048 and 8192 bytes. Each has 30 pairs with a seeded, shuffled and balanced first-binary order (15 baseline first, 15 current first). Every invocation checks the expected exit code; stdout/stderr byte counts and hashes must match between binaries and remain identical throughout that workload. Candidate-only `--version` measurements use normal argv and trailing arguments at the same four sizes, 30 samples each. Each must produce exact manifest-derived version stdout, exit zero and empty stderr. These 150 version samples have no equivalent successful baseline workload and establish no speedup. Total: 450 child invocations.

Raw numeric sample arrays have explicit column names. Per-child `os.wait4(pid, ...)` supplies user/system CPU time and peak RSS; macOS bytes and Linux KiB are normalized to bytes. Wall time uses `perf_counter_ns` and includes process startup, scheduling and 250-microsecond wait polling. RSS includes launcher/pre-exec accounting, not a steady-state allocation profile. No warmup or cache flushing is performed. Interpolated p50/p95/p99/max and paired latency ratios are descriptive; 30 samples provide no statistical confidence or exhaustive OS argument-limit guarantee. No performance threshold is invented by the helper.

The default individual timeout is two seconds. A timeout kills and reaps the specific child and aborts the entire batch. Children remain in the enclosing Check guardian's process group, so runner completion also cleans same-group descendants. There is no detached-process containment claim. Use the generated argv through the supervised Check, not as an unsupervised process manager.

Regenerate after any program, manifest, source or binary change; this creates a different approval scope:

```sh
experiment_root=/tmp/antislop-selfhost-00wizn49
python3 scripts/selfhost_benchmark.py \
  --baseline "$experiment_root/benchmark-baseline/source/target/debug/antislop" \
  --current "$experiment_root/source/target/debug/antislop" \
  --baseline-manifest "$experiment_root/benchmark-argv-manifest.json" \
  --output "$experiment_root/supplemental-benchmark-argv.json"

./target/debug/antislop check "$experiment_root/state" \
  c77f6677-f800-483d-bc8a-f04d43b89b10 \
  "$experiment_root/supplemental-benchmark-argv.json" --approve-scoped
```

Check requires a stopped nonterminal run with reconciled effects and unchanged pinned inputs. The check records evidence without changing the immutable task, stage or attempt. A failed benchmark remains failed evidence.

Validation: `cargo test --locked --test selfhost_benchmark` passed three fixtures: all paired/raw statistics and version scaling; exact-output equivalence plus post-approval binary mutation rejection; and an individual timeout with confirmed descendant launch followed by guardian cleanup. Independent review tests are tracked separately in `tests/verifier_selfhost_benchmark*`.
