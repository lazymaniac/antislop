use super::super::*;
use super::builder::Builder;
use std::collections::{HashMap, HashSet};

pub(super) fn visible_tasks(builder: &Builder<'_>) -> Vec<(usize, u16)> {
    builder
        .view
        .tasks
        .iter()
        .enumerate()
        .filter_map(|(index, task)| {
            let worker = builder.matches_query(&format!("worker:{}", task.id));
            let verifier = builder.matches_query(&format!("verifier:{}", task.id));
            (worker || verifier).then_some((index, builder.columns[index]))
        })
        .collect()
}

pub(super) fn place_nodes(builder: &mut Builder<'_>, visible: Vec<(usize, u16)>) {
    let (depths, mut tasks) = by_relative_depth(visible);
    let mut placed = HashSet::new();
    match builder.viewport {
        Some((width, _)) => flow_nodes(builder, width, &depths, &mut tasks, &mut placed),
        None => column_nodes(builder, &depths, &mut tasks, &mut placed),
    }
    builder.placed_tasks = placed;
}

fn by_relative_depth(visible: Vec<(usize, u16)>) -> (Vec<u16>, HashMap<u16, Vec<usize>>) {
    let depths: Vec<u16> = visible
        .iter()
        .map(|(_, col)| *col)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut grouped = HashMap::<u16, Vec<usize>>::new();
    for (index, raw) in visible {
        let relative = depths.iter().position(|&col| col == raw).unwrap_or(0) as u16;
        grouped.entry(relative).or_default().push(index);
    }
    let mut sorted = grouped.keys().copied().collect::<Vec<_>>();
    sorted.sort_unstable();
    (sorted, grouped)
}

fn flow_nodes(
    builder: &mut Builder<'_>,
    width: u16,
    depths: &[u16],
    grouped: &mut HashMap<u16, Vec<usize>>,
    placed: &mut HashSet<usize>,
) {
    let max_cols = ((width.saturating_sub(MARGIN) + GAP_X) / (builder.size.w + GAP_X)).max(1);
    let admin_rows = flow_admin(builder, max_cols);
    let ordered = ordered_tasks(depths, grouped);
    for (slot, task_index) in ordered.into_iter().enumerate() {
        placed.insert(task_index);
        let column = slot as u16 % max_cols;
        let row = admin_rows + slot as u16 / max_cols;
        push_task(builder, task_index, column, row);
    }
}

fn flow_admin(builder: &mut Builder<'_>, max_cols: u16) -> u16 {
    if usize::from(max_cols) >= builder.admin.len() {
        let admin = builder.admin.clone();
        for (slot, (key, visible)) in admin.into_iter().enumerate() {
            push_admin(builder, key, slot as u16, 0, visible);
        }
        return u16::from(builder.admin.iter().any(|(_, visible)| *visible));
    }
    let mut shown = 0u16;
    let admin = builder.admin.clone();
    for (key, visible) in admin {
        if visible {
            push_admin(builder, key, shown % max_cols, shown / max_cols, true);
            shown += 1;
        } else {
            push_admin(builder, key, 0, 0, false);
        }
    }
    shown.div_ceil(max_cols)
}

fn column_nodes(
    builder: &mut Builder<'_>,
    depths: &[u16],
    grouped: &mut HashMap<u16, Vec<usize>>,
    placed: &mut HashSet<usize>,
) {
    let admin = builder.admin.clone();
    for (slot, (key, visible)) in admin.iter().enumerate() {
        push_admin(builder, key, slot as u16, 0, *visible);
    }
    for &depth in depths {
        let mut tasks = grouped.get(&depth).cloned().unwrap_or_default();
        tasks.sort_unstable();
        let first_row = u16::from(builder.admin.iter().any(|(_, visible)| *visible));
        for (row, task_index) in (first_row..).zip(tasks) {
            placed.insert(task_index);
            push_task(builder, task_index, depth, row);
        }
    }
}

fn ordered_tasks(depths: &[u16], grouped: &mut HashMap<u16, Vec<usize>>) -> Vec<usize> {
    let mut ordered = Vec::new();
    for depth in depths {
        let tasks = grouped.get_mut(depth).expect("known depth");
        tasks.sort_unstable();
        ordered.extend(tasks.iter().copied());
    }
    ordered
}

fn push_admin(builder: &mut Builder<'_>, key: &str, column: u16, row: u16, visible: bool) {
    builder.graph.nodes.push(Node {
        key: key.to_owned(),
        x: if visible {
            column_x(builder.size, column)
        } else {
            0
        },
        y: if visible { row_y(builder.size, row) } else { 0 },
        width: builder.size.w,
        height: ADMIN_HEIGHT.min(builder.size.h),
        visible,
        group: None,
        collapsed: false,
    });
}

fn push_task(builder: &mut Builder<'_>, index: usize, column: u16, row: u16) {
    let id = &builder.view.tasks[index].id;
    let mut node = builder.size.node(
        format!("worker:{id}"),
        column_x(builder.size, column),
        row_y(builder.size, row),
    );
    node.collapsed = builder.graph.collapsed.contains(id)
        || builder.graph.collapsed.contains(&format!("task:{id}"));
    node.group = Some(format!("task:{id}"));
    node.visible = builder.matches_query(&format!("worker:{id}"))
        || builder.matches_query(&format!("verifier:{id}"));
    builder.graph.nodes.push(node);
}

pub(super) fn append_hidden_tasks(builder: &mut Builder<'_>) {
    for (index, task) in builder.view.tasks.iter().enumerate() {
        if !builder.placed_tasks.contains(&index) {
            let collapsed = builder.graph.collapsed.contains(&task.id)
                || builder
                    .graph
                    .collapsed
                    .contains(&format!("task:{}", task.id));
            builder.graph.nodes.push(Node {
                key: format!("worker:{}", task.id),
                x: 0,
                y: 0,
                width: builder.size.w,
                height: builder.size.h,
                visible: false,
                group: Some(format!("task:{}", task.id)),
                collapsed,
            });
        }
    }
}
