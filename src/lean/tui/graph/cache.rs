use super::*;

#[derive(Default)]
pub struct LayoutCache {
    pub(super) entry: Option<CachedGraph>,
}

pub(super) struct CachedGraph {
    generation: u64,
    revision: u64,
    counts: (usize, usize, usize),
    idle: bool,
    viewport: Option<(u16, u16)>,
    pub(super) expires: Option<std::time::Instant>,
    graph: std::rc::Rc<Graph>,
}

struct CacheRequest<'a> {
    view: &'a View,
    generation: u64,
    scope: &'a Scope,
    collapsed: &'a std::collections::HashSet<String>,
    grouping: GroupingMode,
    viewport: Option<(u16, u16)>,
}

impl LayoutCache {
    pub fn get(
        &mut self,
        view: &View,
        generation: u64,
        scope: &Scope,
        collapsed: &std::collections::HashSet<String>,
        grouping: GroupingMode,
        viewport: Option<(u16, u16)>,
    ) -> std::rc::Rc<Graph> {
        let request = CacheRequest {
            view,
            generation,
            scope,
            collapsed,
            grouping,
            viewport,
        };
        let now = std::time::Instant::now();
        if self.matches(&request, now) {
            return std::rc::Rc::clone(&self.entry.as_ref().expect("matched cache entry").graph);
        }
        self.rebuild(request, now)
    }

    fn matches(&self, request: &CacheRequest<'_>, now: std::time::Instant) -> bool {
        self.entry.as_ref().is_some_and(|entry| {
            let view = request.view;
            entry.generation == request.generation
                && entry.revision == view.layout_revision
                && entry.counts == counts(view)
                && entry.idle == view.idle
                && entry.viewport == request.viewport
                && &entry.graph.scope == request.scope
                && &entry.graph.collapsed == request.collapsed
                && entry.graph.grouping == request.grouping
                && entry.expires.is_none_or(|expiry| now < expiry)
        })
    }

    fn rebuild(
        &mut self,
        request: CacheRequest<'_>,
        now: std::time::Instant,
    ) -> std::rc::Rc<Graph> {
        let view = request.view;
        let graph = std::rc::Rc::new(Graph::new_with_viewport(
            view,
            request.scope,
            request.collapsed,
            request.grouping,
            request.viewport,
        ));
        let expires = view
            .links
            .values()
            .filter_map(|link| link.last.checked_add(RECENT))
            .filter(|expiry| *expiry > now)
            .min();
        self.entry = Some(CachedGraph {
            generation: request.generation,
            revision: view.layout_revision,
            counts: counts(view),
            idle: view.idle,
            viewport: request.viewport,
            expires,
            graph: std::rc::Rc::clone(&graph),
        });
        graph
    }
}

fn counts(view: &View) -> (usize, usize, usize) {
    (view.tasks.len(), view.agents.len(), view.links.len())
}
