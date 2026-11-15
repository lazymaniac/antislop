use super::super::*;

mod graph;

pub(super) fn draw_secondary_or_main(app: &mut App, frame: &mut Frame) {
    if draw_plan(app, frame) || draw_diff(app, frame) || draw_details(app, frame) {
        return;
    }
    app.draw_main(frame);
}

fn draw_plan(app: &mut App, frame: &mut Frame) -> bool {
    let Some(plan) = &mut app.plan_view else {
        return false;
    };
    let theme = Theme::from_env();
    // PlanView::draw and its presentation helpers remain pending ownership transfer
    // to tui-specialized-screens; this coordinator only dispatches and syncs viewport state.
    plan.draw(frame, frame.area(), &app.view, &theme);
    app.text_area = Some(Rect::new(
        plan.doc_area.x.saturating_add(planview::GUTTER as u16),
        plan.doc_area.y,
        plan.doc_area.width.saturating_sub(planview::GUTTER as u16),
        plan.doc_area.height,
    ));
    app.text_origin = plan.scroll;
    true
}

fn draw_diff(app: &mut App, frame: &mut Frame) -> bool {
    if app.graph_expanded.is_some() {
        return false;
    }
    let Some(diff) = &mut app.diff else {
        return false;
    };
    diff.draw(frame, &app.view);
    true
}

fn draw_details(app: &mut App, frame: &mut Frame) -> bool {
    if app.graph_expanded.is_some() {
        return false;
    }
    let Some(details) = &mut app.details else {
        return false;
    };
    details.draw(frame, &app.view);
    app.text_area = Some(details.text_area);
    app.text_origin = details.scroll;
    app.sync_native_views();
    true
}

impl App {
    fn draw_footer(&self, frame: &mut Frame, footer: Rect, bar_height: u16, theme: &Theme) {
        frame.render_widget(Block::default().style(theme.bar()), footer);
        let text_area = Rect::new(footer.x, footer.y + (bar_height - 1) / 2, footer.width, 1);
        let content_width = footer.width.saturating_sub(4);
        let [status, help] =
            Layout::horizontal([Constraint::Length(content_width), Constraint::Length(4)])
                .areas(text_area);
        let line = self.header_line(content_width);
        // A taller bar makes its buttons taller too: each is filled on the
        // rows around its label.
        let buttons = button_rows(&line);
        for y in footer.y..footer.y + bar_height {
            if y != status.y {
                let row = Rect::new(status.x, y, status.width, 1);
                frame.render_widget(Paragraph::new(buttons.clone()).style(theme.bar()), row);
            }
        }
        frame.render_widget(Paragraph::new(line).style(theme.bar()), status);
        frame.render_widget(
            Paragraph::new(Span::styled(" [?]", theme.title())).style(theme.bar()),
            help,
        );
    }

