use super::super::*;

impl App {
    pub(in crate::lean::tui) fn graph_scope_with_exception(&self) -> graph::Scope {
        let mut scope = self.graph_scope.clone();
        scope.visibility_exception = self
            .graph_visibility_exception
            .as_ref()
            .filter(|key| **key == self.selected)
            .cloned();
        scope
    }

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
        let work = super::super::work_units::WorkUnits::new(&self.view);
        self.navigation.work_selection =
            work.for_stage(&key)
                .map(|unit| super::super::work_units::WorkSelection {
                    unit: unit.id.clone(),
                    stage: None,
                });
        self.selected = work.card_key(&key);
        self.graph_visibility_exception = None;
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
        self.graph_visibility_exception = None;
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

impl App {
    pub(in crate::lean::tui) fn selected_work_stage(&self) -> Option<String> {
        let work = super::super::work_units::WorkUnits::new(&self.view);
        let unit = work.for_stage(&self.selected)?;
        let selected = self
            .navigation
            .work_selection
            .as_ref()
            .filter(|selection| selection.unit == unit.id)
            .and_then(|selection| selection.stage.as_ref());
        Some(
            selected
                .filter(|id| unit.stages.iter().any(|stage| stage.id == **id))
                .map_or_else(
                    || unit.active_stage(&self.view).id.0.clone(),
                    |id| id.0.clone(),
                ),
        )
    }

    pub(in crate::lean::tui) fn cycle_work_stage(&mut self, backwards: bool) {
        let work = super::super::work_units::WorkUnits::new(&self.view);
        let Some(unit) = work.for_stage(&self.selected) else {
            return;
        };
        let current = self
            .navigation
            .work_selection
            .as_ref()
            .filter(|selection| selection.unit == unit.id)
            .and_then(|selection| selection.stage.as_ref());
        let next = current
            .and_then(|id| unit.stages.iter().position(|stage| stage.id == *id))
            .map_or(if backwards { unit.stages.len() - 1 } else { 0 }, |index| {
                if backwards {
                    (index + unit.stages.len() - 1) % unit.stages.len()
                } else {
                    (index + 1) % unit.stages.len()
                }
            });
        self.select_work_stage(unit.stages[next].id.0.clone());
    }

    pub(in crate::lean::tui) fn select_work_stage(&mut self, key: String) {
        use super::super::work_units::{StageId, WorkSelection, WorkUnits};
        let work = WorkUnits::new(&self.view);
        if let Some(unit) = work.for_stage(&key) {
            self.selected = unit.card.clone();
            let stage = StageId(key);
            self.navigation.work_selection = Some(WorkSelection {
                unit: unit.id.clone(),
                stage: Some(stage),
            });
        }
    }
}

impl App {
    pub(in crate::lean::tui) fn open_work_card_or_stage(
        &mut self,
        row: &super::super::workbench::SidebarRow,
    ) {
        let target = if row.stage || row.key == "conversation" {
            row.key.clone()
        } else {
            self.select_node(row.key.clone());
            self.selected_work_stage()
                .unwrap_or_else(|| row.key.clone())
        };
        self.open_details(target);
    }
}

impl App {
    pub(in crate::lean::tui) fn activate_inspector(
        &mut self,
        action: super::super::work_units::InspectorAction,
    ) {
        use super::super::work_units::InspectorAction;
        match action {
            InspectorAction::Stage(key) => self.select_work_stage(key),
            InspectorAction::Decisions => self.open_decision_inbox(),
            InspectorAction::Results(key) => {
                self.open_details(key);
                self.workbench_select_tab(WorkbenchTab::Results);
            }
            InspectorAction::Diff(key) => self.open_diff(&key),
            InspectorAction::Recovery(key) => self.open_unblock(Some(key)),
        }
    }
}
