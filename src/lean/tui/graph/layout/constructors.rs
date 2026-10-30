use super::super::*;

impl Graph {
    pub fn new(view: &View) -> Self {
        Self::new_with_options(
            view,
            None,
            &std::collections::HashSet::new(),
            GroupingMode::Dependency,
        )
    }

    pub fn new_with_options(
        view: &View,
        filter_query: Option<&str>,
        collapsed: &std::collections::HashSet<String>,
        grouping: GroupingMode,
    ) -> Self {
        let scope = Scope {
            query: filter_query.map(str::to_owned),
            ..Scope::default()
        };
        Self::new_with_viewport(view, &scope, collapsed, grouping, None)
    }
}
