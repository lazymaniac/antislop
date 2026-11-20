# TUI render capture adapter

The adapter is a test-only module inside `lean::tui`, so fixtures can reach private state without widening the production API. It calls the production `App::draw` on a fresh Ratatui `Terminal<TestBackend>` for every sample. Its `section_2_catalog` manifest maps all 22 logical catalog entries to one or more fixture IDs; the catalog includes the graph, Workbench shell and all six tabs, then the remaining fourteen surfaces.

Run the focused contract test. It checks post-fixture App state for every registry entry (surface and declared state), rendered content and the semantic style token assigned to that surface for every applicable geometry, the dedicated modal/tab/PTY signals, and exact buffer dimensions. Composer fixtures must route through the Workbench Composer focus; agent and recovery fixtures assert their selected agent; picker fixtures assert their exact picker kind and rigor; recovery/help fixtures assert their selected mode. With `NO_COLOR=1`, color expectations reduce to the surface's preserved modifier or reset layout style; the same view and state checks still apply. The 12x5 minimum terminal guard has an explicit N/A reason for full surface styling/content; state remains checked at larger sizes.

```sh
./cargo.sh test --lib lean::tui::capture::fixture_contracts_reach_the_requested_views_and_export_full_buffers -- --exact --nocapture
```

Export a new durable capture run by selecting a new directory outside the checkout:

```sh
ANTISLOP_CAPTURE_DIR=/absolute/path/to/new-run ./cargo.sh test --lib lean::tui::capture::export_catalog -- --exact --ignored --test-threads=1
```

The export refuses an existing directory. `final-renders.json` uses the `antislop-ratatui-testbackend-cells-v1` object format shared with the retained readiness reference and contains one render record for each fixture and each of `12x5`, `40x12`, `80x24`, `100x30`, `120x40`, and `160x50`. Each record identifies fixture, view, state, and name, and contains the full row-major cell buffer with coordinates, text, foreground/background, and modifiers. `manifest.json` lists the fixture catalog, schema, renderer, geometries, exact mapping of the 22 section 2 entries, and justified N/A cases. A non-null `na_reason` explains an intentionally unavailable state/geometry. `native_panel` reports whether the session's native terminal route reached a panel; PTY fixtures use deterministic terminal snapshots and never start a CLI provider.

Report consumers can parse `final-renders.json` and use `(fixture, view, state, width, height)` as the sample key. Cell styles are serialized as Ratatui debug names, preserving style distinctions without coupling the export schema to internal enum layouts. The report producer should retain its reference generation and point its new evidence at this generated file.
