use super::*;
use serde_json::Value;

pub(super) fn on_graph_task(view: &mut View, payload: &Value) {
    if payload["plan"].as_str() != view.plan.as_deref() {
        return;
    }
    let Some(mut row) = graph_task_row(payload) else {
        return;
    };
    row.status = payload["status"].as_str().unwrap_or_default().to_owned();
    row.round = payload["round"].as_u64().unwrap_or(0);
    insert_graph_task(view, row);
    refresh_parallel_tasks(view);
}

fn graph_task_row(payload: &Value) -> Option<TaskRow> {
    let mut projection = View::default();
    projection.apply(&serde_json::json!({"seq":1,"kind":"plan_proposed","payload":{"plan":payload["plan"],"tasks":[payload["spec"]]}}));
    if let Some(mut row) = projection.tasks.pop() {
        row.uuid = payload["id"].as_str().map(str::to_owned);
        Some(row)
    } else {
        None
    }
}

fn insert_graph_task(view: &mut View, row: TaskRow) {
    if let Some(existing) = view.tasks.iter_mut().find(|existing| existing.id == row.id) {
        *existing = row;
    } else {
        if let Some(uuid) = &row.uuid {
            view.task_rows.insert(uuid.clone(), view.tasks.len());
        }
        view.tasks.push(row);
    }
}

fn refresh_parallel_tasks(view: &mut View) {
    let nodes = view
        .tasks
        .iter()
        .map(|row| crate::lean::tasks::PeerNode {
            id: &row.id,
            depends_on: &row.depends_on,
            writes: row.writes,
            scope: &row.scope,
        })
        .collect::<Vec<_>>();
    let peers = crate::lean::tasks::peers_of(&nodes);
    for row in &mut view.tasks {
        row.parallel_with = peers.get(&row.id).cloned().unwrap_or_default();
    }
}

pub(super) fn on_graph_updated(view: &mut View, payload: &Value) {
    if payload["plan"].as_str() != view.plan.as_deref() {
        return;
    }
    apply_graph_order(view, &payload["order"]);
    view.plan_scope_hash = payload["scope_hash"].as_str().map(str::to_owned);
    view.conversation_scope_hash = view.plan_scope_hash.clone();
    report_graph_amendment(view, payload["text"].as_str().unwrap_or_default());
}

fn apply_graph_order(view: &mut View, order: &Value) {
    let Some(order) = order.as_array() else {
        return;
    };
    let positions: HashMap<_, _> = order
        .iter()
        .enumerate()
        .filter_map(|(index, id)| id.as_str().map(|id| (id, index)))
        .collect();
    view.tasks
        .retain(|task| positions.contains_key(task.id.as_str()));
    view.tasks.sort_by_key(|task| {
        positions
            .get(task.id.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
    view.task_rows = view
        .tasks
        .iter()
        .enumerate()
        .filter_map(|(index, task)| task.uuid.as_ref().map(|uuid| (uuid.clone(), index)))
        .collect();
}

fn report_graph_amendment(view: &mut View, description: &str) {
    view.plan_doc
        .push_str(&format!("\n\n## Graph amendment\n{description}\n"));
    view.cards.push(TranscriptCard::System {
        title: "Graph updated".into(),
        meta: String::new(),
        text: description.to_owned(),
        tone: crate::lean::tui::theme::Tone::Active,
    });
    view.push(dim(), format!("Graph updated: {description}"));
}
