use super::*;
use std::collections::{BTreeMap, HashMap, HashSet};

pub(super) fn units(builder: &Builder<'_>) -> Vec<Unit> {
    let tasks = &builder.view.tasks;
    let options_of = options_of(tasks);
    let (mut units, unit_of) = unit_skeleton(tasks, &options_of);
    populate_units(builder, &mut units, &unit_of);
    units
}

fn options_of(tasks: &[TaskRow]) -> HashMap<usize, usize> {
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
    options_of
}

fn unit_skeleton(
    tasks: &[TaskRow],
    options_of: &HashMap<usize, usize>,
) -> (Vec<Unit>, HashMap<usize, usize>) {
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
    (units, unit_of)
}

fn populate_units(builder: &Builder<'_>, units: &mut [Unit], unit_of: &HashMap<usize, usize>) {
    let tasks = &builder.view.tasks;
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
pub(super) fn member_keys(view: &View, unit: &Unit) -> Vec<String> {
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
pub(super) fn layers(builder: &Builder<'_>, units: &[Unit]) -> Vec<u16> {
    if builder.graph.grouping == GroupingMode::Status {
        return status_layers(builder, units);
    }
    dependency_layers(units)
}

fn status_layers(builder: &Builder<'_>, units: &[Unit]) -> Vec<u16> {
    units
        .iter()
        .map(|unit| match builder.view.tasks[unit.task].status.as_str() {
            "proposed" | "queued" => 0,
            "working" | "verifying" => 1,
            "approved" => 2,
            _ => 3,
        })
        .collect()
}

fn dependency_layers(units: &[Unit]) -> Vec<u16> {
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
pub(super) fn rows(builder: &Builder<'_>, units: &[Unit], layers: &[u16]) -> Vec<Vec<usize>> {
    let mut by_layer = BTreeMap::<u16, Vec<usize>>::new();
    for (number, _) in units.iter().enumerate().filter(|(_, unit)| unit.visible) {
        by_layer.entry(layers[number]).or_default().push(number);
    }
    ordered_rows(builder, units, by_layer)
}

fn ordered_rows(
    builder: &Builder<'_>,
    units: &[Unit],
    by_layer: BTreeMap<u16, Vec<usize>>,
) -> Vec<Vec<usize>> {
    let mut position = HashMap::<usize, f64>::new();
    let mut rows = Vec::new();
    for (_, layer) in by_layer {
        append_layer_rows(builder, units, layer, &mut position, &mut rows);
    }
    rows
}

fn append_layer_rows(
    builder: &Builder<'_>,
    units: &[Unit],
    mut layer: Vec<usize>,
    position: &mut HashMap<usize, f64>,
    rows: &mut Vec<Vec<usize>>,
) {
    sort_by_parent_position(units, &mut layer, position);
    let limit = builder
        .viewport
        .map_or(u16::MAX, |(width, _)| width.saturating_sub(MARGIN + GAP_X));
    wrap_layer(builder, units, layer, limit, position, rows);
}

fn sort_by_parent_position(units: &[Unit], layer: &mut [usize], position: &HashMap<usize, f64>) {
    let position_of = |unit: &usize| {
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
    layer.sort_by(|a, b| position_of(a).total_cmp(&position_of(b)).then(a.cmp(b)));
}

fn wrap_layer(
    builder: &Builder<'_>,
    units: &[Unit],
    layer: Vec<usize>,
    limit: u16,
    position: &mut HashMap<usize, f64>,
    rows: &mut Vec<Vec<usize>>,
) {
    let mut row = Vec::new();
    let mut used = 0u16;
    for (slot, unit) in layer.into_iter().enumerate() {
        position.insert(unit, slot as f64);
        push_wrapped_unit(builder, units, unit, limit, &mut used, &mut row, rows);
    }
    if !row.is_empty() {
        rows.push(row);
    }
}

fn push_wrapped_unit(
    builder: &Builder<'_>,
    units: &[Unit],
    unit: usize,
    limit: u16,
    used: &mut u16,
    row: &mut Vec<usize>,
    rows: &mut Vec<Vec<usize>>,
) {
    let width = unit_width(builder.size, &units[unit]);
    if !row.is_empty() && used.saturating_add(GAP_X + width) > limit {
        rows.push(std::mem::take(row));
        *used = 0;
    }
    *used = if row.is_empty() {
        width
    } else {
        used.saturating_add(GAP_X + width)
    };
    row.push(unit);
}

fn cards(unit: &Unit) -> u16 {
    unit.folded.len() as u16
}

pub(super) fn unit_width(size: Size, unit: &Unit) -> u16 {
    let cards = cards(unit);
    2 * FRAME_X + cards * size.w + cards.saturating_sub(1) * GAP_X
}

pub(super) fn card_height(size: Size, folded: bool) -> u16 {
    if folded {
        size.h.min(FOLDED_HEIGHT)
    } else {
        size.h
    }
}

pub(super) fn measure(builder: &Builder<'_>, units: &[Unit], rows: &[Vec<usize>]) -> Vec<Placed> {
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
