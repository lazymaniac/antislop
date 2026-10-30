use super::super::details::{details_footer_hint_for, format_tabs, tabs_for};
use super::super::*;
use super::specialized_details_layout::*;
use ratatui::widgets::Paragraph;

impl Details {
    pub fn draw(&mut self, frame: &mut Frame, view: &View) {
        self.normalize_tab();
        let area = frame.area();
        if area.width == 0 || area.height == 0 {
            return;
        }
        let theme = Theme::from_env();
        let summary = self.summary(view, &theme);
        let areas = detail_areas(area, self.overview(), summary.len());
        self.draw_header(frame, &areas, summary, &theme);
        let body = self.draw_round_header(frame, areas.body, view, &theme);
        if self.draw_secondary_screen(frame, body, areas.footer, view, &theme) {
            return;
        }
        self.draw_detail_content(frame, body, areas.footer, view, &theme);
    }

    fn normalize_tab(&mut self) {
        if !tabs_for(&self.key).contains(&self.tab) {
            self.tab = 0;
        }
    }

    fn draw_header(
        &self,
        frame: &mut Frame,
        areas: &DetailAreas,
        summary: Vec<Line<'static>>,
        theme: &Theme,
    ) {
        let mut tabs = format_tabs(&self.key, self.tab, usize::from(areas.header.width));
        if !self.query.is_empty() {
            tabs.spans.push(Span::styled(
                format!(" [🔍 \"{}\"]", self.query),
                Style::default().fg(super::super::theme::ayu::BLUE),
            ));
        }
        clean_header_spans(&mut tabs.spans, theme);
        frame.render_widget(Paragraph::new(tabs), areas.header);
        frame.render_widget(Paragraph::new(summary).style(theme.bar()), areas.summary);
    }

    fn draw_round_header(
        &self,
        frame: &mut Frame,
        mut body: Rect,
        view: &View,
        theme: &Theme,
    ) -> Rect {
        if body.height <= 1 || self.key == "conversation" {
            return body;
        }
        let latest = latest_round(self, view);
        let label = round_label(self, self.selected_round.unwrap_or(latest));
        frame.render_widget(
            Paragraph::new(theme.clean(&label)).style(theme.dim()),
            Rect::new(body.x, body.y, body.width, 1),
        );
        body.y = body.y.saturating_add(1);
        body.height = body.height.saturating_sub(1);
        body
    }

    fn draw_secondary_screen(
        &mut self,
        frame: &mut Frame,
        body: Rect,
        footer: Rect,
        view: &View,
        theme: &Theme,
    ) -> bool {
        self.split = false;
        if self.draw_overview_screen(frame, body, footer, view, theme) {
            return true;
        }
        self.draw_dashboard_screen(frame, body, footer, view, theme)
    }

    fn draw_overview_screen(
        &mut self,
        frame: &mut Frame,
        body: Rect,
        footer: Rect,
        view: &View,
        theme: &Theme,
    ) -> bool {
        if !self.overview() || self.key == "conversation" || !self.query.is_empty() {
            return false;
        }
        if self.task_id().is_some()
            && let Some([info, terminal]) = overview_split(body)
        {
            return self.draw_split_overview(frame, view, info, terminal, footer, theme);
        }
        self.draw_single_overview(frame, body, footer, view, theme)
    }

    fn draw_split_overview(
        &mut self,
        frame: &mut Frame,
        view: &View,
        info: Rect,
        terminal: Rect,
        footer: Rect,
        theme: &Theme,
    ) -> bool {
        self.split = true;
        self.draw_dashboard(frame, view, info, theme);
        self.draw_overview(frame, view, terminal, theme);
        self.draw_footer_hint(
            frame,
            footer,
            overview_split_hint(self.searching, &self.query, footer.width),
            theme,
        );
        true
    }

    fn draw_single_overview(
        &mut self,
        frame: &mut Frame,
        body: Rect,
        footer: Rect,
        view: &View,
        theme: &Theme,
    ) -> bool {
        if !self.show_info {
            self.draw_overview(frame, view, body, theme);
            self.draw_footer_hint(
                frame,
                footer,
                overview_hint(self.searching, &self.query),
                theme,
            );
            return true;
        }
        false
    }

