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
        let content = super::super::list_area(area);
        let width = content.width.saturating_add(2).min(area.width);
        let box_area = Rect::new(
            content.x.saturating_sub(1).max(area.x),
            content.y.saturating_sub(1).max(area.y),
            width,
            content.height.saturating_add(2).min(area.height),
        );
        let height = box_area.height;
        let visible_rows = usize::from(height.saturating_sub(header_rows as u16 + 3))
            .min(item_count.max(1))
            .max(1);
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
