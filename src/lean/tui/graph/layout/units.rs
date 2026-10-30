//! Tasks as units: a task's options card, worker and verifier side by side
//! in one frame. Units stand in layers by their dependencies, one row per
//! layer (wrapped when the viewport is too narrow), so a dependency always
//! runs from the bottom of one frame to the top of another: through the gap
//! between two rows when they follow each other, else down a lane of its own
//! left of every row. Each edge has its own line in a gap, and a gap grows
//! with the edges through it.

use super::super::*;
use super::builder::Builder;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Rows between a frame's border and its cards.
const FRAME_Y: u16 = 1;
/// Columns between two lanes left of the rows.
const LANE_STEP: u16 = 2;

struct Unit {
    /// The task the unit carries out; its options task rides along.
    task: usize,
    options: Option<usize>,
    parents: Vec<usize>,
    visible: bool,
    folded: Vec<bool>,
}

struct Placed {
    unit: usize,
    row: usize,
    x: u16,
    width: u16,
    height: u16,
}

/// A dependency between two placed units.
struct Link {
    from: usize,
    to: usize,
    /// The lane left of the rows, for one that skips rows.
    lane: Option<u16>,
}

pub(super) fn place(builder: &mut Builder<'_>) {
    let units = units(builder);
    let layers = layers(builder, &units);
    let rows = rows(builder, &units, &layers);
    let mut placed = measure(builder, &units, &rows);
    let links = links(&units, &placed);
    let lanes = links
        .iter()
        .filter_map(|link| link.lane)
        .max()
        .map_or(0, |lane| lane + 1);
    let admin_rows = admin_rows(builder);
    let room = room(builder, &placed, &admin_rows, MARGIN);
    center_rows(&mut placed, MARGIN, room);
    // Rows narrower than the viewport leave the lanes room of their own;
    // only what the lanes still lack moves everything right.
    let lanes_end = MARGIN + LANE_STEP * lanes;
    let leftmost = placed
        .iter()
        .map(|spot| spot.x)
        .chain(
            admin_rows
                .iter()
                .map(|row| MARGIN + (room - admin_width(builder.size, row)) / 2),
        )
        .min()
        .unwrap_or(lanes_end);
    let shift = lanes_end.saturating_sub(leftmost);
    for spot in &mut placed {
        spot.x += shift;
    }
    let left = MARGIN + shift;
    let gaps = gap_lanes(&links, &placed, rows.len());
    let tops = row_tops(builder, &admin_rows, &placed, &gaps, rows.len());
    push_admin(builder, &admin_rows, &tops.admin, left, room);
    for spot in &placed {
        push_unit(builder, &units[spot.unit], spot, tops.units[spot.row]);
    }
    push_hidden(builder, &units, &placed);
    push_links(builder, &units, &placed, &links, &gaps, &tops);
    builder.graph.height = tops.bottom;
}

fn units(builder: &Builder<'_>) -> Vec<Unit> {
    let tasks = &builder.view.tasks;
    let mut options_of = HashMap::new();
    for (index, task) in tasks.iter().enumerate().filter(|(_, task)| is_pair(task)) {
        let owner = |own: bool| {
            tasks.iter().position(|other| {
                !is_pair(other)
                    && other.depends_on.contains(&task.id)
                    && (!own || format!("{}-options", other.id) == task.id)
            })
        };
        if let Some(owner) = owner(true).or_else(|| owner(false)) {
            options_of.entry(owner).or_insert(index);
        }
    }
    let riders: HashSet<usize> = options_of.values().copied().collect();
    let mut unit_of = HashMap::new();
    let mut units: Vec<Unit> = Vec::new();
    for index in (0..tasks.len()).filter(|index| !riders.contains(index)) {
        let options = options_of.get(&index).copied();
        unit_of.insert(index, units.len());
        if let Some(options) = options {
            unit_of.insert(options, units.len());
        }
        units.push(Unit {
            task: index,
            options,
            parents: Vec::new(),
            visible: false,
            folded: Vec::new(),
        });
    }
    for (number, unit) in units.iter_mut().enumerate() {
        let members = unit.options.into_iter().chain([unit.task]);
        let mut parents: Vec<usize> = members
            .flat_map(|member| &tasks[member].depends_on)
            .filter_map(|id| builder.index.get(id.as_str()))
            .filter_map(|task| unit_of.get(task).copied())
            .filter(|parent| *parent != number)
            .collect();
        parents.sort_unstable();
        parents.dedup();
        unit.parents = parents;
        unit.visible = member_keys(builder.view, unit)
            .iter()
            .any(|key| builder.matches_query(key));
        unit.folded = member_tasks(builder.view, unit)
            .into_iter()
            .map(|task| folded(builder, task))
            .collect();
    }
    units
}

