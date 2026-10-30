use super::super::*;

impl App {
    pub(in crate::lean::tui) fn scroll_graph_node(
        &mut self,
        graph: &Graph,
        down: bool,
        amount: u16,
    ) {
        if let Some(node) = graph.node(&self.selected) {
            let height = self.graph_viewport.map_or(10, |(_, height)| height);
            let max = node
                .y
                .saturating_add(node.height)
                .saturating_sub(height)
                .max(node.y);
            self.offset.1 = if down {
                self.offset.1.saturating_add(amount).clamp(node.y, max)
            } else {
                self.offset.1.saturating_sub(amount).clamp(node.y, max)
            };
        }
    }

    pub fn select_node(&mut self, key: String) {
        self.selected = graph::task_node_key(&key);
    }

    pub fn toggle_selected_collapse(&mut self) {
        let task_id = if let Some((_, id)) = self.selected.split_once(':') {
            id.to_string()
        } else {
            return;
        };
        if self.graph_collapsed.contains(&task_id) {
            self.graph_collapsed.remove(&task_id);
        } else {
            self.graph_collapsed.insert(task_id);
        }
    }

    pub fn set_graph_filter(&mut self, query: Option<String>) {
        self.graph_filter = query;
        let graph = self.current_graph();
        if !graph
            .nodes
            .iter()
            .any(|node| node.visible && node.key == self.selected)
        {
            self.selected = self.visible_selection(&graph);
        }
    }

    fn current_graph(&self) -> Graph {
        Graph::new_with_viewport(
            &self.view,
            self.graph_filter.as_deref(),
            &self.graph_collapsed,
            self.graph_grouping,
            self.graph_viewport,
        )
    }

    fn visible_selection(&self, graph: &Graph) -> String {
        if let Some((_, id)) = self.selected.split_once(':') {
            let worker = format!("worker:{id}");
            if graph.node(&worker).is_some_and(|node| node.visible) {
                return worker;
            }
        }
        graph
            .nodes
            .iter()
            .find(|node| node.visible && node.key == ORCHESTRATOR)
            .or_else(|| graph.nodes.iter().find(|node| node.visible))
            .map_or_else(|| ORCHESTRATOR.into(), |node| node.key.clone())
    }
}
