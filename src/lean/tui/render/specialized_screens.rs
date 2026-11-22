use super::super::*;
use ratatui::layout::Alignment;

mod graph;

pub(super) fn draw_secondary_or_main(app: &mut App, frame: &mut Frame, theme: &Theme) {
    if draw_plan(app, frame, theme) || draw_workbench(app, frame, theme) {
        return;
    }
    app.draw_main(frame, theme);
}

fn draw_plan(app: &mut App, frame: &mut Frame, theme: &Theme) -> bool {
    let Some(plan) = &mut app.plan_view else {
        return false;
    };
    // PlanView::draw and its presentation helpers remain pending ownership transfer
    // to tui-specialized-screens; this coordinator only dispatches and syncs viewport state.
    plan.draw(frame, frame.area(), &app.view, theme);
    app.text_area = Some(Rect::new(
        plan.doc_area.x.saturating_add(planview::GUTTER as u16),
        plan.doc_area.y,
        plan.doc_area.width.saturating_sub(planview::GUTTER as u16),
        plan.doc_area.height,
    ));
    app.text_origin = plan.scroll;
    true
}

fn draw_workbench(app: &mut App, frame: &mut Frame, theme: &Theme) -> bool {
    if app.graph_expanded.is_some()
        || app.navigation.screen != Screen::Workbench
        || (app.diff.is_none() && app.details.is_none())
    {
        return false;
    }
    let presentation = app.navigation.presentation();
    let terminal_owner = app.workbench_terminal_owner();
    let terminal_view_role = app.workbench_terminal_view_role();
    let terminal_round = workbench_terminal_round(
        &app.workbench_sessions,
        app.workbench_sessions_after,
        app.details.as_ref(),
        &presentation,
        &terminal_owner,
    );
    let historical_terminal = historical_workbench_terminal(app.details.as_ref(), &terminal_owner);
    let historical_diff = app.historical_diff_selected();
    let areas = super::super::workbench::areas_with_footer(
        frame.area(),
        &presentation,
        app.navigation.focus,
        super::direction::footer_height(app, frame.area()),
    );
    app.workbench_areas = areas;
    draw_workbench_chrome(
        frame,
        areas,
        app,
        &presentation,
        &terminal_owner,
        terminal_round,
        theme,
    );
    if areas.list_visible {
        draw_workbench_list(
            frame,
            areas.list,
            &app.view,
            &app.navigation.target,
            &terminal_owner,
            theme,
        );
    }
    if let Some(diff) = &mut app.diff {
        let panel = areas.content.inner(ratatui::layout::Margin::new(
            u16::from(areas.content.width >= 80) * 2,
            u16::from(areas.content.height >= 10),
        ));
        let [heading, tabs, content] = Layout::vertical([
            Constraint::Length(if panel.height >= 16 { 2 } else { 0 }),
            Constraint::Length(if panel.height >= 12 { 2 } else { 1 }),
            Constraint::Min(0),
        ])
        .areas(panel);
        let task = app
            .details
            .as_ref()
            .and_then(|details| details.task_id())
            .and_then(|id| app.view.tasks.iter().find(|task| task.id == id));
        let title = task.map_or("Changes", |task| task.title.as_str());
        let mut label = vec![Span::styled(theme.clean(title), theme.text())];
        if let Some(branch) = &app.view.session_branch {
            label.push(Span::styled(
                format!("    {}", theme.clean(branch)),
                theme.dim(),
            ));
        }
        frame.render_widget(
            Paragraph::new(Line::from(label)),
            Rect::new(heading.x, heading.y, heading.width, 1),
        );
        let (counts, metadata) = diff.workbench_labels(theme);
        let counts_width = unicode_width::UnicodeWidthStr::width(counts.as_str()) as u16;
        if heading.width >= counts_width + 24 {
            frame.render_widget(
                Paragraph::new(counts).style(theme.accent()),
                Rect::new(heading.right() - counts_width, heading.y, counts_width, 1),
            );
        }
        if tabs.height > 1 {
            frame.render_widget(
                Paragraph::new(metadata).style(theme.dim()),
                Rect::new(tabs.x, tabs.y + 1, tabs.width, 1),
            );
        }
        if let Some(details) = &app.details {
            frame.render_widget(
                super::super::workbench::tab_line(&details.key, true, WorkbenchTab::Diff, theme),
                Rect::new(tabs.x, tabs.y, tabs.width, 1),
            );
        }
        if historical_diff {
            frame.render_widget(
                Paragraph::new(theme.clean("No historical diff is retained for this round."))
                    .style(theme.dim()),
                content,
            );
        } else {
            diff.paint_content(frame, content, &app.view, theme);
        }
    } else if app
        .details
        .as_ref()
        .is_some_and(|details| details.key == "conversation")
        && presentation.tab == WorkbenchTab::Conversation
    {
        app.draw_chat(frame, areas.content, theme);
    } else if let Some(details) = &mut app.details {
        details.draw_workbench_at(
            frame,
            areas.content.inner(ratatui::layout::Margin::new(
                u16::from(areas.content.width >= 80) * 2,
                u16::from(areas.content.height >= 10),
            )),
            Rect::default(),
            &app.view,
            presentation.tab,
            theme,
        );
    }
    if let Some(details) = &mut app.details {
        draw_workbench_terminal(
            frame,
            areas.terminal,
            details,
            &app.view,
            &terminal_view_role,
            presentation.native_input,
            historical_terminal,
            theme,
        );
        if app.diff.is_none() && details.key != "conversation" {
            app.text_area = Some(details.text_area);
            app.text_origin = details.scroll;
        }
    }
    if let Some(notice) = &app.notice {
        frame.render_widget(
            Paragraph::new(theme.clean(notice)).style(theme.warning()),
            Rect::new(
                areas.footer.x,
                areas.footer.bottom().saturating_sub(1),
                areas.footer.width,
                1,
            ),
        );
    }
    app.sync_native_views();
    true
}

