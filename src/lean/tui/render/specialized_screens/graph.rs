use super::super::super::*;

impl App {
    pub(super) fn draw_graph(&mut self, frame: &mut Frame, area: Rect) {
        self.selected = graph::task_node_key(&self.selected);
        let (inner, now_area, progress) = self.graph_areas(frame, area);
        let graph = self.graph_layout_cache.get(
            &self.view,
            self.generation,
            &self.graph_scope,
            &self.graph_collapsed,
            self.graph_grouping,
            self.graph_viewport,
        );
        self.keep_graph_selection_visible(&graph);
        let mut canvas =
            Canvas::new_window(self.offset.0, self.offset.1, inner.width, inner.height);
        graph.render_with(
            &self.view,
            &mut canvas,
            self.relations,
            &self.selected,
            &Theme::from_env(),
            self.tick,
        );
        canvas.show(frame.buffer_mut(), inner, self.offset.0, self.offset.1);
        self.draw_progress(frame, now_area, &progress);
    }

    fn graph_areas(&mut self, frame: &mut Frame, area: Rect) -> (Rect, Option<Rect>, Vec<String>) {
        let frame_inner = self.draw_graph_frame(frame, area);
        let progress = self.view.progress();
        let (graph_area, now_area) = graph_progress_areas(frame_inner, progress.len());
        let inner = padded(graph_area);
        self.graph_viewport = Some((inner.width, inner.height));
        self.graph_area = Some(inner);
        (inner, now_area, progress)
    }

    fn draw_graph_frame(&self, frame: &mut Frame, area: Rect) -> Rect {
        let border = if self.focus == Focus::Graph {
            Style::default().fg(theme::ayu::BLUE)
        } else {
            dim()
        };
        let title = graph_title(
            &self.relations.name(),
            self.graph_scope.is_filtered(),
            self.graph_grouping.name(),
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        inner
    }

    fn keep_graph_selection_visible(&mut self, graph: &Graph) {
        self.repair_graph_selection(graph);
        self.fit_graph_offset(graph);
        // The view follows the selection only when it moves, so the user
        // can scroll away from it.
        if self.graph_revealed.as_deref() != Some(self.selected.as_str()) {
            self.reveal_selected_node(graph);
            self.graph_revealed = Some(self.selected.clone());
        }
    }

    fn repair_graph_selection(&mut self, graph: &Graph) {
        if graph.node(&self.selected).is_some_and(|node| node.visible) {
            return;
        }
        let worker = self
            .selected
            .split_once(':')
            .map(|(_, id)| format!("worker:{id}"));
        self.selected = worker
            .filter(|key| graph.node(key).is_some_and(|node| node.visible))
            .or_else(|| {
                graph
                    .nodes
                    .iter()
                    .find(|node| node.visible && node.key == ORCHESTRATOR)
                    .map(|node| node.key.clone())
            })
            .or_else(|| {
                graph
                    .nodes
                    .iter()
                    .find(|node| node.visible)
                    .map(|node| node.key.clone())
            })
            .unwrap_or_else(|| ORCHESTRATOR.into());
    }

    fn fit_graph_offset(&mut self, graph: &Graph) {
        let Some((width, height)) = self.graph_viewport else {
            return;
        };
        self.offset.0 = self.offset.0.min(graph.width.saturating_sub(width));
        self.offset.1 = self.offset.1.min(graph.height.saturating_sub(height));
    }

    fn reveal_selected_node(&mut self, graph: &Graph) {
        let Some(node) = graph.node(&self.selected) else {
            return;
        };
        let (left, top) = &mut self.offset;
        reveal_axis(
            left,
            node.x,
            node.width,
            self.graph_viewport.map_or(0, |v| v.0),
            3,
        );
        reveal_tall_axis(
            top,
            node.y,
            node.height,
            self.graph_viewport.map_or(0, |v| v.1),
        );
    }

    fn draw_progress(&self, frame: &mut Frame, area: Option<Rect>, progress: &[String]) {
        let Some(area) = area else {
            return;
        };
        let width = usize::from(area.width.saturating_sub(4));
        let lines = progress
            .iter()
            .take(usize::from(area.height.saturating_sub(2)))
            .map(|line| Line::raw(truncate(line, width)))
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme::ayu::YELLOW))
                    .title(" now ")
                    .padding(ratatui::widgets::Padding::horizontal(1)),
            ),
            area,
        );
    }
}

fn graph_title(relations: &str, filtered: bool, grouping: &str) -> String {
    let cards = if filtered { "some" } else { "all" };
    format!(" team · relations: {relations} (1-4) · cards: {cards} (f) · groups: {grouping} (g) ")
}

fn graph_progress_areas(area: Rect, progress_lines: usize) -> (Rect, Option<Rect>) {
    let height = (progress_lines.min(6) as u16 + 2).min(area.height / 2);
    if height < 3 {
        return (area, None);
    }
    let [graph, now] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(height)]).areas(area);
    (graph, Some(now))
}

fn reveal_axis(offset: &mut u16, start: u16, size: u16, viewport: u16, margin: u16) {
    if start < *offset {
        *offset = start.saturating_sub(margin);
    } else if start.saturating_add(size).saturating_add(margin) > offset.saturating_add(viewport) {
        *offset = start
            .saturating_add(size)
            .saturating_add(margin)
            .saturating_sub(viewport);
    }
}

fn reveal_tall_axis(offset: &mut u16, start: u16, size: u16, viewport: u16) {
    if size > viewport {
        let last = start.saturating_add(size).saturating_sub(viewport);
        if *offset < start || *offset > last {
            *offset = start;
        }
    } else if start < *offset {
        *offset = start;
    } else if start.saturating_add(size) > offset.saturating_add(viewport) {
        *offset = start.saturating_add(size).saturating_sub(viewport);
    }
}
