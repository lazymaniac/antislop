mod card;

use super::*;

pub(super) struct DrawDetails<'a> {
    pub(super) expanded_tab: Option<usize>,
    pub(super) critical_path: &'a std::collections::HashSet<String>,
}

#[cfg(test)]
pub(super) fn draw_node(
    view: &View,
    canvas: &mut Canvas,
    node: &Node,
    selected: bool,
    expanded_tab: Option<usize>,
    theme: &Theme,
    tick: u64,
) {
    let critical_path: std::collections::HashSet<String> =
        view.critical_path_chain().into_iter().collect();
    draw_node_with_critical_path(
        view,
        canvas,
        node,
        selected,
        DrawDetails {
            expanded_tab,
            critical_path: &critical_path,
        },
        theme,
        tick,
    );
}

pub(super) fn draw_node_with_critical_path(
    view: &View,
    canvas: &mut Canvas,
    node: &Node,
    selected: bool,
    details: DrawDetails<'_>,
    theme: &Theme,
    tick: u64,
) {
    card::draw(view, canvas, node, selected, details, theme, tick);
}
