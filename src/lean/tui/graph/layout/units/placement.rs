use super::links::gap_height;
use super::*;
use std::collections::BTreeMap;

/// Preparation and final review occupy separate rows, in workflow order.
pub(super) fn admin_rows<'a>(builder: &Builder<'a>) -> Vec<Vec<&'a str>> {
    let visible: Vec<&str> = builder
        .admin
        .iter()
        .filter(|(_, visible)| *visible)
        .map(|(key, _)| *key)
        .collect();
    let max_width = visible
        .iter()
        .map(|key| builder.size.width(key, true))
        .max()
        .unwrap_or(builder.size.admin_w);
    let across = builder.viewport.map_or(visible.len().max(1), |(width, _)| {
        usize::from(width.saturating_sub(MARGIN) / (max_width + GAP_X)).max(1)
    });
    let mut rows: Vec<Vec<&str>> = Vec::new();
    for key in visible {
        if let Some(row) = rows.last_mut()
            && row.len() < across
            && is_final_review(row[0]) == is_final_review(key)
        {
            row.push(key);
        } else {
            rows.push(vec![key]);
        }
    }
    rows
}

pub(super) fn admin_width(size: &Size, row: &[&str]) -> u16 {
    row.iter().map(|key| size.width(key, true)).sum::<u16>()
        + (row.len() as u16).saturating_sub(1) * GAP_X
}

/// The width every row is centered in, right of the lanes: the viewport's,
/// or the widest row's when that is wider.
pub(super) fn room(
    builder: &Builder<'_>,
    placed: &[Placed],
    admin: &[Vec<&str>],
    left: u16,
) -> u16 {
    let widest = placed
        .iter()
        .map(|spot| spot.x + spot.width)
        .chain(admin.iter().map(|row| admin_width(&builder.size, row)))
        .max()
        .unwrap_or(0);
    builder.viewport.map_or(widest, |(width, _)| {
        width.saturating_sub(left + GAP_X).max(widest)
    })
}

pub(super) fn center_rows(placed: &mut [Placed], left: u16, room: u16) {
    let mut widths = BTreeMap::<usize, u16>::new();
    for spot in placed.iter() {
        widths.insert(spot.row, spot.x + spot.width);
    }
    for spot in placed.iter_mut() {
        spot.x += left + (room - widths[&spot.row]) / 2;
    }
}

pub(super) fn row_tops(
    builder: &Builder<'_>,
    admin: &[Vec<&str>],
    placed: &[Placed],
    gaps: &[Vec<usize>],
    rows: usize,
) -> Tops {
    let split = admin.partition_point(|row| !is_final_review(row[0]));
    let (mut admin_tops, y) = admin_row_tops(builder, &admin[..split], 0);
    let heights = row_heights(placed, rows);
    let (unit_tops, gap_tops, mut bottom) = content_tops(&heights, gaps, split == 0, y);
    if split < admin.len() && (split > 0 || rows > 0) {
        bottom += GAP_Y;
    }
    let (review_tops, bottom) = admin_row_tops(builder, &admin[split..], bottom);
    admin_tops.extend(review_tops);
    Tops {
        admin: admin_tops,
        units: unit_tops,
        gaps: gap_tops,
        bottom,
    }
}

fn admin_row_tops(builder: &Builder<'_>, admin: &[Vec<&str>], mut y: u16) -> (Vec<u16>, u16) {
    let mut tops = Vec::new();
    for (index, row) in admin.iter().enumerate() {
        let height = row
            .iter()
            .map(|key| {
                card_content::height(
                    builder.view,
                    key,
                    builder.size.width(key, true),
                    builder.size.layout_height(key, true),
                    false,
                )
            })
            .max()
            .unwrap_or(0);
        if index > 0 {
            y += GAP_Y;
        }
        tops.push(y);
        y += height;
    }
    (tops, y)
}

fn row_heights(placed: &[Placed], rows: usize) -> Vec<u16> {
    let mut heights = vec![0u16; rows];
    for spot in placed {
        heights[spot.row] = heights[spot.row].max(spot.height);
    }
    heights
}

fn content_tops(
    heights: &[u16],
    gaps: &[Vec<usize>],
    no_admin: bool,
    mut y: u16,
) -> (Vec<u16>, Vec<u16>, u16) {
    let mut units = Vec::new();
    let mut gap_tops = Vec::new();
    for (row, height) in heights.iter().enumerate() {
        gap_tops.push(y);
        y += gap_height(gaps[row].len(), row == 0 && no_admin);
        units.push(y);
        y += height;
    }
    gap_tops.push(y);
    if let Some(below) = gaps.last().filter(|below| !below.is_empty()) {
        y += gap_height(below.len(), false);
    }
    (units, gap_tops, y)
}
