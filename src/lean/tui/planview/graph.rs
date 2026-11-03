use super::*;
use crate::lean::tui::graph::{Canvas, PlanGraph};
use crate::lean::tui::view::wrap;

impl PlanView {
    pub(super) fn draw_task_graph(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &View,
        theme: &Theme,
    ) {
        let block = panel(
            " Tasks (v) · arrows select · Shift+↑/↓ scroll ",
            self.graph_focused,
            theme,
        );
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let detail_height = if inner.height >= 12 { 7 } else { 0 };
        let [viewport, details] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(detail_height)]).areas(inner);
        if viewport != self.graph_area {
            self.graph_reveal = true;
        }
        self.graph_area = viewport;
        self.graph_selected = self.graph_selected.min(view.tasks.len().saturating_sub(1));
        let graph = PlanGraph::new(&view.tasks, viewport.width);
        if let Some(node) = graph.nodes.get(self.graph_selected)
            && self.graph_reveal
        {
            reveal(&mut self.graph_offset.0, node.x, node.width, viewport.width);
            reveal(
                &mut self.graph_offset.1,
                node.y,
                node.height,
                viewport.height,
            );
        }
        self.graph_reveal = false;
        self.graph_offset.0 = self
            .graph_offset
            .0
            .min(graph.width.saturating_sub(viewport.width));
        self.graph_offset.1 = self
            .graph_offset
            .1
            .min(graph.height.saturating_sub(viewport.height));
        let (left, top) = self.graph_offset;
        let mut canvas = Canvas::new_window(left, top, viewport.width, viewport.height);
        graph.render(&view.tasks, &mut canvas, self.graph_selected, theme);
        canvas.show(frame.buffer_mut(), viewport, left, top);
        self.task_rects = graph
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                let visible =
                    node.intersection(Rect::new(left, top, viewport.width, viewport.height));
                (!visible.is_empty()).then(|| {
                    (
                        Rect::new(
                            viewport.x + visible.x - left,
                            viewport.y + visible.y - top,
                            visible.width,
                            visible.height,
                        ),
                        index,
                    )
                })
            })
            .collect();
        if let Some(task) = view.tasks.get(self.graph_selected) {
            self.draw_task_details(frame, details, task, view, theme);
        }
    }

    fn draw_task_details(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        task: &TaskRow,
        view: &View,
        theme: &Theme,
    ) {
        let mut text = format!(
            "{}: {}\n{} · {}",
            task.id,
            task.title,
            task.kind,
            if task.writes { "writes" } else { "read-only" },
        );
        for line in self
            .build_lines(view)
            .iter()
            .filter(|line| line.task_id.as_deref() == Some(&task.id))
        {
            for comment in self.line_comments(view, line.number) {
                text.push_str(&format!(
                    "\n#{} [{}]: {}",
                    comment.id,
                    view.comment_state(comment.id),
                    clean(&comment.text)
                ));
                if let Some(error) = view.comment_error(comment.id) {
                    text.push_str(&format!("\nError: {}", clean(error)));
                }
            }
        }
        text.push_str(&format!(
            "\n{}\nDepends on: {} · Parallel with: {}",
            task.goal,
            if task.depends_on.is_empty() {
                "none".into()
            } else {
                task.depends_on.join(", ")
            },
            if task.parallel_with.is_empty() {
                "none".into()
            } else {
                task.parallel_with.join(", ")
            }
        ));
        let block = Block::default()
            .borders(Borders::TOP)
            .border_style(theme.dim())
            .title(" Selected task · c comment · Ctrl+u/d scroll ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let rows = wrap(&text, usize::from(inner.width).max(1));
        self.task_scroll = self
            .task_scroll
            .min(rows.len().saturating_sub(usize::from(inner.height)));
        let lines: Vec<_> = rows
            .into_iter()
            .skip(self.task_scroll)
            .take(usize::from(inner.height))
            .map(Line::raw)
            .collect();
        frame.render_widget(Paragraph::new(lines).style(theme.text()), inner);
    }

    pub(super) fn selected_task_line(&self, view: &View) -> Option<usize> {
        let task = view.tasks.get(self.graph_selected)?;
        let lines = self.build_lines(view);
        lines
            .iter()
            .find(|line| line.is_heading && line.text.contains(&format!("[{}]", task.id)))
            .or_else(|| {
                lines
                    .iter()
                    .find(|line| line.is_heading && line.task_id.as_deref() == Some(&task.id))
            })
            .map(|line| line.number)
    }

    pub(super) fn graph_key(&mut self, key: KeyEvent, view: &View) -> bool {
        if view.tasks.is_empty() {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('d') => self.task_scroll = self.task_scroll.saturating_add(3),
                KeyCode::Char('u') => self.task_scroll = self.task_scroll.saturating_sub(3),
                _ => return false,
            }
            return true;
        }
        if key.modifiers.contains(KeyModifiers::SHIFT) {
            let (x, y) = &mut self.graph_offset;
            match key.code {
                KeyCode::Left => *x = x.saturating_sub(4),
                KeyCode::Right => *x = x.saturating_add(4),
                KeyCode::Up => *y = y.saturating_sub(3),
                KeyCode::Down => *y = y.saturating_add(3),
                _ => return false,
            }
            self.graph_reveal = false;
            return true;
        }
        let last = view.tasks.len().saturating_sub(1);
        self.graph_selected = self.graph_selected.min(last);
        self.graph_selected = match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.graph_selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => (self.graph_selected + 1).min(last),
            KeyCode::PageUp => self.graph_selected.saturating_sub(5),
            KeyCode::PageDown => (self.graph_selected + 5).min(last),
            KeyCode::Home | KeyCode::Char('g') => 0,
            KeyCode::End | KeyCode::Char('G') => last,
            KeyCode::Left | KeyCode::Right => {
                let task = &view.tasks[self.graph_selected];
                view.tasks
                    .iter()
                    .position(|other| {
                        if key.code == KeyCode::Left {
                            task.depends_on.contains(&other.id)
                        } else {
                            other.depends_on.contains(&task.id)
                        }
                    })
                    .unwrap_or(self.graph_selected)
            }
            _ => return false,
        };
        self.graph_reveal = true;
        self.task_scroll = 0;
        true
    }
}

fn reveal(offset: &mut u16, start: u16, size: u16, viewport: u16) {
    if start < *offset || size >= viewport {
        *offset = start;
    } else if start.saturating_add(size) > offset.saturating_add(viewport) {
        *offset = start.saturating_add(size).saturating_sub(viewport);
    }
}

pub(super) fn panel(title: &str, focused: bool, theme: &Theme) -> Block<'static> {
    let borders = if theme.ascii {
        ratatui::symbols::border::Set {
            top_left: "+",
            top_right: "+",
            bottom_left: "+",
            bottom_right: "+",
            vertical_left: "|",
            vertical_right: "|",
            horizontal_top: "-",
            horizontal_bottom: "-",
        }
    } else {
        ratatui::symbols::border::ROUNDED
    };
    Block::default()
        .borders(Borders::ALL)
        .border_set(borders)
        .border_style(if focused { theme.accent() } else { theme.dim() })
        .title(title.to_owned())
}
