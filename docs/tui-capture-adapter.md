# TUI render capture adapter

The test-only adapter in `lean::tui::capture` invokes production `App::draw` on a fresh Ratatui `Terminal<TestBackend>`. It exports every width-by-height cell with coordinates, symbol, foreground/background, and modifiers. Each sample carries a target, state, and geometry identifier. The existing six baseline geometries remain: `minimum-12x5`, `compact-40x12`, `standard-80x24`, `wide-100x30`, `large-120x40`, and `large-160x50`.

The section 2 map has 22 logical headings from user108. `manifest.json.combinations` is the semantic 22 × 6 × 6 index: each heading, state (`ordinary`, `empty`, `waiting`, `error`, `no_history`, `compact`), and baseline geometry has either concrete sample references or an explicit `semantic_na` explanation. Applicability is a reviewed rule about the surface's model, independent of whether a fixture happens to exist. Contracts reject missing registrations, contradictory N/A, and references to a different surface. A heading can own several variants: for example the shell includes session and agent targets, and the picker heading includes the actual existing-session picker. That picker is **not** evidence of the New session surface; the latter uses the real Workbench goal composer.

The ordinary plan/history family contains both an active Plan viewer and a separate saved-history record loaded through `Browser::loaded`; neither selector counts as the other viewer. Each Workbench tab has its own target, actual Details tab (prepared through the existing handler helper), state model, and visible-content contract. Checks uses a reviewer, not a worker whose role has no Checks tab; its error specimen is a settled review issue with evidence, rather than an invented task-fetch error body. Results covers stored reports, absent reports, selected-document loading and failure; Comments covers a scoped retained comment and an empty collection; Diff covers loaded files, empty changes, loading, failure, and unavailable historical diff. Conversation's absent retained transcript is `no_history`, not a second distinguishable empty state. Native PTY no-history selects an unavailable historical round and requires the real historical-PTY notice; an empty live terminal is not treated as missing history. Summary includes queued/recovery, absent task data, and a completed **session** with branch and all three existing completion actions. The stopped-task actions heading references the actual Unblock panel, including a failed-task cause; integration summaries are owned only by Summary and never count as the actions dialog. The tab fixtures turn off the optional terminal split so their content can be inspected; the independent native fixtures retain the split threshold contract.

Configuration includes a clean parsed configuration, a validation error, applying status, and an unsaved draft with no provider/profile entries but retained scalar settings and routes. This last specimen means an empty editable provider/profile collection, **not** valid empty TOML or a zero-field editor (which the existing renderer does not support). No file is saved. `config-compact` takes the real compact renderer at 12×5. Separate compact Workbench specimens take the 40×12 layout without the task list or terminal split. Other surfaces whose model has no separate compact state still render their applicable data at all six geometries; compactness alone does not fabricate a new asynchronous or historical state.

The r13/g23 readiness inventory (174 captured, 42 responsive N/A, 576 missing) is historical diagnostic input. It is neither a count of semantically required new fixtures nor a claim about this export. The two readiness outputs and historical buffers remain untouched.

The matrix adds both sides of, and the exact point at, these boundaries: graph inspector content 110 columns and 16 rows; Workbench list 100 columns and 24 rows; split terminal 24 rows; and the loaded Diff file list 90 columns. Graph terminal geometries are wider/taller than the target graph area because the production main screen reserves chat and footer space; the named samples are validated by whether the inspector actually renders. Workbench records the post-draw `workbench_areas`, including actual content dimensions and list/split visibility. `workbench-tab-diff` loads a deterministic `FileDiff` fixture and checks its rendered filename plus counts, so the 90-column assertion covers the real content panel behavior.

Build in the allocated checkout, then execute the resulting test binary **from durable task state**. Cargo's `--manifest-path` does not relocate test cwd; integration helpers can write relative runtime paths. The following command requires `ANTISLOP_TASK_STATE` to name an existing durable directory outside the checkout. It creates a fresh run and never overwrites an export:

