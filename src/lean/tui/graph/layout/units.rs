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
    if lanes > 0 && builder.size.expanded_key.is_some() {
        // The lanes left of the rows take room the expanded card had.
        fit_expanded_card(builder, &units, LANE_STEP * lanes);
        (rows, placed, links, lanes) = arrange(builder, &units);
    }
    let admin_rows = admin_rows(builder);
    let measured = placed.clone();
    let room = room(builder, &placed, &admin_rows, MARGIN);
    center_rows(&mut placed, MARGIN, room);
    let shift = lane_shift(&placed, &admin_rows, builder, room, lanes);
    let left = MARGIN + shift;
    // Center again right of the lanes, so the rows keep within the viewport.
    let room = if shift > 0 {
        placed = measured;
        let room = self::room(builder, &placed, &admin_rows, left);
        center_rows(&mut placed, left, room);
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
    let available = width.saturating_sub(MARGIN + GAP_X + reserve);
    let in_unit = units
        .iter()
        .any(|unit| member_keys(builder.view, unit).contains(&expanded));
    let frame = if in_unit { 2 * FRAME_X } else { 0 };
    builder
        .size
        .limit_expanded_width(available.saturating_sub(frame));
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
