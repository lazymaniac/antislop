use super::*;
use std::collections::{BTreeMap, HashMap};

pub(super) fn units(builder: &Builder<'_>) -> Vec<Unit> {
    let (mut units, unit_of) = unit_skeleton(builder);
    populate_units(builder, &mut units, &unit_of);
    units
}

fn unit_skeleton(builder: &Builder<'_>) -> (Vec<Unit>, HashMap<usize, usize>) {
    let mut unit_of = HashMap::new();
    let mut units = Vec::new();
    for work in &builder.graph.work.units {
        let Some(task) = work
            .task
            .as_deref()
            .and_then(|id| builder.index.get(id).copied())
        else {
            continue;
        };
        let members: Vec<_> = work.stages.iter().map(|stage| stage.id.0.clone()).collect();
        let mut tasks: Vec<_> = members
            .iter()
            .filter_map(|key| key.split_once(':'))
            .filter_map(|(_, id)| builder.index.get(id).copied())
            .collect();
        tasks.sort_unstable();
        tasks.dedup();
        for member in &tasks {
            unit_of.insert(*member, units.len());
        }
        units.push(Unit {
            task,
            tasks,
            members,
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
        let members = unit.tasks.iter().copied();
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
        unit.folded = unit
            .members
            .iter()
            .map(|_| folded(builder, unit.task))
            .collect();
    }
}

/// Exact runtime role addresses retained beneath this grouped card.
pub(super) fn member_keys(_view: &View, unit: &Unit) -> Vec<String> {
    unit.members.clone()
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
            "working" | "reviewing" => 1,
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
    let mut rows = ordered_rows(builder, units, by_layer);
    isolate_expanded_unit(builder, units, &mut rows);
    rows
}

/// Give the unit containing the expanded card its own measured row. Its
/// height can then grow without sharing a vertical band with other units.
fn isolate_expanded_unit(builder: &Builder<'_>, units: &[Unit], rows: &mut Vec<Vec<usize>>) {
    let Some(expanded) = builder.size.expanded_key.as_deref() else {
        return;
    };
    let Some(unit) = units.iter().position(|unit| {
        member_keys(builder.view, unit)
            .iter()
            .any(|key| key == expanded)
    }) else {
        return;
    };
    let Some(row) = rows.iter().position(|row| row.contains(&unit)) else {
        return;
    };
    if rows[row].len() == 1 {
        return;
    }
    rows[row].retain(|member| *member != unit);
    let next_row = row + 1;
    rows.insert(next_row, vec![unit]);
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
    let limit = builder.viewport.map_or(u16::MAX, |(width, _)| {
        let limit = width.saturating_sub(MARGIN + GAP_X);
        if builder.lanes == 0 {
            limit
        } else {
            limit.min(width.saturating_sub(2 * builder.lanes))
        }
    });
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
    let width = unit_width(builder, &units[unit]);
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

/// The unit's cards the layout places, each with whether it is folded: all
/// of them, or only the expanded one, which then takes the unit's width.
pub(super) fn shown_members(builder: &Builder<'_>, unit: &Unit) -> Vec<(String, bool)> {
    let key = format!("worker:{}", builder.view.tasks[unit.task].id);
    vec![(key, folded(builder, unit.task))]
}

pub(super) fn unit_width(builder: &Builder<'_>, unit: &Unit) -> u16 {
    let shown = shown_members(builder, unit);
    let widths = shown.iter().map(|(key, _)| builder.size.width(key, false));
    let frame = if super::shares_expanded_frame(builder, unit) {
        0
    } else {
        2 * FRAME_X
    };
    frame + widths.sum::<u16>() + (shown.len() as u16).saturating_sub(1) * CARD_GAP
}

pub(super) fn card_height(builder: &Builder<'_>, key: &str, folded: bool) -> u16 {
    if builder.size.expanded_key.as_deref() == Some(key) {
        card_content::height(
            builder.view,
            key,
            builder.size.width(key, false),
            builder.size.layout_height(key, false),
            false,
        )
        .max(builder.size.layout_height(key, false))
    } else if folded {
        4
    } else {
        8
    }
}

pub(super) fn measure(builder: &Builder<'_>, units: &[Unit], rows: &[Vec<usize>]) -> Vec<Placed> {
    let mut placed = Vec::new();
    for (row, members) in rows.iter().enumerate() {
        let mut x = 0;
        for &unit in members {
            let shown = shown_members(builder, &units[unit]);
            let width = unit_width(builder, &units[unit]);
            let tallest = shown
                .iter()
                .map(|(key, folded)| card_height(builder, key, *folded))
                .max()
                .unwrap_or(builder.size.h);
            let shared_expanded_frame = super::shares_expanded_frame(builder, &units[unit]);
            placed.push(Placed {
                unit,
                row,
                x,
                width,
                height: tallest
                    + if shared_expanded_frame {
                        0
                    } else {
                        2 * FRAME_Y
                    },
            });
            x += width + GAP_X;
        }
    }
    placed
}
