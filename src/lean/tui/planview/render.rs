use super::*;
mod buttons;
mod rows;

impl PlanView {
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, view: &View, theme: &Theme) {
        self.draw_with_history(frame, area, view, theme, None);
    }

    pub(in crate::lean::tui) fn draw_history(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &View,
        theme: &Theme,
        navigation: &str,
    ) {
        self.draw_with_history(frame, area, view, theme, Some(navigation));
    }

    fn draw_with_history(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &View,
        theme: &Theme,
        history_navigation: Option<&str>,
    ) {
        self.doc_area = Rect::default();
        self.row_docs.clear();
        self.task_rects.clear();
        self.task_area = Rect::default();
        self.button_rects = [Rect::default(); 4];
        frame.render_widget(Clear, area);
        frame.render_widget(Block::default().style(theme.overlay()), area);
        if let Some(navigation) = history_navigation
            && (area.width < 14 || area.height < 8)
        {
            self.graph_area = Rect::default();
            let inner = area;
            let [status, document] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(inner);
            self.draw_history_navigation(frame, status, navigation, theme);
            self.draw_document(frame, document, view, theme);
            return;
        }
        let Some(chunks) =
            self.frame_layout(frame, area, view, theme, history_navigation.is_some())
        else {
            self.graph_area = Rect::default();
            return;
        };
        let open_comments = view.open_plan_comments();
        self.draw_banner(
            frame,
            chunks[0],
            view,
            theme,
            &open_comments,
            history_navigation.is_some(),
        );
        if view.tasks.is_empty() {
            self.graph_area = Rect::default();
            self.graph_focused = false;
            self.draw_document(frame, chunks[1], view, theme);
        } else {
            // A tall narrow screen, as a phone's, stacks the panes rather
            // than squeezing the description beside the graph.
            let side_by_side =
                chunks[1].width >= 100 || (chunks[1].width >= 60 && chunks[1].height < 30);
            let [description, tasks] = if !side_by_side && chunks[1].height < 10 {
                if self.graph_focused {
                    [Rect::default(), chunks[1]]
                } else {
                    [chunks[1], Rect::default()]
                }
            } else if side_by_side {
                let [description, _, tasks] = Layout::horizontal([
                    Constraint::Percentage(40),
                    Constraint::Length(3),
                    Constraint::Min(1),
                ])
                .areas(chunks[1]);
                [description, tasks]
            } else {
                // A short description leaves the rest to the graph.
                let lines = self
                    .description_lines(view)
                    .len()
                    .saturating_add(3)
                    .min(usize::from(u16::MAX)) as u16;
                let height = lines.clamp(3, (chunks[1].height * 2 / 5).max(3));
                Layout::vertical([Constraint::Length(height), Constraint::Min(1)]).areas(chunks[1])
            };
            let block = graph::panel(" Description (v) ", !self.graph_focused, theme);
            let inner = block.inner(description).inner(ratatui::layout::Margin::new(
                u16::from(description.width >= 24),
                0,
            ));
            frame.render_widget(block, description);
            self.draw_document(frame, inner, view, theme);
            self.draw_task_graph(frame, tasks, view, theme, history_navigation.is_some());
        }
        if let Some(navigation) = history_navigation {
            self.draw_history_navigation(frame, chunks[2], navigation, theme);
        } else {
            self.draw_buttons(frame, chunks[2], view, theme);
        }
    }

    fn frame_layout(
        &self,
        frame: &mut Frame,
        area: Rect,
        view: &View,
        theme: &Theme,
        history: bool,
    ) -> Option<[Rect; 3]> {
        let inner = crate::lean::tui::render::chrome::inset(area);
        let heading = if history { "Plan History" } else { "Plan" };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(heading, theme.text().add_modifier(Modifier::BOLD)),
                Span::styled(
                    if history {
                        String::new()
                    } else {
                        format!("  /  {}", view.rigor_label())
                    },
                    theme.dim(),
                ),
            ])),
            Rect::new(inner.x, inner.y, inner.width, inner.height.min(1)),
        );
        if inner.height < 6 || inner.width < 14 {
            return None;
        }
        let has_banner = !view.open_plan_comments().is_empty() || view.comment_activity().is_some();
        let header_height = if inner.height >= 20 { 2 } else { 1 };
        let body = Rect::new(
            inner.x,
            inner.y + header_height,
            inner.width,
            inner.height - header_height,
        );
        let [banner, _, content, footer] = Layout::vertical([
            Constraint::Length(if has_banner { 3 } else { 1 }),
            Constraint::Length(u16::from(inner.height >= 16)),
            Constraint::Min(1),
            Constraint::Length(if inner.height >= 12 { 3 } else { 2 }),
        ])
        .areas(body);
        Some([banner, content, footer])
    }

    fn draw_banner(
        &self,
        frame: &mut Frame,
        area: Rect,
        view: &View,
        theme: &Theme,
        open_comments: &[&super::super::view::Comment],
        history: bool,
    ) {
        if !open_comments.is_empty() || view.comment_activity().is_some() {
            frame.render_widget(
                Paragraph::new(self.comment_banner(view, theme, open_comments, area.width)),
                area,
            );
        } else {
            frame.render_widget(
                Paragraph::new(self.task_banner(view, theme, area.width, history)),
                area,
            );
        }
    }

    fn comment_banner(
        &self,
        view: &View,
        theme: &Theme,
        open_comments: &[&super::super::view::Comment],
        width: u16,
    ) -> Vec<Line<'static>> {
        let status = view.comment_activity().unwrap_or_else(|| {
            open_comments.first().map_or_else(String::new, |comment| {
                format!(
                    "Comment #{}: {}",
                    comment.id,
                    view.comment_state(comment.id)
                )
            })
        });
        let drafts = view.draft_plan_comments().len();
        let sent = open_comments.len() - drafts;
        let notes = self.comment_notes(theme, drafts, sent);
        vec![
            Line::styled(
                theme.fit(&theme.clean(&status), usize::from(width)),
                theme.accent().add_modifier(Modifier::BOLD),
            ),
            Line::from(notes),
            Line::styled(
                format!(
                    " {}",
                    theme.chrome(&fit_hint(
                        &[
                            "R review drafts",
                            "D delete draft",
                            "r resolve",
                            "o retry/reopen",
                        ],
                        usize::from(width.saturating_sub(1)),
                        true,
                    ))
                ),
                theme.dim(),
            ),
        ]
    }

    fn comment_notes(&self, theme: &Theme, drafts: usize, sent: usize) -> Vec<Span<'static>> {
        let mut notes = Vec::new();
        if drafts > 0 {
            notes.push(Span::styled(
                format!("{drafts} draft comment(s)"),
                theme.accent(),
            ));
        }
        if sent > 0 {
            if drafts > 0 {
                notes.push(Span::styled(theme.chrome(" · "), theme.dim()));
            }
            notes.push(Span::styled(
                format!("{sent} unresolved comment(s)"),
                theme.warning(),
            ));
        }

        notes
    }

    fn task_banner(&self, view: &View, theme: &Theme, width: u16, history: bool) -> Line<'static> {
        let completed = view
            .tasks
            .iter()
            .filter(|task| task.status == "approved")
            .count();
        let state = if history {
            "read only"
        } else if view.plan_proposed() {
            "review before execution"
        } else {
            "execution progress"
        };
        Line::styled(
            theme.fit(
                &theme.chrome(&format!(
                    "{completed}/{} tasks completed · {state}",
                    view.tasks.len()
                )),
                usize::from(width),
            ),
            theme.dim(),
        )
    }

    fn draw_document(&mut self, frame: &mut Frame, doc_rect: Rect, view: &View, theme: &Theme) {
        self.doc_area = doc_rect;
        if doc_rect.is_empty() {
            self.row_docs.clear();
            return;
        }
        let doc_lines = self.description_lines(view);
        let max_cursor = doc_lines.len().saturating_sub(1);
        self.cursor = self.cursor.min(max_cursor);

        let (rows, cursor_rows) = self.layout_rows(view, &doc_lines, doc_rect.width, theme);
        let visible_h = usize::from(doc_rect.height);
        self.keep_cursor_visible(cursor_rows, visible_h, rows.len());
        self.row_docs = rows.iter().map(|row| row.doc).collect();
        let rendered: Vec<Line<'static>> = rows
            .into_iter()
            .skip(self.scroll)
            .take(visible_h)
            .map(|row| row.line)
            .collect();
        frame.render_widget(Paragraph::new(rendered), doc_rect);
    }

    fn keep_cursor_visible(
        &mut self,
        (first, last): (usize, usize),
        height: usize,
        row_count: usize,
    ) {
        if first < self.scroll {
            self.scroll = first;
        } else if height > 0 && last >= self.scroll + height {
            self.scroll = (last + 1 - height).min(first);
        }
        self.scroll = self.scroll.min(row_count.saturating_sub(height.max(1)));
    }

    fn draw_history_navigation(
        &self,
        frame: &mut Frame,
        area: Rect,
        navigation: &str,
        theme: &Theme,
    ) {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    theme.fit(&theme.clean(navigation), usize::from(area.width)),
                    theme.accent(),
                ),
                Line::styled(
                    theme.chrome(&fit_hint(
                        &[
                            "[ older",
                            "] newer",
                            "v pane",
                            "arrows navigate",
                            "Esc close",
                        ],
                        usize::from(area.width),
                        true,
                    )),
                    theme.dim(),
                ),
            ]),
            Rect::new(
                area.x,
                area.bottom().saturating_sub(2).max(area.y),
                area.width,
                area.height.min(2),
            ),
        );
    }
}
