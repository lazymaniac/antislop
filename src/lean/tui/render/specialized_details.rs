use super::super::details::{details_footer_hint_for, format_tabs, tabs_for};
use super::super::*;
use super::specialized_details_layout::*;
use ratatui::widgets::Paragraph;

impl Details {
    pub fn draw(&mut self, frame: &mut Frame, view: &View) {
        self.draw_at_inner(frame, frame.area(), view, None, None, &Theme::from_env());
    }

    pub(super) fn draw_workbench_at(
        &mut self,
        frame: &mut Frame,
        content: Rect,
        terminal: Rect,
        view: &View,
        tab: WorkbenchTab,
        theme: &Theme,
    ) {
        self.draw_at_inner(frame, content, view, Some(terminal), Some(tab), theme);
    }

    fn draw_at_inner(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &View,
        terminal: Option<Rect>,
        workbench_tab: Option<WorkbenchTab>,
        theme: &Theme,
    ) {
        self.normalize_tab();
        if area.width == 0 || area.height == 0 {
            if let Some(terminal) = terminal {
                self.draw_workbench_terminal(frame, terminal, view, theme);
            }
            return;
        }
        if let Some(tab) = workbench_tab {
            self.show_info = tab == WorkbenchTab::Summary;
        }
        let summary = self.summary(view, theme);
        let areas = detail_areas(area, self.overview(), summary.len());
        self.draw_header(frame, &areas, summary, theme, view, workbench_tab);
        let body = self.draw_round_header(frame, areas.body, view, theme);
        if workbench_tab == Some(WorkbenchTab::Conversation) {
            self.draw_conversation(frame, body, areas.footer, view, theme);
            if let Some(terminal) = terminal {
                self.draw_workbench_terminal(frame, terminal, view, theme);
            }
            return;
        }
        if workbench_tab == Some(WorkbenchTab::Summary)
            && self.draw_workbench_summary(frame, body, view, theme)
        {
            if let Some(terminal) = terminal {
                self.draw_workbench_terminal(frame, terminal, view, theme);
            }
            return;
        }
        if self.draw_secondary_screen(frame, body, areas.footer, view, theme, terminal.is_some()) {
            if let Some(terminal) = terminal {
                self.draw_workbench_terminal(frame, terminal, view, theme);
            }
            return;
        }
        self.draw_detail_content(frame, body, areas.footer, view, theme);
        if let Some(terminal) = terminal {
            self.draw_workbench_terminal(frame, terminal, view, theme);
        }
    }

    fn draw_workbench_summary(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &View,
        theme: &Theme,
    ) -> bool {
        let projection = crate::lean::tui::workbench_summary::project(view, &self.key);
        super::specialized_summary::draw(frame, area, projection, &mut self.scroll, theme)
    }

    fn draw_conversation(
        &mut self,
        frame: &mut Frame,
        body: Rect,
        footer: Rect,
        view: &View,
        theme: &Theme,
    ) {
        let inner = self.draw_detail_block(frame, body, theme);
        let width = usize::from(inner.width.saturating_sub(1));
        let records = self.project_dialogue_entries(view);
        let lines = self.layout_cache.update(records, &self.query, width);
        let bottom = lines.len().saturating_sub(usize::from(inner.height));
        self.scroll = self.scroll.min(bottom);
        let end = self
            .scroll
            .saturating_add(usize::from(inner.height))
            .min(lines.len());
        frame.render_widget(
            Paragraph::new(
                lines[self.scroll..end]
                    .iter()
                    .cloned()
                    .map(|line| theme.line(line))
                    .collect::<Vec<_>>(),
            ),
            self.text_area,
        );
        draw_record_scrollbar(frame, inner, lines.len(), self.scroll, theme);
        self.draw_footer_hint(
            frame,
            footer,
            theme.chrome("↑/↓ scroll · Ctrl+F search history · G latest"),
            theme,
        );
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
        view: &View,
        workbench_tab: Option<WorkbenchTab>,
    ) {
        let mut tabs = if let Some(active) = workbench_tab {
            let delivers = self
                .task_id()
                .and_then(|id| view.tasks.iter().find(|task| task.id == id))
                .is_some_and(|task| task.change_kind.has_delivery());
            let labels = |tab| match tab {
                WorkbenchTab::Summary => "Podsumowanie",
                WorkbenchTab::Conversation => "Rozmowa",
                WorkbenchTab::Results => "Wyniki",
                WorkbenchTab::Checks => "Sprawdzenia",
                WorkbenchTab::Comments => "Komentarze",
                WorkbenchTab::Diff => "Diff",
            };
            let mut spans = Vec::new();
            for (index, tab) in super::super::workbench::tabs_for(&self.key, delivers)
                .into_iter()
                .enumerate()
            {
                if index > 0 {
                    spans.push(Span::styled(theme.chrome(" · "), theme.faint()));
                }
                let style = if tab == active {
                    theme.title().add_modifier(Modifier::REVERSED)
                } else {
                    theme.text()
                };
                spans.push(Span::styled(labels(tab), style));
            }
            Line::from(spans)
        } else {
            format_tabs(&self.key, self.tab, usize::from(areas.header.width))
        };
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
            Paragraph::new(theme.chrome(&label)).style(theme.dim()),
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
        external_terminal: bool,
    ) -> bool {
        self.split = false;
        if self.draw_overview_screen(frame, body, footer, view, theme, external_terminal) {
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
        external_terminal: bool,
    ) -> bool {
        if external_terminal && self.overview() {
            self.draw_dashboard(frame, view, body, theme);
            self.draw_footer_hint(
                frame,
                footer,
                overview_split_hint(self.searching, &self.query, footer.width, theme),
                theme,
            );
            return true;
        }
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
            overview_split_hint(self.searching, &self.query, footer.width, theme),
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
                overview_hint(self.searching, &self.query, footer.width, theme),
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
            dashboard_hint(self.searching, &self.query, footer.width, theme),
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
        let Some(key) = document_key(self) else {
            return false;
        };
        let Some(document) = self.documents.get(&key) else {
            if key.starts_with("result_document:") {
                let message = self
                    .result_error
                    .as_ref()
                    .filter(|(failed, _)| failed == &key)
                    .map(|(_, error)| format!("Could not load result document: {error}"))
                    .unwrap_or_else(|| "Loading result document…".into());
                frame.render_widget(
                    Paragraph::new(theme.clean(&message)).style(theme.dim()),
                    self.text_area,
                );
                self.draw_result_document_footer(frame, footer, theme);
                return true;
            }
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
        self.draw_result_document_footer(frame, footer, theme);
        true
    }

    fn draw_result_document_footer(&self, frame: &mut Frame, footer: Rect, theme: &Theme) {
        let mut hint = details_footer_hint_for(&self.key, self.tab, usize::from(footer.width));
        if self.visible_result_keys().len() > 1 {
            hint.push_str(" · ,/. documents");
        }
        self.draw_footer_hint(frame, footer, theme.chrome(&hint), theme);
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
            Paragraph::new(
                lines[self.scroll..end]
                    .iter()
                    .cloned()
                    .map(|line| theme.line(line))
                    .collect::<Vec<_>>(),
            ),
            self.text_area,
        );
        draw_record_scrollbar(frame, inner, lines.len(), self.scroll, theme);
        self.draw_footer_hint(
            frame,
            footer,
            records_hint(self, footer.width, theme),
            theme,
        );
    }
}
