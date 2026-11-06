use super::*;
mod builder;
mod constructors;
mod edges;
mod units;

impl Graph {
    pub fn new_with_viewport(
        view: &View,
        scope: &Scope,
        collapsed: &std::collections::HashSet<String>,
        grouping: GroupingMode,
        viewport: Option<(u16, u16)>,
    ) -> Self {
        Self::new_with_expanded(view, scope, collapsed, grouping, viewport, None, 0)
    }

    pub fn new_with_expanded(
        view: &View,
        scope: &Scope,
        collapsed: &std::collections::HashSet<String>,
        grouping: GroupingMode,
        viewport: Option<(u16, u16)>,
        expanded: Option<String>,
        expanded_tab: usize,
    ) -> Self {
        let mut graph =
            builder::Builder::new(view, scope, collapsed, grouping, viewport, expanded.clone())
                .build();
        graph.expanded = expanded;
        graph.expanded_tab = expanded_tab;
        graph
    }
}
