//! Lay task cards out in dependency rows centred across the viewport, with
//! the links into each unit gathered on one bus and the links that skip
//! rows routed in lanes on either side of the rows.

use super::super::*;
use super::builder::Builder;

mod build;
mod links;
mod model;
mod placement;
mod render;

use build::{card_height, layers, measure, member_keys, rows, units};
use links::{buses, crossed, gap_lines, lane_reserve, links, place_lanes};
use model::{Bus, Lane, Link, Placed, Side, Tops, Unit};
use placement::{admin_rows, admin_width, center_rows, row_tops};
use render::{Wiring, entry_columns, push_admin, push_hidden, push_links, push_unit};

/// Rows between a frame's border and its cards.
const FRAME_Y: u16 = 1;
/// Columns between two lanes beside the rows.
const LANE_STEP: u16 = 2;
/// Layouts tried while making room for the lanes, which may need more.
const ATTEMPTS: usize = 4;

pub(super) fn place(builder: &mut Builder<'_>) {
    let units = units(builder);
    let Arrangement {
        rows,
        mut placed,
        links,
        mut buses,
    } = settle(builder, &units);
    let admin_rows = admin_rows(builder);
    let width = layout_width(builder, &placed, &buses, &admin_rows);
    center_rows(&mut placed, width);
    place_lanes(&mut buses, &placed);
    let entries = entry_columns(builder, &units, &placed, &buses);
    let gaps = gap_lines(&links, &buses, &placed, rows.len(), &entries);
    let tops = row_tops(builder, &admin_rows, &placed, &gaps, rows.len());
    push_admin(builder, &admin_rows, &tops.admin, width);
    for spot in &placed {
        push_unit(builder, &units[spot.unit], spot, tops.units[spot.row]);
    }
    push_hidden(builder, &units, &placed);
    let wiring = Wiring {
        links: &links,
        buses: &buses,
        gaps: &gaps,
        tops: &tops,
        entries: &entries,
    };
    push_links(builder, &units, &placed, &wiring);
    builder.graph.width = width;
    builder.graph.height = tops.bottom;
}

struct Arrangement {
    rows: Vec<Vec<usize>>,
    placed: Vec<Placed>,
    links: Vec<Link>,
    buses: Vec<Bus>,
}

/// Arrange the units, then again with room kept for the lanes on both
/// sides of the rows, until the lanes fit the room they have.
fn settle(builder: &mut Builder<'_>, units: &[Unit]) -> Arrangement {
    let (mut reserve, mut narrowed) = (0, false);
    let mut attempt = 0;
    loop {
        resize(builder, units, reserve, narrowed);
        let arrangement = arrange(builder, units);
        let need = lane_reserve(&arrangement.buses);
        // The lanes passing the expanded card take room it had.
        let crossing = expanded_row(builder, units, &arrangement.placed)
            .is_some_and(|row| crossed(&arrangement.buses, row));
        attempt += 1;
        if attempt == ATTEMPTS || (need <= reserve && (narrowed || !crossing)) {
            return arrangement;
        }
        reserve = reserve.max(need);
        narrowed |= crossing;
    }
}

fn arrange(builder: &Builder<'_>, units: &[Unit]) -> Arrangement {
    let rows = rows(builder, units, &layers(builder, units));
    let placed = measure(builder, units, &rows);
    let links = links(units, &placed);
    let buses = buses(&links, &placed);
    Arrangement {
        rows,
        placed,
        links,
        buses,
    }
}

/// Size the cards for `reserve` columns kept clear on either side of the
/// rows; the expanded card narrows too when a lane passes its row.
fn resize(builder: &mut Builder<'_>, units: &[Unit], reserve: u16, narrowed: bool) {
    builder.size =
        Size::fit(builder.viewport, builder.size.expanded_key.clone()).animated(builder.step);
    builder.lanes = reserve;
    if let (Some((width, _)), true) = (builder.viewport, reserve > 0) {
        builder.size.reserve_columns(width, 2 * reserve);
    }
    let extra = if narrowed {
        2 * reserve.saturating_sub(MARGIN)
    } else {
        0
    };
    fit_expanded_card(builder, units, extra);
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

/// The width the rows are centred in: the viewport's, so the graph never
/// scrolls sideways, unless a row and the lanes beside it cannot fit.
/// Without a viewport, the widest row with its margins.
fn layout_width(
    builder: &Builder<'_>,
    placed: &[Placed],
    buses: &[Bus],
    admin: &[Vec<&str>],
) -> u16 {
    let reserve = lane_reserve(buses);
    let mut rows = std::collections::BTreeMap::<usize, u16>::new();
    for spot in placed {
        let right = rows.entry(spot.row).or_default();
        *right = (*right).max(spot.x + spot.width);
    }
    let margin = |row: usize| {
        if crossed(buses, row) {
            reserve
        } else if builder.viewport.is_some() {
            0
        } else {
            MARGIN
        }
    };
    let needed = rows
        .iter()
        .map(|(&row, &width)| width + 2 * margin(row))
        .chain(admin.iter().map(|row| {
            admin_width(&builder.size, row)
                + if builder.viewport.is_some() {
                    0
                } else {
                    2 * MARGIN
                }
        }))
        .max()
        .unwrap_or(0);
    builder
        .viewport
        .map_or(needed, |(width, _)| width.max(needed))
}
