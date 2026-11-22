# Grouped work TUI capability map

The Canvas, inspector and Workbench share `tui::work_units::WorkUnits`. Unit IDs
are independent of turn IDs and rounds. Stage IDs retain runtime role addresses.
No database schema or daemon request format changes are needed.

| Capability | Entry point | State owner |
|---|---|---|
| Work ownership, dedicated/shared options | Canvas and Workbench hierarchy | WorkUnits, runtime task metadata |
| Workflow, planning/review cycles, audit | Work cards and nested stages | Original role address and run history |
| Filters/search/grouping/relations/critical path | `/`, f, g, 1–4, inspector | Existing graph scope and DAG layout |
| Fold, pan and reveal selection | z, page keys, wheel/Shift+wheel | Conversation Canvas state |
| Role results, checks, evidence, comments, diff | Stage Workbench tabs; inspector links | Details cache per conversation and role |
| Historical rounds and documents | comma/period, Alt+Left/Right | Per-target presentation and Details |
| Terminal attachment, following, pinning, resizing | t, Ctrl+], p, z, Alt+Up/Down | Native views and per-target presentation |
| Session history and completion actions | Ctrl+T; Summary/Conversation | Original conversation projection |
| Addressed/broadcast messages, queue/steer/interrupt | Ctrl+K, Ctrl+R, Ctrl+T | Independent session/agent drafts and receipt IDs |
| Attachments, clipboard images, recalled history | Session composer paste/drop, Ctrl+V, Up/Down | Existing image and input-history stores |
| Questions, approvals and approval modes | d; inspector; Ctrl+P | Decisions inbox and original approval requests |
| Plan review, accept/reject, revisions, history | Ctrl+L, h, Ctrl+P | Existing plan view/history/revision commands |
| Edit/add/split/hold/retire/link tasks | e/a/S/D/l; Ctrl+P for hold | Existing flow editor and revision commands |
| Direct execution and rigor | `/direct`, `/rigor`, Ctrl+P | Existing workflow commands |
| Recovery and consent | u; inspector; Ctrl+P | Existing recovery and explicit consent dialogs |
| Session/project management, observer takeover | Ctrl+N/O/G; Ctrl+P | Existing conversation/project ownership |
| Configuration, help, search, modal isolation | Ctrl+P, F1, `/` | Existing editors, pickers and modal state |
| Unicode, ASCII, monochrome, small terminals | All surfaces | Shared theme, grapheme editing and viewport canvas |

Canvas Enter restores a remembered valid stage or selects current activity.
Selecting a stage explicitly never changes its underlying role. External mail
routes join work cards; the inspector retains exact internal endpoints.

The visual baseline is [Canvas](../work/visualizations/agent-layout-1.html) and
[Workbench with PTY](../work/visualizations/agent-workbench-pty.html). The terminal
translation uses a single font size and maps spacing onto cells. At 160×50:

| Element | Geometry and treatment |
|---|---|
| Outer chrome | Two-column side margins, one-row top margin; Canvas starts below the brand, without a heading or subtitle |
| Canvas | Flat work cards; three-column gap before the padded 36-column inspector |
| Workbench | 30-column task hierarchy, three-column gap, equal content/PTY shares |
| Content | Filled panel with padded title and tabs; no nested diff toolbar or box outlines |
| Native terminal | Raised header, dark `#0a1114` surface, quiet controls, visible attachment errors |
| Direction | One full-width dock: invitation becomes an inline editor; questions and available actions expand above it |
| Bottom chrome | One quiet keybar; mouse hit regions come from the rendered labels |

The top bar reads `> antislop <session branch>` on the left and the session
status plus total work time on the right, in both Canvas and Workbench. Before
the session creates a branch it uses the project's cached branch; an unavailable
branch is shown as a dash. Long names are clipped on grapheme boundaries before
the reserved status/time area. The clock unions recorded agent-turn intervals,
so concurrent agents count once and idle gaps do not count. Replaying the
existing event log restores the total after reopening or switching sessions;
no database or protocol change is needed. Hours appear when the total reaches
one hour. Workbench terminal owner, round, input mode and pin state remain on
the Workbench title row.

