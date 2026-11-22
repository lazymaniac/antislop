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

The design baseline is [Canvas](../work/visualizations/agent-layout-1.html) and
[Workbench with PTY](../work/visualizations/agent-workbench-pty.html). The terminal
translation uses a single font size and maps spacing onto cells. At 160×50:

| Element | Geometry and treatment |
|---|---|
| Outer chrome | Two-column side margins, one-row top margin; Canvas starts below the brand, without a heading or subtitle |
| Canvas | Flat work cards; three-column gap before the padded 36-column inspector |
| Workbench | 30-column task hierarchy, three-column gap, equal content/PTY shares |
| Content | Filled panel with padded title and tabs; no nested diff toolbar or box outlines |
| Native terminal | Raised header, dark `#0a1114` surface, quiet controls, visible attachment errors |
| Direction | Full-width filled invitation; raised composer with dark input and mint Send action |
| Bottom chrome | One quiet keybar; mouse hit regions come from the rendered labels |

The retained PNG design references are [Canvas](../work/visualizations/agent-layout-1.png)
and [Workbench with PTY](../work/visualizations/agent-workbench-pty-uniform-font.png).
They are historical design material, not captures of the current application;
their original generation path and current manual use are unconfirmed. See
[the retention decision](api-and-reference-retention.md) for the evidence and
conditions for reconsidering them.

Application colors match the reference tokens, including selected cards and
diff additions/removals. Native CLI output retains its own colors and text.
The exact 160×50 offsets, pane widths, split and backgrounds have a regression
test. The capture catalog also checks real diff-plus-PTY output, compact layouts,
pending decisions, unavailable history, errors, drafts and monochrome/ASCII modes.

Reviewed 160×50 captures: [Canvas](images/tui-canvas.png),
[Workbench](images/tui-workbench.png), [direction composer](images/tui-direction.png).
These rasterize the real TestBackend cell buffers with a fixed monospace font;
native PTY cells come from deterministic terminal fixtures.

Historical validation of the visual correction on 2026-11-22 with the pinned
Rust toolchain (these totals are not validation of subsequent changes):

- Formatting, `cargo.sh check`, clippy for all targets with warnings denied,
  and `git diff --check`.
- Full library suite: 1,105 passed; seven opt-in tests ignored.
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

### Navigation model for frontend adapters

`NavigationState` owns the current `screen`, `target` and `focus`.
`WorkbenchPresentation.focus` remembers a target's preferred focus;
`NavigationState.background_focus` retains the graph/chat surface behind a
Workbench pane or composer. These values have different lifetimes and conversation
snapshots preserve them without reconciling mirrored fields. Frontend callers that
previously wrote `App.focus` should use `App::set_focus(Focus)` to select a surface;
read `navigation.background_focus` for the retained surface and `navigation.focus`
for the current input focus. `ConversationState.focus` is likewise replaced by its
navigation state. Keeping writable mirrors would reintroduce ambiguous ownership.

Workbench content receives `WorkbenchTab` directly. Scroll/query memory uses
`(target key, WorkbenchTab)`, scoped to the conversation, without reserved integer
slots. `Details.tab`, its numeric switch methods, and standalone rendering remain
the compatibility boundary for older callers. The Workbench adapter ignores that
numeric selection; callers inspecting active Workbench selection should read
`navigation.presentation().tab`. Calling `Details::switch_tab` or standalone
`Details::draw` selects the legacy API again. Modal keyboard, paste and pointer
routing retain their existing precedence; this model does not grant PTY input.
