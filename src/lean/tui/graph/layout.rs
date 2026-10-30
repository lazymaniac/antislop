use super::*;
mod builder;
mod columns;
mod constructors;
mod edges;
mod geometry;
mod placement;

impl Graph {
    pub fn new_with_viewport(
        view: &View,
        filter_query: Option<&str>,
        collapsed: &std::collections::HashSet<String>,
        grouping: GroupingMode,
        viewport: Option<(u16, u16)>,
    ) -> Self {
        builder::Builder::new(view, filter_query, collapsed, grouping, viewport).build()
    }
}
