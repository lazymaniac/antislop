use super::super::super::*;
use super::*;
use std::collections::HashMap;
use unicode_width::UnicodeWidthStr;

/// Each dependency leaves its parent's frame at the bottom and enters its
/// child's at the top, the points spread along the border in the order of
/// the other ends, so edges leaving or entering one frame do not cross.
pub(in crate::lean::tui::graph::layout::units) fn push_links(
    builder: &mut Builder<'_>,
    units: &[Unit],
    placed: &[Placed],
    links: &[Link],
    gaps: &[Vec<usize>],
    tops: &Tops,
) {
    let spots = placed
        .iter()
        .map(|spot| (spot.unit, spot))
        .collect::<HashMap<_, _>>();
    let (exits, entries) = link_columns(builder, units, links, &spots);
    let mut drawing = LinkDrawing {
        builder,
        units,
        gaps,
        tops,
        spots: &spots,
        exits: &exits,
        entries: &entries,
    };
    for (number, link) in links.iter().enumerate() {
        drawing.push(link, number);
    }
}

fn link_columns(
    builder: &Builder<'_>,
    units: &[Unit],
    links: &[Link],
    spots: &HashMap<usize, &Placed>,
) -> (HashMap<usize, u16>, HashMap<usize, u16>) {
    let outgoing = link_ends(links, spots, true);
    let incoming = link_ends(links, spots, false);
    let exits = spread_ends(outgoing, spots, None);
    let entries = spread_ends(incoming, spots, Some((builder, units)));
    (exits, entries)
}

fn link_ends(
    links: &[Link],
    spots: &HashMap<usize, &Placed>,
    outgoing: bool,
) -> HashMap<usize, Vec<(u16, usize)>> {
    let mut ends = HashMap::new();
    for (number, link) in links.iter().enumerate() {
        let (unit, other) = if outgoing {
            (link.from, link.to)
        } else {
            (link.to, link.from)
        };
        ends.entry(unit)
            .or_insert_with(Vec::new)
            .push((far(link, other, spots), number));
    }
    ends
}

fn far(link: &Link, end: usize, spots: &HashMap<usize, &Placed>) -> u16 {
    if link.lane.is_some() {
        0
    } else {
        spots[&end].x
    }
}

fn spread_ends(
    ends: HashMap<usize, Vec<(u16, usize)>>,
    spots: &HashMap<usize, &Placed>,
    titled: Option<(&Builder<'_>, &[Unit])>,
) -> HashMap<usize, u16> {
    let mut columns = HashMap::new();
    for (unit, mut ends) in ends {
        ends.sort_unstable();
        let frame = spots[&unit];
        let skip = titled.map_or(0, |(builder, units)| title_end(builder, units, unit, frame));
        spread_frame_ends(&mut columns, frame, ends, skip);
    }
    columns
}

fn title_end(builder: &Builder<'_>, units: &[Unit], unit: usize, frame: &Placed) -> u16 {
    let task = &builder.view.tasks[units[unit].task];
    let title = UnicodeWidthStr::width(clean(&task.title).as_str()) as u16 + 4;
    title.min(frame.width.saturating_sub(4)) + 3
}

fn spread_frame_ends(
    columns: &mut HashMap<usize, u16>,
    frame: &Placed,
    ends: Vec<(u16, usize)>,
    skip: u16,
) {
    let count = ends.len() as u16;
    let (start, width) = if frame.width.saturating_sub(skip + 2) > count {
        (frame.x + skip, frame.width - skip - 1)
    } else {
        (frame.x, frame.width)
    };
    for (slot, (_, link)) in ends.into_iter().enumerate() {
        columns.insert(link, start + (slot as u16 + 1) * width / (count + 1));
    }
}

struct LinkDrawing<'a, 'b, 'c> {
    builder: &'a mut Builder<'b>,
    units: &'c [Unit],
    gaps: &'c [Vec<usize>],
    tops: &'c Tops,
    spots: &'c HashMap<usize, &'c Placed>,
    exits: &'c HashMap<usize, u16>,
    entries: &'c HashMap<usize, u16>,
}

impl LinkDrawing<'_, '_, '_> {
    fn push(&mut self, link: &Link, number: usize) {
        let from = self.spots[&link.from];
        let to = self.spots[&link.to];
        let route = LinkRoute {
            number,
            bottom: self.tops.units[from.row] + from.height - 1,
            top: self.tops.units[to.row],
            columns: (self.exits[&number], self.entries[&number]),
            rows: (from.row, to.row),
        };
        let path = link_path(link, self.gaps, self.tops, &route);
        self.builder.graph.edges.push(Edge {
            from: format!(
                "task:{}",
                self.builder.view.tasks[self.units[link.from].task].id
            ),
            to: format!(
                "task:{}",
                self.builder.view.tasks[self.units[link.to].task].id
            ),
            kind: Kind::Depends,
            label: None,
            path: Some(path),
        });
    }
}

struct LinkRoute {
    number: usize,
    bottom: u16,
    top: u16,
    columns: (u16, u16),
    rows: (usize, usize),
}

fn gap_line(gaps: &[Vec<usize>], tops: &Tops, gap: usize, link: usize) -> u16 {
    let index = gaps[gap]
        .iter()
        .position(|other| *other == link)
        .unwrap_or(0);
    tops.gaps[gap] + 1 + index as u16
}

fn link_path(link: &Link, gaps: &[Vec<usize>], tops: &Tops, route: &LinkRoute) -> Vec<(u16, u16)> {
    let (px, cx) = route.columns;
    let (from_row, to_row) = route.rows;
    let entry = gap_line(gaps, tops, to_row, route.number);
    match link.lane {
        None => vec![
            (px, route.bottom),
            (px, entry),
            (cx, entry),
            (cx, route.top),
        ],
        Some(lane) => {
            let exit = gap_line(gaps, tops, from_row + 1, route.number);
            let x = MARGIN - 1 + LANE_STEP * lane;
            vec![
                (px, route.bottom),
                (px, exit),
                (x, exit),
                (x, entry),
                (cx, entry),
                (cx, route.top),
            ]
        }
    }
}
