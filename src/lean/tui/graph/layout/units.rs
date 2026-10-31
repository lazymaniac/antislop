//! Lay task cards out in dependency rows with routed links.

use super::super::*;
use super::builder::Builder;

mod build;
mod links;
mod model;
mod placement;
mod render;

use build::{card_height, layers, measure, rows, units};
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
    let rows = rows(builder, &units, &layers(builder, &units));
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
    let shift = lane_shift(&placed, &admin_rows, builder, room, lanes);
    placed.iter_mut().for_each(|spot| spot.x += shift);
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
                .map(|row| MARGIN + (room - admin_width(builder.size, row)) / 2),
        )
        .min()
        .unwrap_or(lanes_end);
    lanes_end.saturating_sub(leftmost)
}
