# Main terminal workspace
Entry: src/tui/mod.rs
Dependencies:
- src/tui/render.rs
  - src/tui/app.rs (navigation, state)
    - src/tui/editor.rs (input)
    - src/tui/backend.rs (daemon actions, not visual context)
  - src/tui/model.rs (snapshot data)
  - src/tui/graph.rs (spatial nodes/edges)
  - src/tui/sanitize.rs (terminal text sanitization)
- src/ipc.rs (backend transport, not design context)
The graphical workspace has no existing implementation.
