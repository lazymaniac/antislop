use super::*;
use std::collections::{HashMap, HashSet};

/// The dependencies between visible units, without those another path
/// already implies; one that skips rows gets a lane, the longest outermost.
pub(super) fn links(units: &[Unit], placed: &[Placed]) -> Vec<Link> {
    let spot: HashMap<usize, &Placed> = placed.iter().map(|spot| (spot.unit, spot)).collect();
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
                    lane: None,
                });
            }
        }
    }
    assign_lanes(&mut links, &spot);
    links
}

fn assign_lanes(links: &mut [Link], spot: &HashMap<usize, &Placed>) {
    // A lane runs from the gap below the parent's row to the gap above the
    // child's; lanes whose rows do not overlap share one column.
    let span = |link: &Link| {
        let (from, to) = (spot[&link.from].row, spot[&link.to].row);
        (from.min(to), from.max(to) + 1)
    };
    let long = long_links(links, spot, &span);
    let mut ends: Vec<usize> = Vec::new();
    for link in long {
        let (start, end) = span(&links[link]);
        let lane = match ends.iter().position(|&last| last < start) {
            Some(free) => free,
            None => {
                ends.push(0);
                ends.len() - 1
            }
        };
        ends[lane] = end;
        links[link].lane = Some(lane as u16);
    }
}

fn long_links(
    links: &[Link],
    spot: &HashMap<usize, &Placed>,
    span: &impl Fn(&Link) -> (usize, usize),
) -> Vec<usize> {
    let mut long: Vec<usize> = (0..links.len())
        .filter(|&link| spot[&links[link].to].row != spot[&links[link].from].row + 1)
        .collect();
    long.sort_by_key(|&link| {
        let (start, end) = span(&links[link]);
        (start, std::cmp::Reverse(end))
    });
    long
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

/// The edges through each gap, in the order of their lines: gap `g` lies
/// above unit row `g`, the last one below the last row.
pub(super) fn gap_lanes(links: &[Link], placed: &[Placed], rows: usize) -> Vec<Vec<usize>> {
    let spot: HashMap<usize, &Placed> = placed.iter().map(|spot| (spot.unit, spot)).collect();
    let mut gaps = vec![Vec::<(u16, usize)>::new(); rows + 1];
    for (number, link) in links.iter().enumerate() {
        let (from, to) = (spot[&link.from], spot[&link.to]);
        gaps[to.row].push((to.x, number));
        if link.lane.is_some() {
            gaps[from.row + 1].push((0, number));
        }
    }
    gaps.into_iter()
        .map(|mut gap| {
            gap.sort_unstable();
            gap.into_iter().map(|(_, link)| link).collect()
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