    fn draw_dashboard_screen(
        &mut self,
        frame: &mut Frame,
        body: Rect,
        footer: Rect,
        view: &View,
        theme: &Theme,
    ) -> bool {
        if !self.dashboard_visible() || body.height < 4 || body.width < 24 {
            return false;
        }
        self.draw_dashboard(frame, view, body, theme);
        self.draw_footer_hint(
            frame,
            footer,
            dashboard_hint(self.searching, &self.query, footer.width),
            theme,
        );
        true
    }

    fn draw_footer_hint(&self, frame: &mut Frame, footer: Rect, hint: String, theme: &Theme) {
        frame.render_widget(
            Paragraph::new(theme.clean(&hint)).style(theme.dim()),
            footer,
        );
    }

    fn draw_detail_content(
        &mut self,
        frame: &mut Frame,
        body: Rect,
        footer: Rect,
        view: &View,
        theme: &Theme,
    ) {
        let inner = self.draw_detail_block(frame, body, theme);
        let width = usize::from(inner.width.saturating_sub(1));
        if self.draw_document(frame, inner, footer, width, theme) {
            return;
        }
        self.draw_records(frame, inner, footer, view, width, theme);
    }

    fn draw_detail_block(&mut self, frame: &mut Frame, body: Rect, theme: &Theme) -> Rect {
        let block = detail_block(self, body, theme);
        let inner = block.inner(body);
        frame.render_widget(block, body);
        self.text_area = Rect::new(
            inner.x,
            inner.y,
            inner.width.saturating_sub(1),
            inner.height,
        );
        inner
    }

    fn draw_document(
        &mut self,
        frame: &mut Frame,
        inner: Rect,
        footer: Rect,
        width: usize,
        theme: &Theme,
    ) -> bool {
        let Some(key) = document_key(self.tab) else {
            return false;
        };
        let Some(document) = self.documents.get(key) else {
            return false;
        };
        let result = document.viewport_filtered(
            width,
            usize::from(inner.height),
            self.scroll,
            false,
            &self.query,
        );
        self.draw_document_result(frame, inner, result, theme);
        let hint = details_footer_hint_for(&self.key, self.tab, usize::from(footer.width));
        self.draw_footer_hint(frame, footer, hint, theme);
        true
    }

    fn draw_document_result(
        &mut self,
        frame: &mut Frame,
        inner: Rect,
        result: crate::Result<Option<super::super::document::Viewport>>,
        theme: &Theme,
    ) {
        match result {
            Ok(Some(page)) => self.draw_document_page(frame, inner, page, theme),
            Ok(None) => frame.render_widget(
                Paragraph::new("Preparing history view...").style(theme.dim()),
                self.text_area,
            ),
            Err(error) => frame.render_widget(
                Paragraph::new(theme.clean(&error.to_string())).style(theme.dim()),
                self.text_area,
            ),
        }
    }

    fn draw_document_page(
        &mut self,
        frame: &mut Frame,
        inner: Rect,
        page: super::super::document::Viewport,
        theme: &Theme,
    ) {
        self.scroll = page.scroll;
        let lines = page
            .lines
            .into_iter()
            .map(|(_, text)| Line::raw(theme.clean(&text)))
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), self.text_area);
        draw_document_scrollbar(frame, inner, page.total, page.scroll, theme);
    }

    fn draw_records(
        &mut self,
        frame: &mut Frame,
        inner: Rect,
        footer: Rect,
        view: &View,
        width: usize,
        theme: &Theme,
    ) {
        let records = self.records(view);
        let lines = self.layout_cache.update(records, &self.query, width);
        position_record_scroll(
            lines,
            self.tab,
            self.show_info,
            &mut self.follow,
            &mut self.scroll,
            &mut self.reveal_check,
            usize::from(inner.height),
        );
        let end = (self.scroll + usize::from(inner.height)).min(lines.len());
        frame.render_widget(
            Paragraph::new(lines[self.scroll..end].to_vec()),
            self.text_area,
        );
        draw_record_scrollbar(frame, inner, lines.len(), self.scroll, theme);
        self.draw_footer_hint(frame, footer, records_hint(self, footer.width), theme);
    }
}