Hologram's timing-helper similarity notice compares `SessionTime::elapsed_ms`
with `WorkUnit::elapsed_ms`. The latter adds member-agent durations for a work
card; the session clock unions overlapping turn intervals, so reusing the card
total would count parallel work more than once. Both reuse the existing clock
formatter. The new projection state is reconstructed from existing events.

Application colors match the reference tokens, including selected cards and
diff additions/removals. Native CLI output retains its own colors and text.
The exact 160×50 offsets, pane widths, split and backgrounds have a regression
test. The capture catalog also checks real diff-plus-PTY output, compact layouts,
pending decisions, unavailable history, errors, drafts and monochrome/ASCII modes.

Reviewed 160×50 captures: [Canvas](images/tui-canvas.png),
[Workbench](images/tui-workbench.png), [direction input](images/tui-direction.png),
[inline question](images/tui-question.png).
The [scope preview](images/tui-scope.png) shows the unfocused confirmation at 80×24.
These rasterize the real TestBackend cell buffers with a fixed monospace font;
native PTY cells come from deterministic terminal fixtures.

The direction editor occupies the bottom dock instead of a centered popup.
Its keybar immediately follows the input surface. The same dock serves session
and agent drafts, recipient/broadcast selection, queue/steer/interrupt, delivery
errors and attachments. Questions expand above it without changing focus;
selecting an answer never submits it. The original Form keyboard handler and
scope cache still own answers and bounded scope scrolling. Custom answers use
the dock input, with their own draft, while Esc restores the direction draft.
Completion buttons call the existing session-action handler; CLI approvals
retain the explicit decision inbox. No new request or automatic approval was
introduced. Responsive footer height is included in Workbench geometry and
separator dragging, so a focused native terminal stays visible when the dock grows.

Scope confirmations show the actual scope summary in the unfocused preview,
before any choice is made. The preview uses the same bounded scope renderer as
the focused question: wheel scrolling reads it without moving the Canvas or
taking keyboard focus, and clicking the scope focuses the answer controls.

The dock review's Hologram API finding records local rectangles, hit targets and
the footer-aware layout adapters. `Form::draw_inline` shares `draw_layout` with
the original modal renderer; its name-similarity notice against diff file-list
rendering is unrelated to that shared question renderer. The original form,
scope cache, and session-action handler are reused rather than duplicated.

Validation completed on 2026-11-22 with the pinned Rust toolchain:

- Formatting, `cargo.sh check`, clippy for all targets with warnings denied,
  and `git diff --check`.
- Full library suite: 1,109 passed; seven opt-in tests ignored.
- `lean_core`: 27 passed; `lean_tui`: 116 passed.
- Messaging: 21 passed; approvals: two; recovery: 20; workflow: three.
- Real daemon sequences: navigation two passed (host clipboard test remains
  opt-in); messaging one; plan consent one; recovery four; all-eight-rigors one.
- The real TestBackend capture catalog covers 160×50, 120×40, 80×24,
  12×5, threshold geometries, ASCII/monochrome, error and unavailable states.
  Color renders were inspected for grouped planning and task stages, inspector,
  native split, diff, and composer. Below-minimum and viewport allocation guards
  are covered by the existing source and integration tests.

`hologram review --json` reports the intentional frontend type/API changes and
three fixture-use notices in `tui/tests.rs`. The delayed-send regression reuses
`test_app_with_client` and the transport/socket functions. Its held acknowledgement
and keyboard/mouse navigation checks are distinct from the existing daemon,
proxy and sequence fixtures; those integration-only helpers cannot be imported
by library tests. No competing daemon fixture or duplicate production helper
was added. Obsolete separate-agent card assertions were consolidated into the
shared ownership, grouped routing, health and stage-preservation tests.

The visual correction's Hologram API finding reflects new render adapters,
palette tokens and mouse rectangles, plus removal of the obsolete toolbar hit
parser. Its name-similarity notices compare `DiffView::paint_content` to daemon
history-content retrieval and `Theme::terminal` to a process terminal predicate;
those have different responsibilities. The diff adapter calls the original
`draw_body`, and capture fixtures reuse `capture_fixture_view`. The recovery
sequence helper was renamed to `click_workflow_control` because it clicks the
rendered keybar and action palette rather than the removed toolbar.
