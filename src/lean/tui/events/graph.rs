use super::super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl App {
    pub(in crate::lean::tui) fn graph_key(&mut self, key: KeyEvent) -> bool {
        self.selected = graph::card_key(&self.view, &self.selected);
        let graph = self.graph_for_key();
        if let Some(handled) = self.graph_scroll_key(key, &graph) {
            return handled;
        }
        if let Some((dx, dy)) = graph_direction(key.code) {
            self.move_graph_selection(&graph, dx, dy);
            return true;
        }
        self.graph_action_key(key)
    }

    fn graph_for_key(&self) -> Graph {
        Graph::new_with_viewport(
            &self.view,
            &self.graph_scope,
            &self.graph_collapsed,
            self.graph_grouping,
            self.graph_viewport,
        )
    }

    fn graph_scroll_key(&mut self, key: KeyEvent, graph: &Graph) -> Option<bool> {
        let down = match key.code {
            KeyCode::PageDown => Some(true),
            KeyCode::PageUp => Some(false),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(true),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(false),
            _ => None,
        }?;
        let height = self.graph_viewport.map_or(10, |(_, height)| height);
        let amount = if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
            height.saturating_sub(2).max(1)
        } else {
            (height / 2).max(1)
        };
        self.pan_graph(graph, down, amount, false);
        Some(true)
    }

    fn move_graph_selection(&mut self, graph: &Graph, dx: i32, dy: i32) {
        if !graph.node(&self.selected).is_some_and(|node| node.visible) {
            self.select_visible_graph_node(graph);
        } else if let Some(next) = graph.neighbour(&self.selected, dx, dy) {
            self.selected = next;
        }
    }

    fn select_visible_graph_node(&mut self, graph: &Graph) {
        let worker = self
            .selected
            .split_once(':')
            .map(|(_, id)| format!("worker:{id}"));
        self.selected = worker
            .and_then(|key| visible_graph_key(graph, &key))
            .or_else(|| visible_graph_key(graph, ORCHESTRATOR))
            .or_else(|| {
                graph
                    .nodes
                    .iter()
                    .find(|node| node.visible)
                    .map(|node| node.key.clone())
            })
            .unwrap_or_else(|| ORCHESTRATOR.into());
    }

    fn graph_action_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => self.focus = Focus::Chat,
            KeyCode::Char('q') => return false,
            KeyCode::Char('?') => self.show_help(Help::Graph),
            KeyCode::Char('/') => self.picker = Some(Picker::new(Kind::Agents)),
            KeyCode::Char('p') => self.run_workflow(Command::Plan),
            KeyCode::Char('u') => self.open_unblock(None),
            KeyCode::Char('z' | ' ') => self.toggle_selected_collapse(),
            KeyCode::Char('g') => self.graph_grouping = self.graph_grouping.next(),
            KeyCode::Char('f') => self.open_graph_filters(),
            KeyCode::Char(digit @ '1'..='4') => {
                self.relations.toggle(usize::from(digit as u8 - b'1'));
            }
            KeyCode::Char('d') => self.open_diff(&self.selected.clone()),
            KeyCode::Char('c') => self.open_graph_comment(),
            KeyCode::Enter => self.open_details(self.selected.clone()),
            _ => {}
        }
        true
    }

    fn open_graph_comment(&mut self) {
        let target = self.selected.clone();
        let input = self.get_comment_draft(&target);
        self.editor = Some(Editor { target, input });
    }
}

fn graph_direction(code: KeyCode) -> Option<(i32, i32)> {
    match code {
        KeyCode::Left => Some((-1, 0)),
        KeyCode::Right => Some((1, 0)),
        KeyCode::Up => Some((0, -1)),
        KeyCode::Down => Some((0, 1)),
        _ => None,
    }
}

fn visible_graph_key(graph: &Graph, key: &str) -> Option<String> {
    graph
        .node(key)
        .filter(|node| node.visible)
        .map(|node| node.key.clone())
}