    pub(super) fn draw_main(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let bar_height = Self::bar_height(area);
        let [body, _, footer] = Self::main_areas(area);
        self.workbench_area = None;
        self.workbench_sections = [Rect::default(); 3];

        let theme = Theme::from_env();
        if self.workbench_docked && area.width >= 100 {
            let chat_width = (body.width / 4).max(26);
            let wb_width = (body.width / 3).clamp(28, 44);
            let [chat, _, team, _, workbench] = Layout::horizontal([
                Constraint::Length(chat_width),
                Constraint::Length(1),
                Constraint::Min(24),
                Constraint::Length(1),
                Constraint::Length(wb_width),
            ])
            .areas(body);
            self.chat_area = chat;
            self.draw_chat(frame, chat, &theme);
            self.draw_graph(frame, team);
            self.draw_docked_workbench(frame, workbench, &theme);
        } else if area.width >= SIDE_BY_SIDE_BREAKPOINT {
            let chat_width = (body.width / 3).max(34);
            let [chat, _, team] = Layout::horizontal([
                Constraint::Length(chat_width),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .areas(body);
            self.chat_area = chat;
            self.draw_chat(frame, chat, &theme);
            if self.workbench_docked && team.height >= 16 {
                let wb_height = (team.height / 3).clamp(6, 11);
                let [graph_pane, workbench] =
                    Layout::vertical([Constraint::Min(8), Constraint::Length(wb_height)])
                        .areas(team);
                self.draw_graph(frame, graph_pane);
                self.draw_docked_workbench(frame, workbench, &theme);
            } else {
                self.draw_graph(frame, team);
            }
        } else if self.focus == Focus::Chat {
            self.chat_area = body;
            self.draw_chat(frame, body, &theme);
        } else if self.approval.is_some() && body.height > 1 {
            let desired = self.approval.as_ref().map_or(6, |approval| {
                let text_width = usize::from(body.width.saturating_sub(2)).max(1);
                super::super::approval::approval_panel_height(
                    &approval.approval.summary,
                    text_width,
                    approval.approval.options.len(),
                    usize::from(body.height),
                    7,
                    usize::from(body.height),
                )
            });
            let chat_height = desired
                .min((body.height / 2).max(5))
                .min(body.height.saturating_sub(1));
            let [team, chat] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(chat_height)]).areas(body);
            self.chat_area = chat;
            self.draw_graph(frame, team);
            self.draw_chat(frame, chat, &theme);
        } else if self.workbench_docked && body.height >= 16 {
            self.chat_area = body;
            let wb_height = (body.height / 3).clamp(6, 11);
            let [graph_pane, workbench] =
                Layout::vertical([Constraint::Min(8), Constraint::Length(wb_height)]).areas(body);
            self.draw_graph(frame, graph_pane);
            self.draw_docked_workbench(frame, workbench, &theme);
        } else {
            self.chat_area = body;
            self.draw_graph(frame, body);
        }

        self.draw_footer(frame, footer, bar_height, &theme);
    }

    fn draw_docked_workbench(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if area.width < 12 || area.height < 4 {
            return;
        }
        self.workbench_area = Some(area);
        let rigor = self.view.active_rigor().name();
        let selected_task = self.active_graph_task().cloned();
        let title = match &selected_task {
            Some(task) => format!(" workbench · {} · rigor: {rigor} (w) ", task.id),
            None => format!(" workbench · rigor: {rigor} (w) "),
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::ayu::BLUE))
            .title(truncate(&title, usize::from(area.width.saturating_sub(2))));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width < 4 || inner.height < 2 {
            return;
        }
        let third_h = (inner.height / 3).max(1);
        self.workbench_sections = [
            Rect::new(inner.x, inner.y, inner.width, third_h),
            Rect::new(
                inner.x,
                inner.y.saturating_add(third_h),
                inner.width,
                third_h,
            ),
            Rect::new(
                inner.x,
                inner.y.saturating_add(third_h * 2),
                inner.width,
                inner.height.saturating_sub(third_h * 2),
            ),
        ];

        let w = usize::from(inner.width.saturating_sub(1)).max(1);
        let mut lines = Vec::new();

