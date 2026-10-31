use super::build::member_keys;
use super::*;
use std::collections::{HashMap, HashSet};

mod link_render;
pub(in crate::lean::tui::graph::layout::units) use link_render::push_links;

pub(super) fn push_admin(
    builder: &mut Builder<'_>,
    rows: &[Vec<&str>],
    tops: &[u16],
    left: u16,
    room: u16,
) {
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
pub(super) fn push_unit(builder: &mut Builder<'_>, unit: &Unit, spot: &Placed, top: u16) {
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
    push_cards(builder, unit, spot, top, &group);
}

fn push_cards(builder: &mut Builder<'_>, unit: &Unit, spot: &Placed, top: u16, group: &str) {
    let keys = member_keys(builder.view, unit);
    let y = top + FRAME_Y;
    let mut previous: Option<(String, u16)> = None;
    for (slot, (key, folded)) in keys.into_iter().zip(&unit.folded).enumerate() {
        let x = spot.x + FRAME_X + slot as u16 * (builder.size.w + GAP_X);
        push_card(builder, &key, *folded, x, y, group, &mut previous);
    }
}

fn push_card(
    builder: &mut Builder<'_>,
    key: &str,
    folded: bool,
    x: u16,
    y: u16,
    group: &str,
    previous: &mut Option<(String, u16)>,
) {
    push_internal_edge(builder, key, x, y, previous);
    builder.graph.nodes.push(Node {
        key: key.to_owned(),
        x,
        y,
        width: builder.size.w,
        height: card_height(builder.size, folded),
        visible: true,
        group: Some(group.to_owned()),
        collapsed: folded,
    });
    *previous = Some((key.to_owned(), x + builder.size.w));
}

fn push_internal_edge(
    builder: &mut Builder<'_>,
    key: &str,
    x: u16,
    y: u16,
    previous: &mut Option<(String, u16)>,
) {
    let Some((from, right)) = previous.take() else {
        return;
    };
    let kind = if key.starts_with("verifier:") {
        Kind::Verifies
    } else {
        Kind::Depends
    };
    builder.graph.edges.push(Edge {
        from,
        to: key.to_owned(),
        kind,
        label: None,
        path: Some(vec![(right, y + 1), (x - 1, y + 1)]),
    });
}

/// Units the scope leaves out keep their cards, hidden, so a selection on
/// them can be told apart from one that no longer exists.
pub(super) fn push_hidden(builder: &mut Builder<'_>, units: &[Unit], placed: &[Placed]) {
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