fn draw_workbench_chrome(
    frame: &mut Frame,
    areas: super::super::workbench::WorkbenchAreas,
    app: &mut App,
    presentation: &WorkbenchPresentation,
    terminal_owner: &str,
    terminal_round: Option<u64>,
    theme: &Theme,
) {
    let role = terminal_owner.split(':').next().unwrap_or(terminal_owner);
    let state = if presentation.native_input {
        "native input"
    } else {
        "preview"
    };
    super::chrome::brand(frame, areas.header, app, theme);
    let context = theme.chrome(&format!(
        "{role} {state} · round {} · {}",
        terminal_round.map_or_else(|| "—".into(), |round| round.to_string()),
        if presentation.pinned {
            "pinned"
        } else {
            "following"
        },
    ));
    let task = app
        .details
        .as_ref()
        .and_then(|details| details.task_id())
        .and_then(|id| app.view.tasks.iter().find(|task| task.id == id));
    let title = task.map_or("Session", |task| task.title.as_str());
    let y = areas.header.y + if areas.header.height >= 4 { 2 } else { 1 };
    let context_width = if areas.header.width >= 80 {
        (unicode_width::UnicodeWidthStr::width(context.as_str()) as u16).min(areas.header.width / 2)
    } else {
        0
    };
    let title_width = areas
        .header
        .width
        .saturating_sub(context_width + u16::from(context_width > 0) * 3);
    frame.render_widget(
        Paragraph::new(theme.fit(
            &format!("Workbench / {}", theme.clean(title)),
            usize::from(title_width),
        ))
        .style(theme.text()),
        Rect::new(areas.header.x, y, title_width, 1),
    );
    if context_width > 0 {
        frame.render_widget(
            Paragraph::new(theme.fit(&context, usize::from(context_width))).style(theme.dim()),
            Rect::new(areas.header.right() - context_width, y, context_width, 1),
        );
    }
    let direction = Rect::new(
        areas.footer.x,
        areas.footer.y,
        areas.footer.width,
        areas.footer.height.saturating_sub(1),
    );
    app.direction_area = direction;
    let footer = Rect::new(
        areas.footer.x,
        areas.footer.bottom().saturating_sub(1),
        areas.footer.width,
        1,
    );
    if !super::direction::keybar(app, frame, footer, theme) {
        super::chrome::keybar(
            app,
            frame,
            footer,
            &[
                ("tab panes", KeyCode::Tab, KeyModifiers::NONE),
                ("t terminal", KeyCode::Char('t'), KeyModifiers::NONE),
                ("p pin", KeyCode::Char('p'), KeyModifiers::NONE),
                ("z expand", KeyCode::Char('z'), KeyModifiers::NONE),
                ("esc back", KeyCode::Esc, KeyModifiers::NONE),
                ("ctrl p actions", KeyCode::Char('p'), KeyModifiers::CONTROL),
                ("F1 help", KeyCode::F(1), KeyModifiers::NONE),
            ],
            theme,
        );
    }
    if !areas.separator.is_empty() {
        frame.render_widget(
            Paragraph::new(if theme.ascii { "..." } else { "⋯" })
                .alignment(Alignment::Center)
                .style(theme.faint()),
            areas.separator,
        );
    }
    frame.render_widget(Block::default().style(theme.card()), areas.content);
}

