use super::super::*;

pub(super) fn center_and_stack(graph: &mut Graph, size: Size, viewport: Option<(u16, u16)>) {
    center_horizontally(graph, viewport);
    stack_visible_rows(graph, size);
}

fn center_horizontally(graph: &mut Graph, viewport: Option<(u16, u16)>) {
    if let Some((vp_w, _)) = viewport {
        let visible = || graph.nodes.iter().filter(|node| node.visible);
        let left = visible().map(|node| node.x).min().unwrap_or(0);
        let right = visible().map(Node::right).max().unwrap_or(0);
        let shift = (vp_w.saturating_sub(right - left) / 2)
            .saturating_sub(left)
            .min(vp_w.saturating_sub(right + GAP_X));
        for node in graph.nodes.iter_mut().filter(|node| node.visible) {
            node.x += shift;
        }
    }
}

fn stack_visible_rows(graph: &mut Graph, size: Size) {
    let mut rows = std::collections::BTreeSet::<u16>::new();
    for node in graph.nodes.iter().filter(|node| node.visible) {
        rows.insert(node.y);
    }
    let stack: HashMap<_, _> = rows
        .into_iter()
        .enumerate()
        .map(|(row, y)| (y, row_y(size, row as u16)))
        .collect();
    for node in graph.nodes.iter_mut().filter(|node| node.visible) {
        node.y = stack[&node.y];
    }
}
