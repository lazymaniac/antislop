use super::*;
mod buttons;
mod rows;

impl PlanView {
    pub fn draw(&mut self, frame: &mut Frame, area: Rect, view: &View, theme: &Theme) {
        self.doc_area = Rect::default();
        self.row_docs.clear();
        frame.render_widget(Clear, area);
        let Some(chunks) = self.frame_layout(frame, area, view) else {
            return;
        };
        let open_comments = view.open_plan_comments();
        self.draw_banner(frame, chunks[0], view, theme, &open_comments);
        self.draw_document(frame, chunks[1], view, theme);
        self.draw_buttons(frame, chunks[2], view, theme, &open_comments);
    }

    fn frame_layout(&self, frame: &mut Frame, area: Rect, view: &View) -> Option<[Rect; 3]> {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(crate::lean::tui::theme::ayu::BLUE))
            .title(format!(" Plan Document ({}) ", self.plan_id));
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
                Paragraph::new(self.comment_banner(view, theme, open_comments)),
                area,
            );
        } else {
            frame.render_widget(Paragraph::new(self.task_banner(view, theme)), area);
        }
    }

    fn comment_banner(
        &self,
        view: &View,
        theme: &Theme,
        open_comments: &[&super::super::view::Comment],
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
                " ↑/↓ move · c comment line · R send drafts for review · D delete draft · Esc close",
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

    fn task_banner(&self, view: &View, theme: &Theme) -> Line<'static> {
        let completed = view
            .tasks
            .iter()
            .filter(|task| task.status == "approved")
            .count();
        Line::from(vec![
            Span::styled(
                format!(" Tasks: {completed}/{} completed ", view.tasks.len()),
                Style::default()
                    .fg(crate::lean::tui::theme::ayu::BLUE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "· Navigate: ↑/↓/j/k · Comment line: c · Comment plan: C · Resolve: r · Close: Esc",
                theme.dim(),
            ),
        ])
    }

    fn draw_document(&mut self, frame: &mut Frame, doc_rect: Rect, view: &View, theme: &Theme) {
        self.doc_area = doc_rect;
        let doc_lines = self.build_lines(view);
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