/// The task behind each of the unit's cards, in the order of `member_keys`.
fn member_tasks(view: &View, unit: &Unit) -> Vec<usize> {
    let mut tasks: Vec<usize> = unit.options.into_iter().collect();
    tasks.push(unit.task);
    if !is_pair(&view.tasks[unit.task]) {
        tasks.push(unit.task);
    }
    tasks
}

/// The unit's cards from left to right: options, worker, verifier.
fn member_keys(view: &View, unit: &Unit) -> Vec<String> {
    let task = &view.tasks[unit.task];
    let mut keys: Vec<String> = unit
        .options
        .map(|options| format!("worker:{}", view.tasks[options].id))
        .into_iter()
        .collect();
    keys.push(format!("worker:{}", task.id));
    if !is_pair(task) {
        keys.push(format!("verifier:{}", task.id));
    }
    keys
}

/// A finished task's cards are folded to their identity rows; toggling a
/// card (`z`) flips that, unfolding a finished one or folding a running one.
fn folded(builder: &Builder<'_>, index: usize) -> bool {
    let task = &builder.view.tasks[index];
    let toggled = builder.graph.collapsed.contains(&task.id)
        || builder
            .graph
            .collapsed
            .contains(&format!("task:{}", task.id));
    let finished = matches!(
        task.status.as_str(),
        "approved" | "skipped" | "cancelled" | "revised"
    );
    finished != toggled
}

/// Each unit's layer: one past its deepest parent, or the task's state
/// when the graph groups by state. A cycle is cut where it closes.
fn layers(builder: &Builder<'_>, units: &[Unit]) -> Vec<u16> {
    if builder.graph.grouping == GroupingMode::Status {
        return units
            .iter()
            .map(|unit| match builder.view.tasks[unit.task].status.as_str() {
                "proposed" | "queued" => 0,
                "working" | "verifying" => 1,
                "approved" => 2,
                _ => 3,
            })
            .collect();
    }
    let mut layer = vec![0u16; units.len()];
    let mut pending: Vec<usize> = units.iter().map(|unit| unit.parents.len()).collect();
    let mut children = vec![Vec::new(); units.len()];
    for (child, unit) in units.iter().enumerate() {
        for &parent in &unit.parents {
            children[parent].push(child);
        }
    }
    let mut queue: std::collections::VecDeque<usize> = (0..units.len())
        .filter(|unit| pending[*unit] == 0)
        .collect();
    while let Some(parent) = queue.pop_front() {
        for &child in &children[parent] {
            layer[child] = layer[child].max(layer[parent] + 1);
            pending[child] -= 1;
            if pending[child] == 0 {
                queue.push_back(child);
            }
        }
    }
    layer
}

/// The visible units row by row: layer after layer, each ordered under its
/// parents, and wrapped where the viewport ends.
fn rows(builder: &Builder<'_>, units: &[Unit], layers: &[u16]) -> Vec<Vec<usize>> {
    let mut by_layer = BTreeMap::<u16, Vec<usize>>::new();
    for (number, _) in units.iter().enumerate().filter(|(_, unit)| unit.visible) {
        by_layer.entry(layers[number]).or_default().push(number);
    }
    let limit = builder
        .viewport
        .map_or(u16::MAX, |(width, _)| width.saturating_sub(MARGIN + GAP_X));
    let mut position = HashMap::<usize, f64>::new();
    let mut rows = Vec::new();
    for (_, mut layer) in by_layer {
        let order = |unit: &usize| {
            let known: Vec<f64> = units[*unit]
                .parents
                .iter()
                .filter_map(|parent| position.get(parent).copied())
                .collect();
            if known.is_empty() {
                f64::MAX
            } else {
                known.iter().sum::<f64>() / known.len() as f64
            }
        };
        layer.sort_by(|a, b| order(a).total_cmp(&order(b)).then(a.cmp(b)));
        let mut row: Vec<usize> = Vec::new();
        let mut used = 0u16;
        for (slot, unit) in layer.into_iter().enumerate() {
            position.insert(unit, slot as f64);
            let width = unit_width(builder.size, &units[unit]);
            if !row.is_empty() && used.saturating_add(GAP_X + width) > limit {
                rows.push(std::mem::take(&mut row));
                used = 0;
            }
            used = if row.is_empty() {
                width
            } else {
                used.saturating_add(GAP_X + width)
            };
            row.push(unit);
        }
        if !row.is_empty() {
            rows.push(row);
        }
    }
    rows
}

