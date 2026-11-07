use super::super::super::*;
use super::super::links::{is_short, spots};
use super::*;
use std::collections::HashMap;
use unicode_width::UnicodeWidthStr;

/// The column where a unit's bus enters its frame: the middle of the top
/// border between the title and the right corner, so the arrow never lands
/// on the title, also when a growing card is still narrow; a border too
/// short for that takes the arrow in its middle.
pub(in crate::lean::tui::graph::layout::units) fn entry_columns(
    builder: &Builder<'_>,
    units: &[Unit],
    placed: &[Placed],
    buses: &[Bus],
) -> HashMap<usize, u16> {
    let spot = spots(placed);
    buses
        .iter()
        .map(|bus| {
            let frame = spot[&bus.to];
            let skip = title_end(builder, units, bus.to, frame);
            let room = frame.width.saturating_sub(skip + 1);
            let x = if room > 0 {
                frame.x + skip + room / 2
            } else {
                frame.x + frame.width / 2
            };
            (bus.to, x)
        })
        .collect()
}

/// The first border column after the frame's title, which is drawn from
/// column 2 as ` mark title ` truncated to the width less 4.
fn title_end(builder: &Builder<'_>, units: &[Unit], unit: usize, frame: &Placed) -> u16 {
    let task = &builder.view.tasks[units[unit].task];
    let title = UnicodeWidthStr::width(clean(&task.title).as_str()) as u16 + 4;
    title.min(frame.width.saturating_sub(4)) + 2
}

/// The routed dependencies of one layout: the links, their buses, the
/// buses' lines in each gap with the gaps' tops, and where each bus enters.
pub(in crate::lean::tui::graph::layout::units) struct Wiring<'a> {
    pub(in crate::lean::tui::graph::layout::units) links: &'a [Link],
    pub(in crate::lean::tui::graph::layout::units) buses: &'a [Bus],
    pub(in crate::lean::tui::graph::layout::units) gaps: &'a [Vec<usize>],
    pub(in crate::lean::tui::graph::layout::units) tops: &'a Tops,
    pub(in crate::lean::tui::graph::layout::units) entries: &'a HashMap<usize, u16>,
}

/// Each dependency leaves its parent's frame at the bottom and joins its
/// child's bus: straight down to the bus's line in the gap above the
/// child, or along the gap below the parent to the bus's lane, which takes
/// it to that line. All links into one unit share the line and its arrow.
pub(in crate::lean::tui::graph::layout::units) fn push_links(
    builder: &mut Builder<'_>,
    units: &[Unit],
    placed: &[Placed],
    wiring: &Wiring<'_>,
) {
    let Wiring {
        links,
        buses,
        gaps,
        tops,
        entries,
    } = *wiring;
    let spot = spots(placed);
    let lanes: HashMap<usize, &Lane> = buses
        .iter()
        .filter_map(|bus| Some((bus.to, bus.lane.as_ref()?)))
        .collect();
    let exits = exit_columns(links, &spot, &lanes, entries);
    for (number, link) in links.iter().enumerate() {
        let (from, to) = (spot[&link.from], spot[&link.to]);
        let (px, cx) = (exits[&number], entries[&link.to]);
        let bottom = tops.units[from.row] + from.height - 1;
        let top = tops.units[to.row];
        let entry = gap_line(gaps, tops, to.row, link.to);
        let mut path = vec![(px, bottom)];
        if is_short(link, &spot) {
            path.push((px, entry));
        } else {
            let lane = lanes[&link.to].x;
            let exit = gap_line(gaps, tops, from.row + 1, link.to);
            path.extend([(px, exit), (lane, exit), (lane, entry)]);
        }
        path.extend([(cx, entry), (cx, top)]);
        let task = |unit: usize| format!("task:{}", builder.view.tasks[units[unit].task].id);
        let (from, to) = (task(link.from), task(link.to));
        builder.graph.edges.push(Edge {
            from,
            to,
            kind: Kind::Depends,
            label: None,
            path: Some(path),
        });
    }
}

/// Where each link leaves its parent: the links out of one frame spread
/// along its bottom border in the order of the columns they head for, so
/// they do not cross on the way.
fn exit_columns(
    links: &[Link],
    spot: &HashMap<usize, &Placed>,
    lanes: &HashMap<usize, &Lane>,
    entries: &HashMap<usize, u16>,
) -> HashMap<usize, u16> {
    let mut outgoing = HashMap::<usize, Vec<(u16, usize)>>::new();
    for (number, link) in links.iter().enumerate() {
        let toward = if is_short(link, spot) {
            entries[&link.to]
        } else {
            lanes[&link.to].x
        };
        outgoing
            .entry(link.from)
            .or_default()
            .push((toward, number));
    }
    let mut columns = HashMap::new();
    for (unit, mut ends) in outgoing {
        ends.sort_unstable();
        let frame = spot[&unit];
        let (count, width) = (ends.len() as u16, frame.width.saturating_sub(1));
        for (slot, (_, link)) in ends.into_iter().enumerate() {
            columns.insert(link, frame.x + (slot as u16 + 1) * width / (count + 1));
        }
    }
    columns
}

/// The row of the bus's line in `gap`.
fn gap_line(gaps: &[Vec<usize>], tops: &Tops, gap: usize, bus: usize) -> u16 {
    let index = gaps[gap]
        .iter()
        .position(|other| *other == bus)
        .unwrap_or(0);
    tops.gaps[gap] + 1 + index as u16
}
