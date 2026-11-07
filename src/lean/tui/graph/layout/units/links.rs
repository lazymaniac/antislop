use super::*;
use std::collections::{BTreeMap, HashMap, HashSet};

/// The dependencies between visible units, without those another path
/// already implies.
pub(super) fn links(units: &[Unit], placed: &[Placed]) -> Vec<Link> {
    let spot = spots(placed);
    let mut links: Vec<Link> = Vec::new();
    for (child, unit) in units.iter().enumerate() {
        if !spot.contains_key(&child) {
            continue;
        }
        for &parent in unit
            .parents
            .iter()
            .filter(|parent| spot.contains_key(parent))
        {
            if !implied(units, &spot, parent, child) {
                links.push(Link {
                    from: parent,
                    to: child,
                });
            }
        }
    }
    links
}

pub(super) fn spots(placed: &[Placed]) -> HashMap<usize, &Placed> {
    placed.iter().map(|spot| (spot.unit, spot)).collect()
}

/// Whether a link runs from the row right above its child, straight through
/// the gap between them; any other link takes its bus's lane.
pub(super) fn is_short(link: &Link, spot: &HashMap<usize, &Placed>) -> bool {
    spot[&link.from].row + 1 == spot[&link.to].row
}

/// One bus per unit with dependencies. A bus fed from rows other than the
/// one right above gets a lane on the side its ends lean to; lanes whose
/// gaps do not overlap share a column, the longest outermost.
pub(super) fn buses(links: &[Link], placed: &[Placed]) -> Vec<Bus> {
    let spot = spots(placed);
    let mut buses: Vec<Bus> = Vec::new();
    for link in links {
        if !buses.iter().any(|bus| bus.to == link.to) {
            buses.push(Bus {
                to: link.to,
                lane: None,
            });
        }
    }
    let lean = Lean::new(placed);
    let mut sides = BTreeMap::<usize, (Side, (usize, usize))>::new();
    let mut counts = (0usize, 0usize);
    for (number, bus) in buses.iter().enumerate() {
        let long: Vec<&Link> = links
            .iter()
            .filter(|link| link.to == bus.to && !is_short(link, &spot))
            .collect();
        if long.is_empty() {
            continue;
        }
        let child = spot[&bus.to].row;
        let exits = long.iter().map(|link| spot[&link.from].row + 1);
        let span = exits
            .chain([child])
            .fold((usize::MAX, 0), |(low, high), gap| {
                (low.min(gap), high.max(gap))
            });
        let offset: i32 =
            long.iter().map(|link| lean.offset(link.from)).sum::<i32>() + lean.offset(bus.to);
        let side = match offset.cmp(&0) {
            std::cmp::Ordering::Less => Side::Left,
            std::cmp::Ordering::Greater => Side::Right,
            std::cmp::Ordering::Equal if counts.0 <= counts.1 => Side::Left,
            std::cmp::Ordering::Equal => Side::Right,
        };
        match side {
            Side::Left => counts.0 += 1,
            Side::Right => counts.1 += 1,
        }
        sides.insert(number, (side, span));
    }
    for side in [Side::Left, Side::Right] {
        for (number, index) in pack(&sides, side) {
            let (_, span) = sides[&number];
            buses[number].lane = Some(Lane {
                side,
                index,
                span,
                x: 0,
            });
        }
    }
    buses
}

/// Each unit's offset from the middle of its row, doubled to stay whole.
struct Lean<'a> {
    spot: HashMap<usize, &'a Placed>,
    rows: HashMap<usize, u16>,
}

impl<'a> Lean<'a> {
    fn new(placed: &'a [Placed]) -> Self {
        let mut rows = HashMap::<usize, u16>::new();
        for spot in placed {
            let right = rows.entry(spot.row).or_default();
            *right = (*right).max(spot.x + spot.width);
        }
        Self {
            spot: spots(placed),
            rows,
        }
    }

    fn offset(&self, unit: usize) -> i32 {
        let spot = self.spot[&unit];
        2 * i32::from(spot.x) + i32::from(spot.width) - i32::from(self.rows[&spot.row])
    }
}

/// First-fit columns for one side's lanes, the longest first among those
/// starting together, so nested lanes do not cross.
fn pack(sides: &BTreeMap<usize, (Side, (usize, usize))>, side: Side) -> Vec<(usize, u16)> {
    let mut lanes: Vec<(usize, (usize, usize))> = sides
        .iter()
        .filter(|(_, (own, _))| *own == side)
        .map(|(number, (_, span))| (*number, *span))
        .collect();
    lanes.sort_by_key(|(_, (start, end))| (*start, std::cmp::Reverse(*end)));
    let mut ends: Vec<usize> = Vec::new();
    lanes
        .into_iter()
        .map(|(number, (start, end))| {
            let column = match ends.iter().position(|&last| last < start) {
                Some(free) => free,
                None => {
                    ends.push(0);
                    ends.len() - 1
                }
            };
            ends[column] = end;
            (number, column as u16)
        })
        .collect()
}