fn cards(unit: &Unit) -> u16 {
    unit.folded.len() as u16
}

fn unit_width(size: Size, unit: &Unit) -> u16 {
    let cards = cards(unit);
    2 * FRAME_X + cards * size.w + cards.saturating_sub(1) * GAP_X
}

fn card_height(size: Size, folded: bool) -> u16 {
    if folded {
        size.h.min(FOLDED_HEIGHT)
    } else {
        size.h
    }
}

fn measure(builder: &Builder<'_>, units: &[Unit], rows: &[Vec<usize>]) -> Vec<Placed> {
    let mut placed = Vec::new();
    for (row, members) in rows.iter().enumerate() {
        let mut x = 0;
        for &unit in members {
            let width = unit_width(builder.size, &units[unit]);
            let tallest = units[unit]
                .folded
                .iter()
                .map(|folded| card_height(builder.size, *folded))
                .max()
                .unwrap_or(builder.size.h);
            placed.push(Placed {
                unit,
                row,
                x,
                width,
                height: tallest + 2 * FRAME_Y,
            });
            x += width + GAP_X;
        }
    }
    placed
}

/// The dependencies between visible units, without those another path
/// already implies; one that skips rows gets a lane, the longest outermost.
fn links(units: &[Unit], placed: &[Placed]) -> Vec<Link> {
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
    // A lane runs from the gap below the parent's row to the gap above the
    // child's; lanes whose rows do not overlap share one column.
    let span = |link: &Link| {
        let (from, to) = (spot[&link.from].row, spot[&link.to].row);
        (from.min(to), from.max(to) + 1)
    };
    let mut long: Vec<usize> = (0..links.len())
        .filter(|&link| spot[&links[link].to].row != spot[&links[link].from].row + 1)
        .collect();
    long.sort_by_key(|&link| {
        (
            span(&links[link]).0,
            std::cmp::Reverse(span(&links[link]).1),
        )
    });
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
    links
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

/// The session's own agents in rows of as many as fit across.
fn admin_rows<'a>(builder: &Builder<'a>) -> Vec<Vec<&'a str>> {
    let visible: Vec<&str> = builder
        .admin
        .iter()
        .filter(|(_, visible)| *visible)
        .map(|(key, _)| *key)
        .collect();
    let across = builder.viewport.map_or(visible.len().max(1), |(width, _)| {
        usize::from(width.saturating_sub(MARGIN) / (builder.size.admin_w + GAP_X)).max(1)
    });
    visible.chunks(across).map(<[&str]>::to_vec).collect()
}

fn admin_width(size: Size, row: &[&str]) -> u16 {
    let cards = row.len() as u16;
    cards * size.admin_w + cards.saturating_sub(1) * GAP_X
}

/// The width every row is centered in, right of the lanes: the viewport's,
/// or the widest row's when that is wider.
fn room(builder: &Builder<'_>, placed: &[Placed], admin: &[Vec<&str>], left: u16) -> u16 {
    let widest = placed
        .iter()
        .map(|spot| spot.x + spot.width)
        .chain(admin.iter().map(|row| admin_width(builder.size, row)))
        .max()
        .unwrap_or(0);
    builder.viewport.map_or(widest, |(width, _)| {
        width.saturating_sub(left + GAP_X).max(widest)
    })
}

fn center_rows(placed: &mut [Placed], left: u16, room: u16) {
    let mut widths = BTreeMap::<usize, u16>::new();
    for spot in placed.iter() {
        widths.insert(spot.row, spot.x + spot.width);
    }
    for spot in placed.iter_mut() {
        spot.x += left + (room - widths[&spot.row]) / 2;
    }
}