```sh
repo="$PWD"
run=$(mktemp -d "${ANTISLOP_TASK_STATE:?set durable task-state directory}/capture.XXXXXX")
CARGO_TARGET_DIR="$run/target" "$repo/cargo.sh" test --lib --no-run --message-format=json > "$run/build.jsonl"
binary=$(python3 - "$run/build.jsonl" <<'PYTHON'
import json, sys
for line in open(sys.argv[1]):
    value = json.loads(line)
    if (value.get("reason") == "compiler-artifact"
            and value.get("executable")
            and value["target"]["kind"] == ["lib"]):
        print(value["executable"])
PYTHON
)
(
  cd "$run"
  for mode in normal ascii no-color combined; do
    (
      unset NO_COLOR ANTISLOP_ASCII ANTISLOP_CAPTURE_SELECT
      case "$mode" in ascii|combined) export ANTISLOP_ASCII=1;; esac
      case "$mode" in no-color|combined) export NO_COLOR=1;; esac
      "$binary" lean::tui::capture:: --test-threads=1
      ANTISLOP_CAPTURE_DIR="$run/cells-$mode" "$binary" lean::tui::capture::export_catalog --exact --ignored --test-threads=1
    ) > "$run/$mode.log" 2>&1 || exit "$?"
  done
)
```

For a partial replay, set `ANTISLOP_CAPTURE_SELECT` on the binary invocation, for example `results-error@wide-100x30,workbench-tab-diff@diff-at-width`. It accepts comma-separated `fixture-id@geometry-id` keys; unknown, inapplicable, duplicate, or empty selections fail. Unselected samples in the full semantic index have `not_selected` status and are never counted as captures. Boundary samples are additional to the 792 baseline index entries and remain registered in `geometries`/`geometry_applicability`.

`final-renders.json` retains format `antislop-ratatui-testbackend-cells-v1`. Every record includes unique `sample` and fixture-qualified `name`, target/view/state/geometry, dimensions, every row-major cell with x/y/symbol/foreground/background/modifiers, reached-model flags, actual target/Details identity, visible state marker (a distinguishing prefix where the renderer clips long text), small model-state measurements including the full result/flow error, and measured panel signals. `manifest.json` lists fixture registrations, catalog ownership, all geometries, combinations, semantic reasons, and sample references. Multiple samples in one combination remain separate; captures from different runs are not added together as unique views. Debug color/modifier names preserve Ratatui distinctions. Native terminal fixtures use deterministic snapshots and start no CLI provider.

`responsive_na` is distinct from semantic N/A. It describes unavailable panels or a specimen's designated responsive mode, never an absent fixture. Existing minimum/modal and PTY/composer viewport exclusions remain explicit. A production rendering failure is **not** N/A: `rendering_finding` and `captured_with_finding` retain the required specimen and its full buffer. The historical capture-state-completion export (598 buffers) disclosed Flow status clipping at 12×5 and hidden history loading at 12×5, 40×12 and 80×24. Those measurements remain historical. The compact-status-repair input reserves visible status space and initializes the history loading document. The current adapter requires visible `Zapisywanie zmian` (`Zapis...` at 12×5), `Err PgUp/Dn` (`Err PgUp/D` at 12×5), and `loading` at every applicable geometry, independently of the full sending/error/loading model. The full Flow error must remain `Fixture flow update failed`. These cases are no longer classified as `captured_with_finding`; any missing marker now fails capture rather than becoming N/A.

Semantic checks follow painted provenance: the failed Unblock reason uses `Theme::error`, Flow sending uses `accent`, Flow error uses `error`, and the empty Flow form's measured upper-left border uses `separator`. Border measurement supports both Unicode corners and ASCII `+`. The history loading navigation uses `accent` over the focused `card` background on its actual `loading` cells, including the compact layout. Graph filters likewise require their own separator corner. The Diff boundary detector requires the filename and both counts with the mode’s real minus glyph (`−1` or ASCII `-1`), preserving the 90-content-column condition. Every cell of each selected marker must match, so unrelated headings or native/document cells cannot satisfy these contracts. An empty separator token in NO_COLOR requires actual Reset foreground/background and NONE modifiers on the corner; it is never replaced by a title token. Other existing surface checks remain in place. Document and native CLI text are not globally constrained to application chrome colors or transliterated.

`semantic_contracts_reject_corrupted_capture_evidence` tests wrong warning styling on the failed reason, hidden actual marker despite forged visibility metadata, wrong reached state/view, wrong sample/target, dimensions, truncated buffers, and invented responsive N/A. Run it in all four modes with the loop above. The independent `theme_modes_tests::environment_modes_are_isolated` regression exercises the compact production status behavior and key handling across all six sizes; it remains separate from adapter expectations.

A successful adapter test/export proves capture integrity, identity and the stated model/content postconditions. It does not establish D1/D2/D3 design compliance, NO_COLOR/ASCII compliance, transport behavior, or completeness of the whole TUI. Read `rendering_finding` and the raw buffers before using samples for readiness decisions.
