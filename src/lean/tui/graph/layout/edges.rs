use super::super::*;

/// The edges between the session's own agents and the messages; the
/// layout of the units already added the dependencies and the arrows
/// inside each frame.
pub(super) fn add_edges(view: &View, graph: &mut Graph, extra_agents: &[&str]) {
    add_role_edges(graph, extra_agents);
    add_mail_edges(view, graph);
}

fn add_role_edges(graph: &mut Graph, extra_agents: &[&str]) {
    push_edge(
        graph,
        PLAN_VERIFIER.into(),
        ORCHESTRATOR.into(),
        Kind::Verifies,
        Some("V".into()),
    );
    push_edge(
        graph,
        AUDITOR.into(),
        ORCHESTRATOR.into(),
        Kind::Verifies,
        Some("V".into()),
    );
    for &key in extra_agents {
        push_edge(
            graph,
            ORCHESTRATOR.into(),
            key.into(),
            Kind::Supervises,
            None,
        );
    }
}

fn add_mail_edges(view: &View, graph: &mut Graph) {
    for ((from, to), (count, recent)) in aggregate_mail(view) {
        push_edge(
            graph,
            from,
            to,
            Kind::Mail { recent },
            Some(format!("M✉{count}")),
        );
    }
}

fn aggregate_mail(view: &View) -> std::collections::BTreeMap<(String, String), (u32, bool)> {
    let mut links = std::collections::BTreeMap::new();
    for ((from, to), link) in &view.links {
        let pair = (card_key(view, from), card_key(view, to));
        if pair.0 == pair.1 {
            continue;
        }
        let (count, recent) = links.entry(pair).or_insert((0u32, false));
        *count = count.saturating_add(link.count);
        *recent |= link.last.elapsed() < RECENT;
    }
    links
}

fn push_edge(graph: &mut Graph, from: String, to: String, kind: Kind, label: Option<String>) {
    if from == to {
        return;
    }
    graph.edges.push(Edge {
        from,
        to,
        kind,
        label,
        path: None,
    });
}
