# Graphical workspace proposal

The current production frontend is a Ratatui terminal application. The user requested a more modern appearance and previously specified a MindNode/Neo4j-style agent graph, a corner overlay for the current step and comments on every visible agent-panel element. A graphical workspace is being designed as a new frontend; this document does not claim it has been implemented.

## Layout

A compact navigation rail and top bar frame a conversation column, dominant spatial graph and collapsible inspector. Agent cards show role, provider/model, observed status and task. Typed curved edges distinguish supervision, verification, dependencies and communication. Search, pan, zoom and fit view make larger teams usable. The corner overlay derives its step from runtime observations, including provider discovery and checks. Status has a text label in addition to color.

The inspector exposes overview, evidence and anchored comments. Comment counts appear on eligible graph elements. An orchestrator reaction is distinct from a queued comment. Historical comments remain bound to their observed element version. Artifact, requirement, task and diff views reuse the same comment interaction.

The primary composer starts an interview from a user request with a visible Send control and Ctrl+S. The orchestrator decides the team and configurations from predefined roles and prompts. Reviewed contract/team cards expose the actual scope. Plan installation and task execution remain separate actions.

## Runtime integration boundary

Reuse the existing daemon and the semantics of `src/tui/backend.rs`: `BeginConversation`, `ChatMessage`, `AddElementComment`, `ResolveElementComment`, `AcceptIntake`, `AcceptTeam`, `StepTeam`, `Pause` and `Resume`. Reuse the observations projected by `src/tui/model.rs`. A design preview must not fabricate live session receipts or applied orchestrator reactions. The transport and graphical toolkit are implementation decisions to resolve after the UI target/direction is selected.

## Review criteria

- A real two-dimensional navigable graph, not an indented agent list.
- Clear distinction between selected, queued, active, failed and completed states.
- Composer and conversation are usable without creating a specification file.
- Current step remains visible without obscuring selected graph nodes.
- Every eligible visible element offers comments and shows actual reaction state.
- Visible keyboard focus, readable contrast, reduced motion and narrow-window layouts.
- The existing terminal frontend remains available during graphical integration.

Visual tokens and resumable canvas context live under `.superdesign/`. The production application has not yet changed as part of this appearance proposal.

## Design review artifact

Superdesign draft `320531d2-e19e-4473-94b0-68db84ccbd67`, version 2: [preview](https://p.superdesign.dev/draft/320531d2-e19e-4473-94b0-68db84ccbd67), [canvas](https://superdesign.dev/teams/62bb3487-2302-4051-9846-d65026008966/projects/96e20f25-f411-488b-ac8e-8c0f98a67617). Includes prototype node selection, graph zoom/pan/fit, inspector tabs, mock composer and comments, and plan review. Source and JavaScript syntax were checked; rendered visual/interaction verification was unavailable because this session exposes no browser. These interactions are a design preview, not daemon integration. The graphical versus terminal choice and visual approval are still pending.

## Terminal frontend refresh

The Ratatui frontend now follows the same direction without leaving the terminal. Borders remain only as single rules between the conversation, agent graph and task list. Messages, agents and tasks are cards on a raised background with a status-colored accent bar (`src/tui/theme.rs`). Graph edges merge into junctions, use rounded corners, and are colored and lettered by relation kind (V verifies, D depends, M messages); same-column relations route through side lanes. The graph viewport scrolls only when the graph does not fit.

Tasks are a checklist overlay in the graph's lower-right corner: `[✓]` done, `[●]` in progress, `[‖]` waiting, `[✕]` failed and `[ ]` to do, with a summary count and the pinned current step. Long lists turn like a carousel: the followed task stays centered and neighbouring tasks fade with distance. The overlay is a third focus target (Tab / Shift+Tab). Unfocused, it follows the first observed active task; focused, Up/Down, PageUp/PageDown and Home/End turn it, and the graph selects the task owner without hiding it under the overlay. NO_COLOR and ARCHITECT_ASCII remain supported.

`tui-nodes` 0.10 was evaluated for the graph and not adopted: it always draws node borders, has no viewport or panning, places roots on the right, panics on cyclic relations and routes every edge with a search over the whole canvas, which does not hold for the 5000-agent regression test.
