use super::super::*;

impl App {
    pub(in crate::lean::tui) fn open_graph_filters(&mut self) {
        self.graph_filters = Some(Default::default());
    }

    /// Moves the view over the whole graph by `amount` rows, or columns
    /// when `sideways`; the selection stays where it is.
    pub(in crate::lean::tui) fn pan_graph(
        &mut self,
        graph: &Graph,
        forward: bool,
        amount: u16,
        sideways: bool,
    ) {
        let (width, height) = self.graph_viewport.unwrap_or((40, 10));
        let (offset, max) = if sideways {
            (&mut self.offset.0, graph.width.saturating_sub(width))
        } else {
            (&mut self.offset.1, graph.height.saturating_sub(height))
        };
        *offset = if forward {
            offset.saturating_add(amount)
        } else {
            offset.saturating_sub(amount)
        }
        .min(max);
    }

    pub fn select_node(&mut self, key: String) {
        self.selected = graph::card_key(&self.view, &key);
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
        self.graph_scope.query = query;
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
            &self.graph_scope,
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
