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
        let width = super::super::list_area(area)
            .width
            .saturating_add(2)
            .min(area.width);
        let available_height = if area.height >= 10 {
            area.height - 4
        } else {
            area.height
        };
        let visible_rows = usize::from(available_height.saturating_sub(header_rows as u16 + 3))
            .min(item_count.max(1))
            .clamp(1, 8);
        let height = (header_rows as u16 + visible_rows as u16 + 3).min(available_height);
        let box_area = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
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
