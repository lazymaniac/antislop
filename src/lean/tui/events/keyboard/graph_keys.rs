use super::*;

impl App {
    pub(super) fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Chat => Focus::Graph,
            Focus::Graph => Focus::Chat,
        };
    }

    pub(super) fn route_graph(&mut self, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('a') => return self.graph_command(Command::Accept),
                KeyCode::Char('x') => return self.graph_command(Command::Cancel),
                KeyCode::Char('r') => return self.graph_command(Command::Resume),
                _ => {}
            }
        }
        self.graph_key(key)
    }

    fn graph_command(&mut self, command: Command) -> bool {
        self.run_workflow(command);
        true
    }
}
