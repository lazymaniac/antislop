use super::*;
mod buttons;
mod rows;

impl PlanView {
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, view: &View, theme: &Theme) {
        self.doc_area = Rect::default();
        self.row_docs.clear();
        self.task_rects.clear();
        self.button_rects = [Rect::default(); 4];
        frame.render_widget(Clear, area);
        let Some(chunks) = self.frame_layout(frame, area, view, theme) else {
            self.graph_area = Rect::default();
            return;
        };
        let open_comments = view.open_plan_comments();
        self.draw_banner(frame, chunks[0], view, theme, &open_comments);
        if view.tasks.is_empty() {
            self.graph_area = Rect::default();
            self.graph_focused = false;
            self.draw_document(frame, chunks[1], view, theme);
        } else {
            // A tall narrow screen, as a phone's, stacks the panes rather
            // than squeezing the description beside the graph.
            let side_by_side =
                chunks[1].width >= 100 || (chunks[1].width >= 60 && chunks[1].height < 30);
            let [description, tasks] = if side_by_side {
                Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
                    .areas(chunks[1])
            } else {
                // A short description leaves the rest to the graph.
                let lines = self.description_lines(view).len() as u16 + 2;
                let height = lines.clamp(3, (chunks[1].height * 2 / 5).max(3));
                Layout::vertical([Constraint::Length(height), Constraint::Min(1)]).areas(chunks[1])
            };
            let block = graph::panel(" Description (v) ", !self.graph_focused, theme);
            let inner = block.inner(description);
            frame.render_widget(block, description);
            self.draw_document(frame, inner, view, theme);
            self.draw_task_graph(frame, tasks, view, theme);
        }
        self.draw_buttons(frame, chunks[2], view, theme);
        // Apply the same terminal fallbacks when this viewer is drawn directly.
        if !theme.color {
            crate::lean::tui::without_color(frame.buffer_mut());
        }
        if theme.ascii {
            crate::lean::tui::in_ascii(frame.buffer_mut());
        }
    }

    fn frame_layout(
        &self,
        frame: &mut Frame,
        area: Rect,
        view: &View,
        theme: &Theme,
    ) -> Option<[Rect; 3]> {
        let block = graph::panel(&format!(" Plan Document ({}) ", self.plan_id), true, theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.height < 4 || inner.width < 12 {
            return None;
        }
        let has_banner = !view.open_plan_comments().is_empty() || view.comment_activity().is_some();
        let constraints = if has_banner {
            [
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(3),
            ]
        } else {
            [
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(3),
            ]
        };
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);
        Some([chunks[0], chunks[1], chunks[2]])
    }

    fn draw_banner(
        &self,
        frame: &mut Frame,
        area: Rect,
        view: &View,
        theme: &Theme,
        open_comments: &[&super::super::view::Comment],
    ) {
        if !open_comments.is_empty() || view.comment_activity().is_some() {
            frame.render_widget(
                Paragraph::new(self.comment_banner(view, theme, open_comments, area.width)),
                area,
            );
        } else {
            frame.render_widget(
                Paragraph::new(self.task_banner(view, theme, area.width)),
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
                format!(" {}", clean(&status)),
                theme.accent().add_modifier(Modifier::BOLD),
            ),
            Line::from(notes),
            Line::styled(
                format!(
                    " {}",
                    fit_hint(
                        &[
                            "↑/↓ move",
                            "c comment line",
                            "R send drafts for review",
                            "D delete draft",
                            "Esc close",
                        ],
                        usize::from(width.saturating_sub(1)),
                        true,
                    )
                ),
                theme.dim(),
            ),
        ]
    }

    fn comment_notes(&self, theme: &Theme, drafts: usize, sent: usize) -> Vec<Span<'static>> {
        let mut notes = Vec::new();
        if drafts > 0 {
            notes.push(Span::styled(
                format!(" ✎ {drafts} draft comment(s) "),
                Style::default()
                    .fg(crate::lean::tui::theme::ayu::BLUE)
                    .add_modifier(Modifier::BOLD),
            ));
            notes.push(Span::styled("— R sends them for review · ", theme.dim()));
        }
        if sent > 0 {
            notes.push(Span::styled(
                format!(" ⚠ {sent} unresolved comment(s) on this plan "),
                Style::default()
                    .fg(crate::lean::tui::theme::ayu::YELLOW)
                    .add_modifier(Modifier::BOLD),
            ));
            notes.push(Span::styled("— r resolve · o retry/reopen", theme.dim()));
        }
        notes
    }

    fn task_banner(&self, view: &View, theme: &Theme, width: u16) -> Line<'static> {
        let completed = view
            .tasks
            .iter()
            .filter(|task| task.status == "approved")
            .count();
        let tasks = format!(" Tasks: {completed}/{} completed ", view.tasks.len());
        Line::from(vec![
            Span::styled(
                tasks.clone(),
                Style::default()
                    .fg(crate::lean::tui::theme::ayu::BLUE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "· {}",
                    fit_hint(
                        &[
                            "v switch pane",
                            "arrows navigate",
                            "Shift+arrows pan graph",
                            "c comment",
                            "Esc close",
                        ],
                        usize::from(width).saturating_sub(tasks.len() + 2),
                        true,
                    )
                ),
                theme.dim(),
            ),
        ])
    }

    fn draw_document(&mut self, frame: &mut Frame, doc_rect: Rect, view: &View, theme: &Theme) {
        self.doc_area = doc_rect;
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
}
