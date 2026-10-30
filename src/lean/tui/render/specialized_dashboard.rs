use super::super::*;
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};

struct DashboardRows {
    columns: [Vec<Line<'static>>; 2],
    starts: Vec<(usize, usize, usize)>,
    total: usize,
    two_columns: bool,
}

impl Details {
    pub(super) fn draw_dashboard(
        &mut self,
        frame: &mut Frame,
        view: &View,
        area: Rect,
        theme: &Theme,
    ) {
        self.text_area = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
        let (two, column_width) = dashboard_geometry(area);
        let rows = self.dashboard_rows(view, theme, two, column_width);
        let height = usize::from(area.height);
        self.reveal_dashboard_focus(&rows.starts, height, rows.total);
        let areas = dashboard_areas(area, two, column_width);
        self.paint_dashboard(frame, &rows, &areas, two, height);
        self.paint_dashboard_scrollbar(frame, area, theme, rows.total, height);
    }

    fn dashboard_rows(
        &mut self,
        view: &View,
        theme: &Theme,
        two: bool,
        width: usize,
    ) -> DashboardRows {
        let sections = self.sections(view, theme, width.saturating_sub(2).max(1));
        self.dash_focus = self.dash_focus.min(sections.len().saturating_sub(1));
        let order = dashboard_order(&sections, two);
        self.dash_ids = order.iter().map(|index| sections[*index].id).collect();
        let focus = self.dash_ids.get(self.dash_focus).copied();
        let mut rows = DashboardRows {
            columns: [Vec::new(), Vec::new()],
            starts: Vec::new(),
            total: 0,
            two_columns: two,
        };
        for index in order {
            append_dashboard_section(self, &sections, index, focus, theme, &mut rows);
        }
        rows.total = rows.columns[0].len().max(rows.columns[1].len());
        rows
    }

    fn reveal_dashboard_focus(
        &mut self,
        starts: &[(usize, usize, usize)],
        height: usize,
        total: usize,
    ) {
        let bottom = total.saturating_sub(height);
        if std::mem::take(&mut self.dash_reveal)
            && let Some(&(_, start, len)) = starts.get(self.dash_focus)
        {
            self.dash_scroll = if start < self.dash_scroll {
                start
            } else if start + len > self.dash_scroll + height {
                (start + len).saturating_sub(height).min(start)
            } else {
                self.dash_scroll
            };
        }
        self.dash_scroll = self.dash_scroll.min(bottom);
    }

    fn paint_dashboard(
        &self,
        frame: &mut Frame,
        rows: &DashboardRows,
        areas: &[Rect],
        two: bool,
        height: usize,
    ) {
        for (column, lines) in rows.columns.iter().enumerate() {
            if column == 1 && !two {
                break;
            }
            let area = areas[if column == 1 { 2 } else { 0 }];
            let end = (self.dash_scroll + height).min(lines.len());
            let from = self.dash_scroll.min(end);
            frame.render_widget(Paragraph::new(lines[from..end].to_vec()), area);
        }
    }

    fn paint_dashboard_scrollbar(
        &self,
        frame: &mut Frame,
        area: Rect,
        theme: &Theme,
        total: usize,
        height: usize,
    ) {
        if total <= height || area.width == 0 {
            return;
        }
        let bottom = total.saturating_sub(height);
        let mut state = ScrollbarState::new(bottom.saturating_add(1))
            .position(self.dash_scroll)
            .viewport_content_length(height);
        let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some(theme.glyphs().track))
            .thumb_symbol(theme.glyphs().thumb)
            .track_style(theme.faint())
            .thumb_style(theme.dim());
        frame.render_stateful_widget(bar, area, &mut state);
    }
}

fn dashboard_geometry(area: Rect) -> (bool, usize) {
    let two = area.width >= super::super::dashboard::TWO_COLUMNS;
    let usable = usize::from(area.width.saturating_sub(1));
    let width = if two {
        usable.saturating_sub(usize::from(super::super::dashboard::GAP)) / 2
    } else {
        usable
    };
    (two, width)
}

fn dashboard_order(sections: &[super::super::dashboard::Section], two: bool) -> Vec<usize> {
    let mut order: Vec<usize> = (0..sections.len()).collect();
    if two {
        order.sort_by_key(|index| sections[*index].right);
    }
    order
}

fn append_dashboard_section(
    details: &Details,
    sections: &[super::super::dashboard::Section],
    index: usize,
    focus: Option<&str>,
    theme: &Theme,
    rows: &mut DashboardRows,
) {
    let section = &sections[index];
    let column = usize::from(rows.two_columns && section.right);
    if !rows.columns[column].is_empty() {
        rows.columns[column].extend([Line::raw(""), Line::raw("")]);
    }
    let lines = super::specialized_dashboard_cards::dashboard_card_lines(
        details,
        section,
        focus == Some(section.id),
        theme,
    );
    rows.starts
        .push((column, rows.columns[column].len(), lines.len()));
    rows.columns[column].extend(lines);
}

fn dashboard_areas(area: Rect, two: bool, width: usize) -> Vec<Rect> {
    let usable = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if two {
        Layout::horizontal([
            Constraint::Length(width as u16),
            Constraint::Length(super::super::dashboard::GAP),
            Constraint::Length(width as u16),
        ])
        .split(usable)
        .to_vec()
    } else {
        vec![usable]
    }
}