        if let Some(task) = &selected_task {
            let meta = self
                .view
                .task_meta
                .get(&task.id)
                .cloned()
                .unwrap_or_default();
            let hold_badge = if meta.deferred { " · ⏸ HELD" } else { "" };
            lines.push(Line::from(Span::styled(
                truncate(
                    &format!("{} · {}{hold_badge}", task.id, view::clean(&task.title)),
                    w,
                ),
                theme.title().add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                truncate(
                    &format!(
                        "kind: {} · delivery: {:?}",
                        if task.kind.is_empty() {
                            "implementation"
                        } else {
                            &task.kind
                        },
                        task.change_kind
                    )
                    .to_ascii_lowercase(),
                    w,
                ),
                theme.dim(),
            )));
            lines.push(Line::from(Span::styled(
                truncate(&taskinfo::next_step(&self.view, task), w),
                theme.accent(),
            )));
            lines.push(Line::raw(""));

            let goal = if task.goal.trim().is_empty() {
                &task.title
            } else {
                &task.goal
            };
            lines.push(Line::from(Span::styled(
                "[1] Goal & Contract (click: details):",
                theme.title().add_modifier(Modifier::BOLD),
            )));
            for line in view::wrap(&view::clean(goal), w).into_iter().take(3) {
                lines.push(Line::raw(line));
            }
            if !task.depends_on.is_empty() {
                lines.push(Line::from(Span::styled(
                    truncate(&format!("depends on: {}", task.depends_on.join(", ")), w),
                    theme.dim(),
                )));
            }
            let unlocks: Vec<&str> = self
                .view
                .tasks
                .iter()
                .filter(|other| other.depends_on.iter().any(|d| d == &task.id))
                .map(|other| other.id.as_str())
                .collect();
            if !unlocks.is_empty() {
                lines.push(Line::from(Span::styled(
                    truncate(&format!("unlocks -> {}", unlocks.join(", ")), w),
                    theme.accent(),
                )));
            }
            if !meta.covers.is_empty() {
                lines.push(Line::from(Span::styled(
                    truncate(&format!("covers: {}", meta.covers.join(", ")), w),
                    theme.dim(),
                )));
            }
            if !meta.outputs.is_empty() {
                lines.push(Line::from(Span::styled(
                    truncate(&format!("outputs: {}", meta.outputs.join(", ")), w),
                    theme.dim(),
                )));
            }
            lines.push(Line::raw(""));

            lines.push(Line::from(Span::styled(
                "[2] Live PTY & Execution (click: console):",
                theme.title().add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                truncate(
                    &format!("W: {}", taskinfo::worker_state(&self.view, task)),
                    w,
                ),
                theme.text(),
            )));
            if let Some(act) = taskinfo::worker_activity(&self.view, task) {
                lines.push(Line::from(Span::styled(
                    truncate(&format!("   {act}"), w),
                    theme.accent(),
                )));
            }
            let pty_lines = self.live_pty_tail_for_task(&task.id, 3);
            if !pty_lines.is_empty() {
                for pty_line in pty_lines {
                    lines.push(Line::from(Span::styled(
                        truncate(&format!("  ▸ {}", view::clean(&pty_line)), w),
                        theme.accent(),
                    )));
                }
            }
            lines.push(Line::from(Span::styled(
                truncate(&format!("V: {}", taskinfo::reviewer_state(task)), w),
                theme.text(),
            )));
            lines.push(Line::raw(""));

            lines.push(Line::from(Span::styled(
                "[3] Gates & Mini-Diff (click: diff):",
                theme.title().add_modifier(Modifier::BOLD),
            )));
            if task.obligations.is_some() {
                lines.push(Line::from(Span::styled(
                    truncate(&format!("   {}", taskinfo::coverage_full(task)), w),
                    theme.dim(),
                )));
            }
            lines.push(Line::from(Span::styled(
                truncate(&taskinfo::integration_line(task), w),
                theme.dim(),
            )));
            if let Some(diff) = self.diff.as_ref().filter(|d| d.task == task.id) {
                for (kind, line_text) in diff.mini_preview(4) {
                    let style = match kind.as_str() {
                        "add" => Style::default().fg(theme::ayu::GREEN),
                        "del" => Style::default().fg(theme::ayu::RED),
                        _ => theme.title(),
                    };
                    lines.push(Line::from(Span::styled(
                        truncate(&format!("  {}", view::clean(&line_text)), w),
                        style,
                    )));
                }
            } else if let Some(files) = task
                .integration
                .as_ref()
                .and_then(|v| v["files"].as_array())
                .filter(|f| !f.is_empty())
            {
                let names: Vec<&str> = files.iter().filter_map(|v| v.as_str()).take(3).collect();
                lines.push(Line::from(Span::styled(
                    truncate(&format!("  files: {}", names.join(", ")), w),
                    theme.accent(),
                )));
            } else if let Some(effect) = &task.effect {
                lines.push(Line::from(Span::styled(
                    truncate(&format!("  Δ {}", view::clean(effect)), w),
                    theme.dim(),
                )));
            }

            if unblock::is_blocked(&task.status) {
                lines.push(Line::raw(""));
                lines.push(Line::from(Span::styled(
                    "Recovery options (press u):",
                    theme.warning().add_modifier(Modifier::BOLD),
                )));
                for (idx, choice) in unblock::choices_for(task).iter().enumerate().take(4) {
                    lines.push(Line::from(Span::styled(
                        truncate(&format!(" {}. {}", idx + 1, choice.label), w),
                        theme.warning(),
                    )));
                }
            }
        } else {
            lines.push(Line::from(Span::styled(
                truncate(&format!("[1] Workflow Rigor: {rigor} (Ctrl+M / R)"), w),
                theme.title().add_modifier(Modifier::BOLD),
            )));
            if !self.view.team_reason.trim().is_empty() {
                lines.push(Line::from(Span::styled(
                    truncate(&view::clean(&self.view.team_reason), w),
                    theme.dim(),
                )));
            } else {
                lines.push(Line::from(Span::styled(
                    truncate("  QuickFix (default) · Standard · Strict", w),
                    theme.dim(),
                )));
            }
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                truncate("[2] Direct Flow Editor (in graph):", w),
                theme.title().add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                truncate("  e edit · a +task · S split · l link", w),
                theme.text(),
            )));
            lines.push(Line::from(Span::styled(
                truncate("  h hold/defer · D delete · s steer", w),
                theme.text(),
            )));
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                truncate("[3] Zero-Ceremony Execution:", w),
                theme.title().add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                truncate("  /direct <prompt> -> instant worktree run", w),
                theme.accent(),
            )));
        }

        if inner.height > 2 {
            let max_body = usize::from(inner.height.saturating_sub(1));
            lines.truncate(max_body);
            while lines.len() < max_body {
                lines.push(Line::raw(""));
            }
            lines.push(Line::from(Span::styled(
                truncate(
                    "e edit · a +task · S split · h hold · D del · l link · s steer · d diff",
                    w,
                ),
                theme.dim(),
            )));
        }

        frame.render_widget(Paragraph::new(lines), inner);
    }

    pub(in crate::lean::tui) fn live_pty_tail_for_task(
        &self,
        task_id: &str,
        max_lines: usize,
    ) -> Vec<String> {
        for role in [format!("worker:{task_id}"), format!("reviewer:{task_id}")] {
            let key = format!("{}:{role}", self.conversation);
            if let Some(view) = self.native_views.get(&key) {
                let tail = view.recent_lines(max_lines);
                if !tail.is_empty() {
                    return tail;
                }
            }
            if let Some(screen) = self
                .details
                .as_ref()
                .and_then(|d| d.native.get(&role))
                .and_then(|snap| snap.screen.as_ref())
            {
                let mut rows: std::collections::BTreeMap<u16, Vec<(u16, String)>> =
                    std::collections::BTreeMap::new();
                for cell in &screen.cells {
                    if !cell.text.is_empty() {
                        rows.entry(cell.y)
                            .or_default()
                            .push((cell.x, cell.text.clone()));
                    }
                }
                let mut lines: Vec<String> = rows
                    .into_values()
                    .filter_map(|mut row_cells| {
                        row_cells.sort_by_key(|(x, _)| *x);
                        let line: String = row_cells.into_iter().map(|(_, text)| text).collect();
                        let trimmed = line.trim_end().to_owned();
                        (!trimmed.trim().is_empty()).then_some(trimmed)
                    })
                    .collect();
                if !lines.is_empty() {
                    if lines.len() > max_lines {
                        lines.drain(..lines.len() - max_lines);
                    }
                    return lines;
                }
            }
        }
        Vec::new()
    }
}

/// The bar line with every label blanked, keeping only the buttons' fill.
fn button_rows(line: &Line<'static>) -> Line<'static> {
    let spans = line.spans.iter().map(|span| {
        let width = unicode_width::UnicodeWidthStr::width(span.content.as_ref());
        let button = span.content.starts_with('[') && span.content.ends_with(']');
        let style = if button { span.style } else { Style::default() };
        Span::styled(" ".repeat(width), style)
    });
    Line::from(spans.collect::<Vec<_>>())
}
