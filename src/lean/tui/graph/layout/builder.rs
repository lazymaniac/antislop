use super::super::*;
use super::{columns, edges, geometry, placement};

pub(super) struct Builder<'a> {
    pub(super) view: &'a View,
    pub(super) graph: Graph,
    pub(super) size: Size,
    pub(super) index: HashMap<&'a str, usize>,
    pub(super) columns: Vec<u16>,
    pub(super) admin: Vec<(&'a str, bool)>,
    pub(super) extra_agents: Vec<&'a str>,
    pub(super) viewport: Option<(u16, u16)>,
    pub(super) placed_tasks: std::collections::HashSet<usize>,
}

impl<'a> Builder<'a> {
    pub(super) fn new(
        view: &'a View,
        scope: &Scope,
        collapsed: &std::collections::HashSet<String>,
        grouping: GroupingMode,
        viewport: Option<(u16, u16)>,
    ) -> Self {
        let size = Size::fit(viewport);
        let graph = empty_graph(scope, collapsed, grouping);
        let index = task_index(view);
        let columns = columns::task_columns(view, &index, grouping);
        let extra_agents = extra_agents(view);
        let admin = admin_agents(view, &extra_agents, &graph);
        Self {
            view,
            graph,
            size,
            index,
            columns,
            admin,
            extra_agents,
            viewport,
            placed_tasks: std::collections::HashSet::new(),
        }
    }

    pub(super) fn build(mut self) -> Graph {
        let visible = placement::visible_tasks(&self);
        placement::place_nodes(&mut self, visible);
        placement::append_hidden_tasks(&mut self);
        geometry::center_and_stack(&mut self.graph, self.size, self.viewport);
        edges::add_edges(self.view, &mut self.graph, &self.index, &self.extra_agents);
        set_graph_bounds(&mut self.graph);
        self.graph
    }

    pub(super) fn matches_query(&self, key: &str) -> bool {
        scope_matches(self.view, &self.graph.scope, key)
    }
}

fn empty_graph(
    scope: &Scope,
    collapsed: &std::collections::HashSet<String>,
    grouping: GroupingMode,
) -> Graph {
    Graph {
        nodes: Vec::new(),
        edges: Vec::new(),
        width: 0,
        height: 0,
        scope: scope.clone(),
        collapsed: collapsed.clone(),
        grouping,
    }
}

fn task_index(view: &View) -> HashMap<&str, usize> {
    view.tasks
        .iter()
        .enumerate()
        .map(|(index, task)| (task.id.as_str(), index))
        .collect()
}

fn extra_agents(view: &View) -> Vec<&str> {
    let first_round = REQUIREMENTS_ROUNDS
        .into_iter()
        .find(|role| view.agents.contains_key(*role));
    let mut agents: Vec<_> = view
        .agents
        .keys()
        .map(String::as_str)
        .filter(|key| {
            !matches!(*key, PLAN_VERIFIER | ORCHESTRATOR | AUDITOR)
                && !key.starts_with("worker:")
                && !key.starts_with("verifier:")
        })
        .filter(|key| requirements_round(key).is_none() || Some(*key) == first_round)
        .collect();
    agents.sort_unstable();
    agents
}

fn admin_agents<'a>(view: &View, extra: &[&'a str], builder: &Graph) -> Vec<(&'a str, bool)> {
    let visible = |key| scope_matches(view, &builder.scope, key);
    let mut agents = vec![
        (PLAN_VERIFIER, visible(PLAN_VERIFIER)),
        (ORCHESTRATOR, visible(ORCHESTRATOR)),
        (AUDITOR, visible(AUDITOR)),
    ];
    agents.extend(extra.iter().map(|&key| (key, visible(key))));
    agents
}

fn task_matches(view: &View, key: &str, query: &str) -> bool {
    let Some((_, id)) = key.split_once(':') else {
        return false;
    };
    let Some(task) = view.tasks.iter().find(|task| task.id == id) else {
        return false;
    };
    task_fields_match(task, query)
        || task
            .summary
            .as_ref()
            .is_some_and(|text| text.to_lowercase().contains(query))
        || task
            .verifier_summary
            .as_ref()
            .is_some_and(|text| text.to_lowercase().contains(query))
        || task
            .effect
            .as_ref()
            .is_some_and(|text| text.to_lowercase().contains(query))
}

fn task_fields_match(task: &super::super::super::view::TaskRow, query: &str) -> bool {
    [
        task.title.to_lowercase(),
        task.status.to_lowercase(),
        taskinfo::worker_status(task).to_lowercase(),
        taskinfo::verifier_status(task).to_lowercase(),
        task.goal.to_lowercase(),
        task.focus.to_lowercase(),
    ]
    .iter()
    .any(|text| text.contains(query))
}

/// Whether a card passes every part of the scope.
fn scope_matches(view: &View, scope: &Scope, key: &str) -> bool {
    query_matches(view, scope.query.as_deref(), key)
        && (scope.states.is_empty() || scope.states.contains(&health(view, key)))
        && (scope.providers.is_empty()
            || super::super::health::providers_of(view, key)
                .iter()
                .any(|provider| scope.providers.contains(*provider)))
}

fn query_matches(view: &View, query: Option<&str>, key: &str) -> bool {
    let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) else {
        return true;
    };
    let query = query.to_lowercase();
    key.to_lowercase().contains(&query)
        || node_name(key).to_lowercase().contains(&query)
        || task_matches(view, key, &query)
}

fn set_graph_bounds(graph: &mut Graph) {
    graph.width = graph
        .nodes
        .iter()
        .filter(|node| node.visible)
        .map(Node::right)
        .max()
        .unwrap_or(0)
        + GAP_X;
    graph.height = graph
        .nodes
        .iter()
        .filter(|node| node.visible)
        .map(|node| node.y.saturating_add(node.height))
        .max()
        .unwrap_or(0);
}
