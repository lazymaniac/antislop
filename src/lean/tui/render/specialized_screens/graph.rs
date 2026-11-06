use super::super::super::*;

impl App {
    pub(super) fn draw_graph(&mut self, frame: &mut Frame, area: Rect) {
        self.selected = graph::card_key(&self.view, &self.selected);
        let (inner, now_area, progress) = self.graph_areas(frame, area);
        if self.graph_anim.is_some() {
            // The card moves while it grows: keep it in view each frame.
            self.graph_revealed = None;
        }
        let (expanded, step) = self.graph_expansion();
        let graph = self.graph_layout_cache.get_with_expanded(
            &self.view,
            &self.graph_scope,
            &self.graph_collapsed,
            self.graph_grouping,
            self.graph_viewport,
            graph::LayoutIdentity {
                generation: self.generation,
                expanded,
                tab: self.graph_tab,
                step,
            },
        );
        self.keep_graph_selection_visible(&graph);
        self.draw_graph_position_markers(frame, inner, &graph);
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
        if step == graph::FULL_STEP {
            self.draw_expanded_card_panels(frame, inner, &graph);
        }
        self.draw_progress(frame, now_area, &progress);
    }

    /// The card being laid out as expanded and how far it has grown: it eases
    /// out over `ANIMATION` both when opening and when closing.
    pub(in crate::lean::tui) fn graph_expansion(&mut self) -> (Option<String>, u8) {
        const ANIMATION: std::time::Duration = std::time::Duration::from_millis(220);
        let Some((key, opening, started)) = self.graph_anim.clone() else {
            return (self.graph_expanded.clone(), graph::FULL_STEP);
        };
        let t = started.elapsed().as_secs_f32() / ANIMATION.as_secs_f32();
        if t >= 1.0 {
            self.graph_anim = None;
            let settled = opening.then_some(key);
            return (settled, graph::FULL_STEP);
        }
        let eased = 1.0 - (1.0 - t).powi(3);
        let full = f32::from(graph::FULL_STEP);
        let grown = (eased * full).round() as u8;
        let step = if opening {
            grown
        } else {
            graph::FULL_STEP - grown
        };
        (Some(key), step)
    }

