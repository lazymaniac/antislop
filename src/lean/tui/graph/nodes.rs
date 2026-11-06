mod card;

use super::*;

pub(super) fn draw_node(
    view: &View,
    canvas: &mut Canvas,
    node: &Node,
    selected: bool,
    expanded_tab: Option<usize>,
    theme: &Theme,
    tick: u64,
) {
    card::draw(view, canvas, node, selected, expanded_tab, theme, tick);
}
