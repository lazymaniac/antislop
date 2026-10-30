use super::super::*;
use crate::lean::tui::view::TaskRow;

pub(super) fn add_edges(
    view: &View,
    graph: &mut Graph,
    index: &HashMap<&str, usize>,
    extra_agents: &[&str],
) {
    add_role_edges(graph, extra_agents);
    add_task_edges(view, graph, index);
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

fn add_task_edges(view: &View, graph: &mut Graph, index: &HashMap<&str, usize>) {
    for task in &view.tasks {
        let worker = format!("worker:{}", task.id);
        push_edge(
            graph,
            ORCHESTRATOR.into(),
            worker.clone(),
            Kind::Supervises,
            None,
        );
        add_dependencies(graph, task, &worker, index);
    }
}

fn add_dependencies(graph: &mut Graph, task: &TaskRow, worker: &str, index: &HashMap<&str, usize>) {
    for parent in &task.depends_on {
        if index.contains_key(parent.as_str()) {
            push_edge(
                graph,
                format!("worker:{parent}"),
                worker.to_owned(),
                Kind::Depends,
                Some("D".into()),
            );
        }
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
    });
}
