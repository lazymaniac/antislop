mod card;

use super::*;

pub(super) fn draw_node(
    view: &View,
    canvas: &mut Canvas,
    node: &Node,
    selected: bool,
    theme: &Theme,
    tick: u64,
) {
    card::draw(view, canvas, node, selected, theme, tick);
}