fn workbench_terminal_round(
    sessions: &[crate::lean::store::SessionInfo],
    sessions_after: u64,
    details: Option<&super::super::Details>,
    presentation: &WorkbenchPresentation,
    owner: &str,
) -> Option<u64> {
    let matching_details = details.filter(|details| details_match_terminal_owner(details, owner));
    matching_details
        .and_then(|details| details.selected_round)
        .or_else(|| {
            matching_details
                .filter(|details| details.fetched_events >= sessions_after)
                .and_then(|details| details.task.as_ref()?.get("round")?.as_u64())
        })
        .or_else(|| {
            sessions
                .iter()
                .filter(|session| session.role == owner)
                .count()
                .checked_sub(1)
                .map(|index| index as u64)
        })
        .or(presentation.round.filter(|_| matching_details.is_some()))
}

fn historical_workbench_terminal(details: Option<&super::super::Details>, owner: &str) -> bool {
    details.is_some_and(|details| {
        details.selected_round.is_some()
            && !details.resume_round
            && details_match_terminal_owner(details, owner)
    })
}

fn details_match_terminal_owner(details: &super::super::Details, owner: &str) -> bool {
    details.composer_recipient(&details.key) == owner
        || details.task_uuid.as_deref().is_some_and(|task| {
            owner
                .split_once(':')
                .is_some_and(|(_, owner_task)| owner_task == task)
        })
}

#[allow(clippy::too_many_arguments)]
fn draw_workbench_terminal(
    frame: &mut Frame,
    area: Rect,
    details: &mut super::super::Details,
    view: &View,
    role: &str,
    native_input: bool,
    historical: bool,
    theme: &Theme,
) {
    if historical {
        details.native_focus = false;
        details.native_areas = [Rect::default(); 2];
        frame.render_widget(
            Paragraph::new(
                theme.clean(
                    "Historical PTY is read-only; no retained terminal snapshot is available.",
                ),
            )
            .style(theme.dim()),
            area,
        );
    } else {
        details.native_focus = native_input;
        details.draw_workbench_terminal_for(frame, area, view, role, theme);
    }
}

