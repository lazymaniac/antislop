# TUI render capture adapter

The test-only adapter in `lean::tui::capture` invokes production `App::draw` on a fresh Ratatui `Terminal<TestBackend>`. It exports every width-by-height cell with coordinates, symbol, foreground/background, and modifiers. Each sample carries a target, state, and geometry identifier. The existing six baseline geometries remain: `minimum-12x5`, `compact-40x12`, `standard-80x24`, `wide-100x30`, `large-120x40`, and `large-160x50`.

The section 2 map keeps the 22 logical catalog headings and links them to named fixtures. Graph maps ordinary, empty, and compact renders; the Workbench shell maps session, agent, and native PTY ordinary/error/no-history states; its six tabs map to their concrete tab fixtures; plan history maps loading/empty/error; composer maps empty/waiting/error and draft modes; the remaining entries map their modal, editor, picker, and recovery fixtures. `manifest.json` includes this map, fixture state names, geometries and semantic N/A explanations. A catalog cell is not declared N/A merely because a fixture is missing. N/A is used only for actual responsive unavailability: the 12x5 minimum guard does not promise full surface styling/content, modal-only surfaces need 40 columns, PTY/session composer panels need 24 rows, and `graph-compact` applies at 40x12 only.

The readiness r10/g8 inventory reports 792 catalog combinations (198 captured, 30 N/A, 564 missing). Those figures are a diagnostic baseline, not a requirement to add 564 files; this adapter adds applicable state and boundary evidence while stating why boundary families do not apply to other targets. The existing readiness outputs are preserved.

The matrix adds both sides of, and the exact point at, these boundaries: graph inspector content 110 columns and 16 rows; Workbench list 100 columns and 24 rows; split terminal 24 rows; and the loaded Diff file list 90 columns. Graph terminal geometries are wider/taller than the target graph area because the production main screen reserves chat and footer space; the named samples are validated by whether the inspector actually renders. Workbench records the post-draw `workbench_areas`, including actual content dimensions and list/split visibility. `workbench-tab-diff` loads a deterministic `FileDiff` fixture and checks its rendered filename plus counts, so the 90-column assertion covers the real content panel behavior.

Run the focused test, or select only requested fixture/geometry pairs with `ANTISLOP_CAPTURE_SELECT`. Keys use `fixture-id@geometry-id`, comma separated; a key that is unknown, inapplicable, or repeated fails rather than silently dropping a request.

```sh
./cargo.sh test --lib lean::tui::capture::fixture_contracts_reach_the_requested_views_and_export_full_buffers -- --exact --nocapture
ANTISLOP_CAPTURE_SELECT='graph-ordinary@inspector-at-width,workbench-summary@list-at-width,native-pty@split-at-height,workbench-tab-diff@diff-at-width' \
ANTISLOP_CAPTURE_DIR=/absolute/path/to/new-run \
./cargo.sh test --lib lean::tui::capture::export_catalog -- --exact --ignored --test-threads=1
```

Without a selector, `export_catalog` writes the full registered sample plan. With one, it exports only those samples, so a readiness consumer can rerun selected missing combinations without replaying the complete history. The export refuses an existing directory. Store each generation in a new directory outside the checkout; earlier reference renders remain untouched. `final-renders.json` keeps format `antislop-ratatui-testbackend-cells-v1`. Each record includes target/view/state/geometry, `na_reason`, full cells, and measured panel signals. `manifest.json` lists geometry identifiers and dimensions, fixture registration, section 2 mapping, and N/A reasons. Cell styles use Ratatui debug names to preserve style distinctions without coupling the export format to internal enum layouts. PTY fixtures use deterministic terminal snapshots and never start a CLI provider.
