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
        if step < graph::FULL_STEP {
            self.center_animated_card(&graph, step);
        }
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
        self.graph_card_area = None;
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

    /// Scroll so the animated card grows out of (or shrinks into) its
    /// centre: its centre moves from where the card was shown when the
    /// animation began (the viewport centre when closing) to where the fully
    /// opened card is shown, while both edges move outward.
    fn center_animated_card(&mut self, graph: &Graph, step: u8) {
        let Some((key, opening, _)) = self.graph_anim.as_ref() else {
            return;
        };
        let (Some(node), Some((width, height))) = (graph.node(key), self.graph_viewport) else {
            return;
        };
        let (node_x, node_y) = (i32::from(node.x), i32::from(node.y));
        let (node_w, node_h) = (i32::from(node.width), i32::from(node.height));
        let (anchor_x, anchor_y) = self
            .graph_anim_anchor
            .filter(|_| *opening)
            .unwrap_or((i32::from(width) / 2, i32::from(height) / 2));
        let progress = i32::from(step);
        let full = i32::from(graph::FULL_STEP);
        // Measured from the target, so the last steps settle onto it instead
        // of stopping a cell short when `from` is above or left of the view.
        let lerp = |from: i32, to: i32| to + (from - to) * (full - progress) / full;
        // The fully opened card lands where reveal_selected_node puts it.
        let settled = node_x - i32::from(expanded_offset_x(graph, (node.x, node.width), width));
        let left = lerp(anchor_x - node_w / 2, settled);
        let top = lerp(anchor_y - node_h / 2, 0);
        let max_x = i32::from(graph.width.saturating_sub(width));
        let max_y = i32::from(graph.height.saturating_sub(height));
        self.offset.0 = (node_x - left).clamp(0, max_x.max(0)) as u16;
        self.offset.1 = (node_y - top).clamp(0, max_y.max(0)) as u16;
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
        // The card's real screen position: when the graph is scrolled past
        // its top or left edge it starts outside the viewport, and the panels
        // must be clipped with it instead of sliding along with the scroll.
        let left = i32::from(viewport.x) + i32::from(node.x) - i32::from(self.offset.0);
        let top = i32::from(viewport.y) + i32::from(node.y) - i32::from(self.offset.1);
        let [card, selector, content] =
            graph::expanded_card_areas(viewport, (left, top), (node.width, node.height));
        if card.is_empty() {
            self.graph_card_area = None;
            return;
        }
        self.graph_card_area = Some(content).filter(|area| !area.is_empty());
        if !selector.is_empty() {
            render_card_round_selector(frame, selector, self.details.as_ref(), &self.view, &key);
        }
        if content.is_empty() {
            return;
        }
        if self.graph_tab == 0 {
            self.draw_card_details(frame, content, &key);
        }

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
        if has_round_snapshot
            && self.graph_tab == 2
            && let Some(scroll) = render_card_round_data(
                frame,
                content,
                self.graph_card_scroll,
                self.details.as_ref(),
            )
        {
            self.graph_card_scroll = scroll;
        }
    }

    /// The Details tab shows the agent's dashboard: its task, state, checks,
    /// findings and history, scrolled within the card.
    fn draw_card_details(&mut self, frame: &mut Frame, content: Rect, key: &str) {
        let theme = Theme::from_env();
        let Some(details) = self
            .details
            .as_mut()
            .filter(|details| details.key == key && details.key != "conversation")
        else {
            return;
        };
        if content.width < 24 || content.height < 3 {
            return;
        }
        frame.render_widget(ratatui::widgets::Clear, content);
        frame.render_widget(Block::default().style(theme.raised()), content);
        // The card names only its role: say which task, as of the round shown.
        let row = key
            .split_once(':')
            .and_then(|(_, id)| self.view.tasks.iter().find(|task| task.id == id));
        let snapshot = |field: &str| {
            details
                .task
                .as_ref()
                .and_then(|task| task[field].as_str())
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        let title = snapshot("title").or_else(|| row.map(|row| row.title.clone()));
        let status = snapshot("status").or_else(|| row.map(|row| row.status.clone()));
        let heading = match (title.filter(|t| !t.is_empty()), status) {
            (Some(title), Some(status)) if !status.is_empty() => format!("{title} · {status}"),
            (Some(title), _) => title,
            _ => String::new(),
        };
        let mut lines = Vec::new();
        if !heading.is_empty() {
            lines.push(Line::styled(
                truncate(&theme.clean(&heading), usize::from(content.width)),
                theme.title().add_modifier(Modifier::BOLD),
            ));
        }
        if let Some(agent) = self.view.agents.get(key) {
            lines.push(Line::styled(
                truncate(&theme.clean(&agent_line(agent)), usize::from(content.width)),
                theme.dim(),
            ));
        }
        let used = (lines.len() as u16).min(content.height);
        frame.render_widget(
            Paragraph::new(lines),
            Rect::new(content.x, content.y, content.width, used),
        );
        let body = Rect::new(
            content.x,
            content.y + used,
            content.width,
            content.height - used,
        );
        details.draw_dashboard(frame, &self.view, body, &theme);
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
            area.width.saturating_sub(2),
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
            *left = expanded_offset_x(
                graph,
                (node.x, node.width),
                self.graph_viewport.map_or(0, |v| v.0),
            );
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
    row: Rect,
    details: Option<&Details>,
    view: &View,
    key: &str,
) {
    if row.width < 4 {
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
    let previous = if position > 1 { " · , previous" } else { "" };
    let round_text = if details.is_none_or(|details| details.selected_round.is_none()) {
        format!(
            "Round {selected} ({position}/{}){previous} · following current work",
            rounds.len()
        )
    } else {
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
        Paragraph::new(truncate(&text, usize::from(row.width))).style(Theme::from_env().accent()),
        row.intersection(frame.area()),
    );
}

/// Draws the selected round's results from `scroll`; returns the scroll the
/// page settled at.
fn render_card_round_data(
    frame: &mut Frame,
    area: Rect,
    scroll: usize,
    details: Option<&Details>,
) -> Option<usize> {
    let Some(details) = details else {
        render_card_history_message(frame, area, "Round data is unavailable.");
        return None;
    };
    if details.task.is_none() {
        let message = if let Some(error) = details.fetch_error.as_deref() {
            format!("Round data unavailable: {error}")
        } else if details.fetch_in_flight {
            "Loading selected round data…".to_owned()
        } else {
            "No stored data is available for this round.".to_owned()
        };
        render_card_history_message(frame, area, &message);
        return None;
    }
    let theme = Theme::from_env();
    if let Some(document) = details.documents.get("result") {
        match document.viewport_filtered(
            usize::from(area.width),
            usize::from(area.height),
            scroll,
            false,
            "",
        ) {
            Ok(Some(page)) => {
                let settled = page.scroll;
                frame.render_widget(
                    Paragraph::new(
                        page.lines
                            .into_iter()
                            .map(|(_, line)| Line::raw(theme.clean(&line)))
                            .collect::<Vec<_>>(),
                    )
                    .style(theme.text()),
                    area,
                );
                return Some(settled);
            }
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
    None
}

/// The CLI behind a card and what its turns cost so far.
fn agent_line(agent: &crate::lean::tui::view::Agent) -> String {
    let mut parts = vec![format!(
        "CLI: {}",
        if agent.provider.is_empty() {
            "unknown"
        } else {
            &agent.provider
        }
    )];
    if let Some(model) = agent.model.as_deref() {
        parts.push(match agent.effort.as_deref() {
            Some(effort) => format!("model {model} ({effort})"),
            None => format!("model {model}"),
        });
    }
    parts.push(if agent.running {
        "running".to_owned()
    } else {
        format!("{} turns", agent.turns)
    });
    if agent.tokens_in.is_some() {
        parts.push(format!("tokens {}", agent.tokens()));
    }
    if agent.busy_ms > 0 {
        parts.push(format!(
            "busy {}",
            crate::lean::tui::view::duration(agent.busy_ms)
        ));
    }
    parts.join(" · ")
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

/// The horizontal offset centring an opened card in a view `width` wide,
/// within the graph: a graph wider than a phone's panel, its other rows
/// scrolling, still shows the card with even margins.
fn expanded_offset_x(graph: &Graph, (x, card): (u16, u16), width: u16) -> u16 {
    (x + card / 2)
        .saturating_sub(width / 2)
        .min(graph.width.saturating_sub(width))
}

fn graph_position_marker_glyph(above: bool, ascii: bool) -> &'static str {
    match (above, ascii) {
        (true, true) => "^",
        (false, true) => "v",
        (true, false) => "↑",
        (false, false) => "↓",
    }
}

/// The longest title that fits `width` between the corners: the keys go
/// first, then the labels, rather than cutting a word in half.
fn graph_title(relations: &str, filtered: bool, grouping: &str, width: u16) -> String {
    let cards = if filtered { "some" } else { "all" };
    [
        format!(
            " team · relations: {relations} (1-4) · cards: {cards} (f) · groups: {grouping} (g) "
        ),
        format!(" team · relations: {relations} · cards: {cards} · groups: {grouping} "),
        format!(" team · {relations} · {cards} · {grouping} "),
        format!(" team · {relations} "),
    ]
    .into_iter()
    .find(|title| unicode_width::UnicodeWidthStr::width(title.as_str()) <= usize::from(width))
    .unwrap_or_else(|| " team ".into())
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
    use super::{graph_position_marker_glyph, graph_title, graph_vertical_markers};
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

    #[test]
    fn graph_title_drops_its_keys_before_cutting_a_word() {
        let full = graph_title("DRM", false, "dependencies", 200);
        assert!(full.contains("groups: dependencies (g)"));
        // A phone-wide panel: the keys go, every word stays whole.
        let narrow = graph_title("DRM", false, "dependencies", 68);
        assert_eq!(
            narrow,
            " team · relations: DRM · cards: all · groups: dependencies "
        );
        assert_eq!(
            graph_title("DRM", true, "status", 30),
            " team · DRM · some · status "
        );
        assert_eq!(graph_title("DRM", false, "dependencies", 4), " team ");
    }
}