fn draw_workbench_list(
    frame: &mut Frame,
    area: Rect,
    view: &View,
    target: &WorkbenchTarget,
    owner: &str,
    theme: &Theme,
) {
    let rows = super::super::workbench::sidebar(view);
    let start = super::super::workbench::visible_start(view, target, area.height);
    let selected = match target {
        WorkbenchTarget::Session => "conversation",
        WorkbenchTarget::Agent(key) => key,
    };
    let work = super::super::work_units::WorkUnits::new(view);
    let selected_unit = work.for_stage(selected);
    for (index, row) in rows
        .iter()
        .skip(start)
        .take(usize::from(area.height))
        .enumerate()
    {
        let row_area = Rect::new(area.x, area.y + index as u16, area.width, 1);
        let current = row.key == selected;
        let group_selected = !row.stage
            && !row.label.is_empty()
            && work
                .for_stage(&row.key)
                .zip(selected_unit)
                .is_some_and(|(a, b)| a.id == b.id);
        let style = if group_selected {
            theme.selected().patch(if row.label.starts_with(' ') {
                theme.dim()
            } else {
                theme.text()
            })
        } else if current && row.stage {
            theme.accent()
        } else if row.stage || row.label.starts_with(' ') || row.key == "conversation" {
            theme.dim()
        } else {
            theme.text()
        };
        let label = if row.stage {
            let health = crate::lean::tui::graph::health(view, &row.key);
            let status = if row.key == owner
                && view.agents.get(&row.key).is_some_and(|agent| agent.running)
            {
                "live PTY"
            } else {
                health.name()
            };
            let left = format!(
                "  {} {}",
                theme.status_glyph(health.name()),
                row.label.trim_start()
            );
            let gap = usize::from(area.width.saturating_sub(1)).saturating_sub(
                unicode_width::UnicodeWidthStr::width(left.as_str()) + status.len(),
            );
            format!("{left}{}{status}", " ".repeat(gap.max(1)))
        } else {
            row.label.clone()
        };
        frame.render_widget(
            Paragraph::new(theme.fit(&theme.clean(&label), usize::from(area.width))).style(style),
            row_area,
        );
    }
    if rows.len().saturating_sub(start) + 5 < usize::from(area.height) {
        let task = owner.split_once(':').and_then(|(_, id)| {
            view.tasks
                .iter()
                .find(|task| task.id == id || task.uuid.as_deref() == Some(id))
        });
        let title = task.map_or("Session", |task| task.title.as_str());
        let role = owner.split(':').next().unwrap_or(owner);
        frame.render_widget(
            Paragraph::new("TERMINAL FOLLOWS").style(theme.dim()),
            Rect::new(area.x, area.bottom() - 4, area.width, 1),
        );
        frame.render_widget(
            Paragraph::new(theme.fit(&format!("{title} / {role}"), usize::from(area.width)))
                .style(theme.text()),
            Rect::new(area.x, area.bottom() - 3, area.width, 1),
        );
        frame.render_widget(
            Paragraph::new(theme.chrome("p · pin this terminal")).style(theme.dim()),
            Rect::new(area.x, area.bottom() - 1, area.width, 1),
        );
    }
}

impl App {
    fn draw_footer(&mut self, frame: &mut Frame, footer: Rect, theme: &Theme) {
        if super::direction::keybar(self, frame, footer, theme) {
            return;
        }
        let pause = if self.view.paused {
            "space continue"
        } else {
            "space pause"
        };
        super::chrome::keybar(
            self,
            frame,
            footer,
            &[
                ("↑ ↓ focus", KeyCode::Down, KeyModifiers::NONE),
                ("enter inspect", KeyCode::Enter, KeyModifiers::NONE),
                ("d decisions", KeyCode::Char('d'), KeyModifiers::NONE),
                (pause, KeyCode::Char(' '), KeyModifiers::NONE),
                ("h history", KeyCode::Char('h'), KeyModifiers::NONE),
                ("ctrl p actions", KeyCode::Char('p'), KeyModifiers::CONTROL),
                ("F1 help", KeyCode::F(1), KeyModifiers::NONE),
            ],
            theme,
        );
    }

    pub(super) fn draw_main(&mut self, frame: &mut Frame, theme: &Theme) {
        let area = frame.area();
        let body = super::chrome::inset(area);
        self.chat_area = body;
        let spacious = body.height >= 25;
        let [identity, canvas, dock] = Layout::vertical([
            Constraint::Length(if spacious {
                2
            } else {
                u16::from(body.height >= 10)
            }),
            Constraint::Min(1),
            Constraint::Length(super::direction::footer_height(self, area)),
        ])
        .areas(body);
        let pending = self.view.approvals.len() + usize::from(self.view.user_decision);
        let status_area = super::chrome::brand(frame, identity, self, theme);
        if pending > 0 && identity.height > 1 && identity.width >= 60 {
            frame.render_widget(
                Paragraph::new(theme.chrome(&format!("{pending} decision(s) · d")))
                    .style(theme.decision()),
                Rect::new(identity.right().saturating_sub(22), identity.y + 1, 22, 1),
            );
        }
        self.draw_graph(frame, canvas, theme);
        self.now_area = status_area;

        self.direction_area = Rect::new(dock.x, dock.y, dock.width, dock.height.saturating_sub(1));
        let footer = Rect::new(dock.x, dock.bottom().saturating_sub(1), dock.width, 1);
        self.draw_footer(frame, footer, theme);
    }
}
