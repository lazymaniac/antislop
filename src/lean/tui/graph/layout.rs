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
        builder::Builder::new(view, scope, collapsed, grouping, viewport).build()
    }
}
