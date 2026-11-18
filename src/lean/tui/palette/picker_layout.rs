use ratatui::layout::Rect;

#[derive(Clone, Copy)]
pub(super) struct PickerLayout {
    pub(super) box_area: Rect,
    pub(super) width: u16,
    pub(super) inner: usize,
    pub(super) visible_rows: usize,
    pub(super) start: usize,
    pub(super) header_rows: usize,
}

impl PickerLayout {
    pub(super) fn new(
        area: Rect,
        item_count: usize,
        selected: usize,
        header_rows: usize,
    ) -> Option<Self> {
        if area.width == 0 || area.height == 0 {
            return None;
        }
        let width = if area.width >= 8 {
            area.width.saturating_sub(4).min(64)
        } else {
            area.width
        };
        let rows = item_count.clamp(1, 12) as u16;
        let height = (rows + header_rows as u16 + 2).min(area.height).max(1);
        let box_area = Rect::new(
            area.x + (area.width.saturating_sub(width)) / 2,
            area.y + area.height.saturating_sub(height) / 3,
            width,
            height,
        );
        let visible_rows = usize::from(height.saturating_sub(header_rows as u16 + 2)).max(1);
        let start = selected.saturating_sub(visible_rows.saturating_sub(1));
        Some(Self {
            box_area,
            width,
            inner: usize::from(width.saturating_sub(2)),
            visible_rows,
            start,
            header_rows,
        })
    }
}