    fn draw_graph_position_markers(&self, frame: &mut Frame, viewport: Rect, graph: &Graph) {
        let Some((_, height)) = self.graph_viewport else {
            return;
        };
        let has_top_margin = viewport.y > 0;
        let bottom_y = viewport.y.saturating_add(viewport.height);
        let has_bottom_margin = bottom_y < frame.area().bottom();
        let (above, below) = graph_vertical_markers(graph, self.offset.1, height);
        if !above && !below {
            return;
        }

        let theme = Theme::from_env();
        let style = Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED);
        let x = viewport.x.saturating_add(viewport.width / 2);
        if above && has_top_margin {
            let marker = graph_position_marker_glyph(true, theme.ascii);
            frame
                .buffer_mut()
                .set_string(x, viewport.y - 1, marker, style);
        }
        if below && has_bottom_margin {
            let marker = graph_position_marker_glyph(false, theme.ascii);
            frame.buffer_mut().set_string(x, bottom_y, marker, style);
        }
    }

    fn draw_expanded_card_panels(&mut self, frame: &mut Frame, viewport: Rect, graph: &Graph) {
        let Some(key) = self.graph_expanded.clone() else {
            return;
        };
        let Some(node) = graph.node(&key).cloned().filter(|node| node.visible) else {
            return;
        };
        let card = Rect::new(
            viewport.x + node.x.saturating_sub(self.offset.0),
            viewport.y + node.y.saturating_sub(self.offset.1),
            node.width,
            node.height,
        )
        .intersection(viewport);
        let content = graph::expanded_card_content_area(card, viewport);

        if self.graph_tab == 1 {
            if let Some(details) = &mut self.details {
                details.native_focus &= details.selected_round.is_none();
                details.native_areas = [content, Rect::default()];
            }
            if self
                .details
                .as_ref()
                .is_some_and(|details| details.selected_round.is_some())
            {
                render_card_history_message(
                    frame,
                    content,
                    "Historical PTY is read-only; no retained terminal snapshot is available.",
                );
            } else {
                self.sync_native_views();
                let snapshot = self
                    .details
                    .as_ref()
                    .and_then(|details| details.native.get(&key))
                    .cloned();
                match snapshot {
                    Some(native_view::Snapshot {
                        screen: Some(screen),
                        ..
                    }) => native_view::render(frame, content, &screen, self.native_focused()),
                    Some(native_view::Snapshot {
                        error: Some(error), ..
                    }) => render_card_history_message(frame, content, &error),
                    _ => render_card_history_message(
                        frame,
                        content,
                        "No current PTY session is available.",
                    ),
                }
            }
        }
        if self.graph_tab == 3
            && let Some(diff) = &mut self.diff
        {
            if self
                .details
                .as_ref()
                .is_some_and(|details| details.selected_round.is_some())
            {
                render_card_history_message(
                    frame,
                    content,
                    "No historical diff is retained for this round.",
                );
            } else {
                diff.paint_at(
                    frame,
                    Rect::new(card.x, content.y, card.width, content.height),
                    &self.view,
                );
            }
        }
        let has_round_snapshot = self.details.as_ref().is_some_and(|details| {
            details.key == key && (details.task_id().is_some() || details.selected_round.is_some())
        });
        if has_round_snapshot && matches!(self.graph_tab, 0 | 2) {
            render_card_round_data(frame, content, self.graph_tab, self.details.as_ref());
        }
        render_card_round_selector(frame, card, self.details.as_ref(), &self.view, &key);
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
        if self.graph_expanded.as_deref() == Some(self.selected.as_str()) {
            // An opened card fills the view: put all of it on screen.
            *left = node.x.saturating_sub(3);
            *top = node.y;
            return;
        }
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

fn render_card_round_selector(
    frame: &mut Frame,
    card: Rect,
    details: Option<&Details>,
    view: &View,
    key: &str,
) {
    if card.height < 4 || card.width < 8 {
        return;
    }
    let details = details.filter(|details| details.key == key);
    let mut rounds = details.map_or_else(Vec::new, |details| details.available_rounds.clone());
    let latest = view
        .tasks
        .iter()
        .find(|task| key.split_once(':').is_some_and(|(_, id)| task.id == id))
        .map(|task| task.round)
        .or_else(|| rounds.last().copied())
        .or_else(|| details.and_then(|details| details.task.as_ref()?.get("round")?.as_u64()))
        .unwrap_or(0);
    if !rounds.contains(&latest) {
        rounds.push(latest);
        rounds.sort_unstable();
    }
    let selected = details
        .and_then(|details| details.selected_round)
        .unwrap_or(latest);
    let position = rounds
        .iter()
        .position(|round| *round == selected)
        .unwrap_or(0)
        + 1;
    let round_text = if details.is_none_or(|details| details.selected_round.is_none()) {
        format!(
            "Round {selected} ({position}/{}) · , previous · following current work",
            rounds.len()
        )
    } else {
        let previous = if position > 1 { " · , previous" } else { "" };
        let next = if position < rounds.len() {
            " · . next"
        } else {
            ""
        };
        format!(
            "Round {selected} ({position}/{}){previous}{next} · . newest follows current",
            rounds.len()
        )
    };
    let text = if rounds.is_empty() {
        "Round unavailable · ,/. navigate retained rounds".to_owned()
    } else {
        round_text
    };
    frame.render_widget(
        Paragraph::new(truncate(&text, usize::from(card.width.saturating_sub(4))))
            .style(Theme::from_env().accent()),
        Rect::new(
            card.x.saturating_add(2),
            card.y.saturating_add(3),
            card.width.saturating_sub(4),
            1,
        )
        .intersection(frame.area()),
    );
}

fn render_card_round_data(frame: &mut Frame, area: Rect, tab: usize, details: Option<&Details>) {
    let Some(details) = details else {
        render_card_history_message(frame, area, "Round data is unavailable.");
        return;
    };
    let Some(task) = details.task.as_ref() else {
        let message = if let Some(error) = details.fetch_error.as_deref() {
            format!("Round data unavailable: {error}")
        } else if details.fetch_in_flight {
            "Loading selected round data…".to_owned()
        } else {
            "No stored data is available for this round.".to_owned()
        };
        render_card_history_message(frame, area, &message);
        return;
    };
    let theme = Theme::from_env();
    if tab == 0 {
        let lines = [
            task["title"].as_str().map(|value| format!("Task: {value}")),
            task["status"]
                .as_str()
                .map(|value| format!("Status: {value}")),
            task["result"]
                .as_str()
                .map(|value| format!("Result: {value}")),
            task["base"].as_str().map(|value| format!("Base: {value}")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if lines.is_empty() {
            render_card_history_message(frame, area, "No details are retained for this round.");
        } else {
            frame.render_widget(
                Paragraph::new(
                    lines
                        .into_iter()
                        .map(|line| Line::raw(theme.clean(&line)))
                        .collect::<Vec<_>>(),
                )
                .style(theme.text()),
                area,
            );
        }
    } else if let Some(document) = details.documents.get("result") {
        match document.viewport_filtered(
            usize::from(area.width),
            usize::from(area.height),
            0,
            false,
            "",
        ) {
            Ok(Some(page)) => frame.render_widget(
                Paragraph::new(
                    page.lines
                        .into_iter()
                        .map(|(_, line)| Line::raw(theme.clean(&line)))
                        .collect::<Vec<_>>(),
                )
                .style(theme.text()),
                area,
            ),
            Ok(None) => {
                render_card_history_message(frame, area, "Preparing selected round results…")
            }
            Err(error) => render_card_history_message(
                frame,
                area,
                &format!("Could not render selected round results: {error}"),
            ),
        }
    } else {
        render_card_history_message(frame, area, "No result data is retained for this round.");
    }
}

fn render_card_history_message(frame: &mut Frame, area: Rect, message: &str) {
    frame.render_widget(
        Paragraph::new(Theme::from_env().clean(message)).style(Theme::from_env().dim()),
        area,
    );
}

fn graph_vertical_markers(graph: &Graph, offset_y: u16, viewport_height: u16) -> (bool, bool) {
    let bottom = offset_y.saturating_add(viewport_height);
    graph
        .nodes
        .iter()
        .filter(|node| node.visible)
        .fold((false, false), |(above, below), node| {
            (
                above || node.y < offset_y,
                below || node.y.saturating_add(node.height) > bottom,
            )
        })
}

fn graph_position_marker_glyph(above: bool, ascii: bool) -> &'static str {
    match (above, ascii) {
        (true, true) => "^",
        (false, true) => "v",
        (true, false) => "↑",
        (false, false) => "↓",
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

#[cfg(test)]
mod tests {
    use super::{graph_position_marker_glyph, graph_vertical_markers};
    use crate::lean::tui::{graph::Graph, view::View};
    use ratatui::style::{Modifier, Style};

    #[test]
    fn graph_position_markers_have_ascii_and_visible_no_color_styles() {
        assert_eq!(graph_position_marker_glyph(true, false), "↑");
        assert_eq!(graph_position_marker_glyph(false, false), "↓");
        assert_eq!(graph_position_marker_glyph(true, true), "^");
        assert_eq!(graph_position_marker_glyph(false, true), "v");

        let marker_style = Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED);
        assert!(marker_style.add_modifier.contains(Modifier::BOLD));
        assert!(marker_style.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn graph_position_marker_conditions_respect_viewport_boundaries() {
        let mut graph = Graph::new(&View::default());
        for node in &mut graph.nodes {
            node.visible = false;
        }
        let node = &mut graph.nodes[0];
        node.visible = true;
        node.y = 5;
        node.height = 3;

        assert_eq!(graph_vertical_markers(&graph, 0, 8), (false, false));
        assert_eq!(graph_vertical_markers(&graph, 0, 7), (false, true));
        assert_eq!(graph_vertical_markers(&graph, 8, 1), (true, false));
        assert_eq!(graph_vertical_markers(&graph, 0, 0), (false, true));
    }
}
