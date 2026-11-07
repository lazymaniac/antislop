//! Lay task cards out in dependency rows with routed links.

use super::super::*;
use super::builder::Builder;

mod build;
mod links;
mod model;
mod placement;
mod render;

use build::{card_height, layers, measure, member_keys, rows, units};
use links::{gap_lanes, links};
use model::{Link, Placed, Tops, Unit};
use placement::{admin_rows, admin_width, center_rows, room, row_tops};
use render::{push_admin, push_hidden, push_links, push_unit};

/// Rows between a frame's border and its cards.
const FRAME_Y: u16 = 1;
/// Columns between two lanes left of the rows.
const LANE_STEP: u16 = 2;

pub(super) fn place(builder: &mut Builder<'_>) {
    let units = units(builder);
    fit_expanded_card(builder, &units, 0);
    let (mut rows, mut placed, mut links, mut lanes) = arrange(builder, &units);
    let crossing = expanded_row(builder, &units, &placed)
        .map_or(0, |row| crossing_lanes(&links, &placed, row));
    if crossing > 0 {
        // The lanes passing the expanded card take room it had.
        fit_expanded_card(builder, &units, LANE_STEP * crossing);
        (rows, placed, links, lanes) = arrange(builder, &units);
    }
    // A lane runs beside the rows between its ends only, so the expanded
    // card's row, when none passes it, keeps the viewport's whole width.
    let free_row = expanded_row(builder, &units, &placed)
        .filter(|row| crossing_lanes(&links, &placed, *row) == 0);
    let admin_rows = admin_rows(builder);
    let measured = placed.clone();
    let room = room(builder, &placed, &admin_rows, MARGIN);
    center_rows(&mut placed, MARGIN, room);
    let lane_rows = placed
        .iter()
        .filter(|spot| Some(spot.row) != free_row)
        .cloned()
        .collect::<Vec<_>>();
    let shift = lane_shift(&lane_rows, &admin_rows, builder, room, lanes);
    let left = MARGIN + shift;
    // Center again right of the lanes, so the rows keep within the viewport.
    let room = if shift > 0 {
        placed = measured.clone();
        let room = self::room(builder, &placed, &admin_rows, left);
        center_rows(&mut placed, left, room);
        if let Some(row) = free_row {
            center_free_row(builder, &mut placed, &measured, row);
        }
        room
    } else {
        room
    };
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

type Arrangement = (Vec<Vec<usize>>, Vec<Placed>, Vec<Link>, u16);

fn arrange(builder: &Builder<'_>, units: &[Unit]) -> Arrangement {
    let rows = rows(builder, units, &layers(builder, units));
    let placed = measure(builder, units, &rows);
    let links = links(units, &placed);
    let lanes = links
        .iter()
        .filter_map(|link| link.lane)
        .max()
        .map_or(0, |lane| lane + 1);
    (rows, placed, links, lanes)
}

/// Narrow the expanded card so that it and its unit's frame fit across the
/// viewport, `reserve` columns aside; its partners step aside meanwhile.
fn fit_expanded_card(builder: &mut Builder<'_>, units: &[Unit], reserve: u16) {
    let (Some((width, _)), Some(expanded)) = (builder.viewport, builder.size.expanded_key.clone())
    else {
        return;
    };
    let available = width.saturating_sub(2 * MARGIN + reserve);
    let in_unit = units
        .iter()
        .any(|unit| member_keys(builder.view, unit).contains(&expanded));
    let frame = if in_unit { 2 * FRAME_X } else { 0 };
    builder
        .size
        .limit_expanded_width(available.saturating_sub(frame));
}

/// The row of the unit holding the expanded card.
fn expanded_row(builder: &Builder<'_>, units: &[Unit], placed: &[Placed]) -> Option<usize> {
    let expanded = builder.size.expanded_key.as_deref()?;
    placed
        .iter()
        .find(|spot| {
            member_keys(builder.view, &units[spot.unit])
                .iter()
                .any(|key| key == expanded)
        })
        .map(|spot| spot.row)
}

/// Lanes beside `row`: a lane runs from the gap below its parent's row to
/// the gap above its child's, so it passes only the rows strictly between.
fn crossing_lanes(links: &[Link], placed: &[Placed], row: usize) -> u16 {
    let row_of = |unit: usize| {
        placed
            .iter()
            .find(|spot| spot.unit == unit)
            .map(|spot| spot.row)
    };
    links
        .iter()
        .filter_map(|link| {
            let lane = link.lane?;
            let (from, to) = (row_of(link.from)?, row_of(link.to)?);
            (from.min(to) < row && row < from.max(to)).then_some(lane + 1)
        })
        .max()
        .unwrap_or(0)
}

/// Center `row` across the viewport from the left margin, ignoring the lanes
/// the other rows step aside for.
fn center_free_row(builder: &Builder<'_>, placed: &mut [Placed], measured: &[Placed], row: usize) {
    let Some((width, _)) = builder.viewport else {
        return;
    };
    let right = measured
        .iter()
        .filter(|spot| spot.row == row)
        .map(|spot| spot.x + spot.width)
        .max()
        .unwrap_or(0);
    let room = width.saturating_sub(2 * MARGIN).max(right);
    for (spot, original) in placed.iter_mut().zip(measured) {
        if spot.row == row {
            spot.x = MARGIN + (room - right) / 2 + original.x;
        }
    }
}

fn lane_shift(
    placed: &[Placed],
    admin_rows: &[Vec<&str>],
    builder: &Builder<'_>,
    room: u16,
    lanes: u16,
) -> u16 {
    let lanes_end = MARGIN + LANE_STEP * lanes;
    let leftmost = placed
        .iter()
        .map(|spot| spot.x)
        .chain(
            admin_rows
                .iter()
                .map(|row| MARGIN + (room - admin_width(&builder.size, row)) / 2),
        )
        .min()
        .unwrap_or(lanes_end);
    lanes_end.saturating_sub(leftmost)
}