/// The columns of lanes on each side.
pub(super) fn lane_counts(buses: &[Bus]) -> (u16, u16) {
    buses
        .iter()
        .filter_map(|bus| bus.lane.as_ref())
        .fold((0, 0), |(left, right), lane| match lane.side {
            Side::Left => (left.max(lane.index + 1), right),
            Side::Right => (left, right.max(lane.index + 1)),
        })
}

/// The columns each side keeps clear of the rows the lanes pass: one blank
/// column between neighbouring lanes, the rows and the viewport's edge.
pub(super) fn lane_reserve(buses: &[Bus]) -> u16 {
    let (left, right) = lane_counts(buses);
    let lanes = left.max(right);
    if lanes == 0 {
        0
    } else {
        MARGIN.max(1 + LANE_STEP * lanes)
    }
}

/// Whether any lane runs beside `row`.
pub(super) fn crossed(buses: &[Bus], row: usize) -> bool {
    buses
        .iter()
        .filter_map(|bus| bus.lane.as_ref())
        .any(|lane| lane.crosses(row))
}

/// Put the lanes beside the rows they pass, hugging them on either side.
pub(super) fn place_lanes(buses: &mut [Bus], placed: &[Placed]) {
    let (left_count, right_count) = lane_counts(buses);
    let passed = placed
        .iter()
        .filter(|spot| crossed(buses, spot.row))
        .map(|spot| (spot.x, spot.x + spot.width));
    let Some((left, right)) = passed.reduce(|(l, r), (x, end)| (l.min(x), r.max(end))) else {
        return;
    };
    for lane in buses.iter_mut().filter_map(|bus| bus.lane.as_mut()) {
        lane.x = match lane.side {
            Side::Left => left
                .saturating_sub(2)
                .saturating_sub(LANE_STEP * (left_count - 1 - lane.index)),
            Side::Right => right + 1 + LANE_STEP * (right_count - 1 - lane.index),
        };
    }
}

/// Whether `child` reaches `parent` through another visible parent.
fn implied(units: &[Unit], spot: &HashMap<usize, &Placed>, parent: usize, child: usize) -> bool {
    let mut stack: Vec<usize> = units[child]
        .parents
        .iter()
        .copied()
        .filter(|other| *other != parent && spot.contains_key(other))
        .collect();
    let mut seen = HashSet::new();
    while let Some(unit) = stack.pop() {
        if unit == parent {
            return true;
        }
        if seen.insert(unit) {
            stack.extend(
                units[unit]
                    .parents
                    .iter()
                    .copied()
                    .filter(|other| spot.contains_key(other)),
            );
        }
    }
    false
}

/// The buses with a line in each gap, top to bottom: gap `g` lies above
/// unit row `g`, the last one below the last row. A bus has one line in the
/// gap above its unit, where all its links enter, and one in each gap below
/// a row its lane collects links from. Lines to lanes go first, outermost
/// lanes first, so the lines to the units below meet none of them on the
/// way down.
pub(super) fn gap_lines(
    links: &[Link],
    buses: &[Bus],
    placed: &[Placed],
    rows: usize,
    entries: &HashMap<usize, u16>,
) -> Vec<Vec<usize>> {
    let spot = spots(placed);
    let mut gaps = vec![Vec::<((u8, u16), usize)>::new(); rows + 1];
    for bus in buses {
        gaps[spot[&bus.to].row].push(((1, entries[&bus.to]), bus.to));
        let Some(lane) = &bus.lane else {
            continue;
        };
        for link in links
            .iter()
            .filter(|link| link.to == bus.to && !is_short(link, &spot))
        {
            let gap = &mut gaps[spot[&link.from].row + 1];
            if !gap.iter().any(|(_, owner)| *owner == bus.to) {
                gap.push(((0, lane.index), bus.to));
            }
        }
    }
    gaps.into_iter()
        .map(|mut gap| {
            gap.sort_unstable();
            gap.into_iter().map(|(_, bus)| bus).collect()
        })
        .collect()
}

pub(super) fn gap_height(lines: usize, tight: bool) -> u16 {
    if lines == 0 && tight {
        0
    } else {
        GAP_Y.max(lines as u16 + 2)
    }
}