/// The edges through each gap, in the order of their lines: gap `g` lies
/// above unit row `g`, the last one below the last row.
fn gap_lanes(links: &[Link], placed: &[Placed], rows: usize) -> Vec<Vec<usize>> {
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

fn gap_height(lines: usize, tight: bool) -> u16 {
    if lines == 0 && tight {
        0
    } else {
        GAP_Y.max(lines as u16 + 2)
    }
}

struct Tops {
    admin: Vec<u16>,
    units: Vec<u16>,
    /// Where each gap starts.
    gaps: Vec<u16>,
    bottom: u16,
}

fn row_tops(
    builder: &Builder<'_>,
    admin: &[Vec<&str>],
    placed: &[Placed],
    gaps: &[Vec<usize>],
    rows: usize,
) -> Tops {
    let admin_height = ADMIN_HEIGHT.min(builder.size.h);
    let mut y = 0u16;
    let mut admin_tops = Vec::new();
    for (index, _) in admin.iter().enumerate() {
        if index > 0 {
            y += GAP_Y;
        }
        admin_tops.push(y);
        y += admin_height;
    }
    let mut heights = vec![0u16; rows];
    for spot in placed {
        heights[spot.row] = heights[spot.row].max(spot.height);
    }
    let mut unit_tops = Vec::new();
    let mut gap_tops = Vec::new();
    for (row, height) in heights.iter().enumerate() {
        gap_tops.push(y);
        y += gap_height(gaps[row].len(), row == 0 && admin.is_empty());
        unit_tops.push(y);
        y += height;
    }
    gap_tops.push(y);
    let below = gaps.get(rows).map_or(0, Vec::len);
    if below > 0 {
        y += gap_height(below, false);
    }
    Tops {
        admin: admin_tops,
        units: unit_tops,
        gaps: gap_tops,
        bottom: y,
    }
}

fn push_admin(builder: &mut Builder<'_>, rows: &[Vec<&str>], tops: &[u16], left: u16, room: u16) {
    let (width, height) = (builder.size.admin_w, ADMIN_HEIGHT.min(builder.size.h));
    let mut shown = HashMap::<&str, (u16, u16)>::new();
    for (row, &y) in rows.iter().zip(tops) {
        let x = left + (room - admin_width(builder.size, row)) / 2;
        for (slot, key) in row.iter().enumerate() {
            shown.insert(*key, (x + slot as u16 * (width + GAP_X), y));
        }
    }
    let admin = builder.admin.clone();
    for (key, _) in admin {
        let spot = shown.get(key).copied();
        builder.graph.nodes.push(Node {
            key: key.to_owned(),
            x: spot.map_or(0, |(x, _)| x),
            y: spot.map_or(0, |(_, y)| y),
            width,
            height,
            visible: spot.is_some(),
            group: None,
            collapsed: false,
        });
    }
}

/// The unit's frame and cards, and the arrows between the cards: from the
/// options to the worker, from the worker to its verifier.
fn push_unit(builder: &mut Builder<'_>, unit: &Unit, spot: &Placed, top: u16) {
    let task = &builder.view.tasks[unit.task];
    let group = format!("task:{}", task.id);
    builder.graph.frames.push(Frame {
        key: group.clone(),
        task: task.id.clone(),
        x: spot.x,
        y: top,
        width: spot.width,
        height: spot.height,
        visible: true,
    });
    let keys = member_keys(builder.view, unit);
    let y = top + FRAME_Y;
    let mut previous: Option<(String, u16)> = None;
    for (slot, (key, folded)) in keys.into_iter().zip(&unit.folded).enumerate() {
        let x = spot.x + FRAME_X + slot as u16 * (builder.size.w + GAP_X);
        if let Some((from, right)) = previous.take() {
            let kind = if key.starts_with("verifier:") {
                Kind::Verifies
            } else {
                Kind::Depends
            };
            builder.graph.edges.push(Edge {
                from,
                to: key.clone(),
                kind,
                label: None,
                path: Some(vec![(right, y + 1), (x - 1, y + 1)]),
            });
        }
        builder.graph.nodes.push(Node {
            key: key.clone(),
            x,
            y,
            width: builder.size.w,
            height: card_height(builder.size, *folded),
            visible: true,
            group: Some(group.clone()),
            collapsed: *folded,
        });
        previous = Some((key, x + builder.size.w));
    }
}

/// Units the scope leaves out keep their cards, hidden, so a selection on
/// them can be told apart from one that no longer exists.
fn push_hidden(builder: &mut Builder<'_>, units: &[Unit], placed: &[Placed]) {
    let shown: HashSet<usize> = placed.iter().map(|spot| spot.unit).collect();
    for (number, unit) in units.iter().enumerate() {
        if shown.contains(&number) {
            continue;
        }
        let group = format!("task:{}", builder.view.tasks[unit.task].id);
        for (key, folded) in member_keys(builder.view, unit)
            .into_iter()
            .zip(&unit.folded)
        {
            builder.graph.nodes.push(Node {
                key,
                x: 0,
                y: 0,
                width: builder.size.w,
                height: builder.size.h,
                visible: false,
                group: Some(group.clone()),
                collapsed: *folded,
            });
        }
    }
}

/// Each dependency leaves its parent's frame at the bottom and enters its
/// child's at the top, the points spread along the border in the order of
/// the other ends, so edges leaving or entering one frame do not cross.
fn push_links(
    builder: &mut Builder<'_>,
    units: &[Unit],
    placed: &[Placed],
    links: &[Link],
    gaps: &[Vec<usize>],
    tops: &Tops,
) {
    let spot: HashMap<usize, &Placed> = placed.iter().map(|spot| (spot.unit, spot)).collect();
    let line = |gap: usize, link: usize| {
        let index = gaps[gap]
            .iter()
            .position(|other| *other == link)
            .unwrap_or(0);
        tops.gaps[gap] + 1 + index as u16
    };
    let far = |link: &Link, end: usize| if link.lane.is_some() { 0 } else { spot[&end].x };
    let mut exits = HashMap::<usize, Vec<(u16, usize)>>::new();
    let mut entries = HashMap::<usize, Vec<(u16, usize)>>::new();
    for (number, link) in links.iter().enumerate() {
        exits
            .entry(link.from)
            .or_default()
            .push((far(link, link.to), number));
        entries
            .entry(link.to)
            .or_default()
            .push((far(link, link.from), number));
    }
    // Edges enter a frame right of its title, where there is room.
    let title_end = |unit: usize| {
        let task = &builder.view.tasks[units[unit].task];
        let title = UnicodeWidthStr::width(clean(&task.title).as_str()) as u16 + 4;
        title.min(spot[&unit].width.saturating_sub(4)) + 3
    };
    let spread = |ends: HashMap<usize, Vec<(u16, usize)>>, titled: bool| {
        let mut at = HashMap::<usize, u16>::new();
        for (unit, mut ends) in ends {
            ends.sort_unstable();
            let frame = spot[&unit];
            let count = ends.len() as u16;
            let skip = if titled { title_end(unit) } else { 0 };
            let (start, width) = if frame.width.saturating_sub(skip + 2) > count {
                (frame.x + skip, frame.width - skip - 1)
            } else {
                (frame.x, frame.width)
            };
            for (slot, (_, link)) in ends.into_iter().enumerate() {
                at.insert(link, start + (slot as u16 + 1) * width / (count + 1));
            }
        }
        at
    };
    let (exit_x, entry_x) = (spread(exits, false), spread(entries, true));
    for (number, link) in links.iter().enumerate() {
        let (from, to) = (spot[&link.from], spot[&link.to]);
        let bottom = tops.units[from.row] + from.height - 1;
        let top = tops.units[to.row];
        let (px, cx) = (exit_x[&number], entry_x[&number]);
        let entry = line(to.row, number);
        let path = match link.lane {
            None => vec![(px, bottom), (px, entry), (cx, entry), (cx, top)],
            Some(lane) => {
                let exit = line(from.row + 1, number);
                let x = MARGIN - 1 + LANE_STEP * lane;
                vec![
                    (px, bottom),
                    (px, exit),
                    (x, exit),
                    (x, entry),
                    (cx, entry),
                    (cx, top),
                ]
            }
        };
        builder.graph.edges.push(Edge {
            from: format!("task:{}", builder.view.tasks[units[link.from].task].id),
            to: format!("task:{}", builder.view.tasks[units[link.to].task].id),
            kind: Kind::Depends,
            label: None,
            path: Some(path),
        });
    }
}
