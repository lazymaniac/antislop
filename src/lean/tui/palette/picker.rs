use super::catalog::COMMANDS;
use super::picker_layout::PickerLayout;
use super::*;
use crate::lean::tui::theme::Theme;

const SMALL_CHANGE_OPTIONS_SUMMARY: &str = "options when enabled; static analysis + tests";

fn command_items() -> Vec<(String, Action)> {
    COMMANDS
        .iter()
        .map(|(command, label)| ((*label).to_owned(), Action::Run(*command)))
        .collect()
}

fn extended_command_items(view: &View) -> Vec<(String, Action)> {
    use crate::lean::tasks::Team;
    let mut items = command_items();
    let current = view.rigor_label();
    items.extend([
        (
            "Pause / continue execution · Space".into(),
            Action::Run(Command::PauseOrContinue),
        ),
        (
            "Cancel current work · Ctrl+X".into(),
            Action::Run(Command::Cancel),
        ),
        (
            "Direct execution: zero-ceremony 1-task run (bypass orchestrator) · /direct".into(),
            Action::Run(Command::DirectExecute),
        ),
        (
            format!("Rigor / ceremony level (current: {current}) · /rigor"),
            Action::Run(Command::OpenRigorPicker),
        ),
        (
            format!("Rigor: small-change · {SMALL_CHANGE_OPTIONS_SUMMARY}"),
            Action::Run(Command::SelectTeam(Team::SmallChange)),
        ),
        (
            "Rigor: feature · full ceremony (plan review + quality gates + audit)".into(),
            Action::Run(Command::SelectTeam(Team::Feature)),
        ),
        (
            "Rigor: refactoring · behavior-preserving change (plan review + audit)".into(),
            Action::Run(Command::SelectTeam(Team::Refactoring)),
        ),
        (
            "Rigor: security-fix · security gate + audit".into(),
            Action::Run(Command::SelectTeam(Team::SecurityFix)),
        ),
        (
            "Rigor: code-review · code review without generated quality gates".into(),
            Action::Run(Command::SelectTeam(Team::CodeReview)),
        ),
        (
            "Rigor: no-quality-gates · requested work + final integration only".into(),
            Action::Run(Command::SelectTeam(Team::NoQualityGates)),
        ),
        (
            "Rigor: research · investigation & evidence (final audit)".into(),
            Action::Run(Command::SelectTeam(Team::Research)),
        ),
        (
            "Rigor: document · documentation & writing".into(),
            Action::Run(Command::SelectTeam(Team::Document)),
        ),
        (
            "Flow editor: edit selected task contract in-place · e".into(),
            Action::Run(Command::EditTask),
        ),
        (
            "Flow editor: add dependent task after selected · a".into(),
            Action::Run(Command::AddDependentTask),
        ),
        (
            "Flow editor: split selected task into two steps · S".into(),
            Action::Run(Command::SplitTask),
        ),
        (
            "Flow editor: hold / release selected task (fast-path)".into(),
            Action::Run(Command::HoldTask),
        ),
        (
            "Flow editor: retire / remove selected task · D".into(),
            Action::Run(Command::RetireTask),
        ),
        (
            "Flow editor: link / unlink dependency edge on graph · l".into(),
            Action::Run(Command::LinkTask),
        ),
        (
            "Steer selected task's agent (queue / steer / interrupt) · s".into(),
            Action::Run(Command::SteerAgent),
        ),
        (
            "Plan: accept proposed plan and start execution · /accept".into(),
            Action::Run(Command::AcceptPlan),
        ),
    ]);
    items
}

fn rigor_items(view: &View) -> Vec<(String, Action)> {
    use crate::lean::tasks::Team;
    let current = view.team;
    let entries = [
        (Team::SmallChange, "small-change", "light ceremony"),
        (
            Team::Feature,
            "feature",
            "full ceremony · plan review + quality gates + final audit",
        ),
        (
            Team::Refactoring,
            "refactoring",
            "behavior-preserving · plan review + static analysis + test + audit",
        ),
        (
            Team::SecurityFix,
            "security-fix",
            "security ceremony · security review + static analysis + test + audit",
        ),
        (
            Team::CodeReview,
            "code-review",
            "review code changes · no generated quality gates",
        ),
        (
            Team::NoQualityGates,
            "no-quality-gates",
            "requested work · ordinary acceptance + final integration only",
        ),
        (
            Team::Research,
            "research",
            "investigation · evidence gathering + final audit",
        ),
        (
            Team::Document,
            "document",
            "documentation · writing + review, no code gates",
        ),
    ];
    entries
        .into_iter()
        .map(|(team, name, desc)| {
            let mark = if current == Some(team) {
                " (current)"
            } else {
                ""
            };
            let label = if team == Team::SmallChange {
                format!("{name}{mark} · {desc} · {SMALL_CHANGE_OPTIONS_SUMMARY}")
            } else {
                format!("{name}{mark} · {desc}")
            };
            (label, Action::Run(Command::SelectTeam(team)))
        })
        .collect()
}

fn slash_command_items(view: &View) -> Vec<(String, Action)> {
    use crate::lean::tasks::Team;
    let current = view.rigor_label();
    vec![
        (
            "/direct · zero-ceremony 1-task execution (bypass orchestrator)".into(),
            Action::Run(Command::DirectExecute),
        ),
        (
            format!("/rigor · switch workflow rigor / ceremony (current: {current})"),
            Action::Run(Command::OpenRigorPicker),
        ),
        (
            format!("/small-change · light rigor ({SMALL_CHANGE_OPTIONS_SUMMARY})"),
            Action::Run(Command::SelectTeam(Team::SmallChange)),
        ),
        (
            "/feature · full rigor (plan review + quality gates + final audit)".into(),
            Action::Run(Command::SelectTeam(Team::Feature)),
        ),
        (
            "/refactoring · refactoring rigor (plan review + audit)".into(),
            Action::Run(Command::SelectTeam(Team::Refactoring)),
        ),
        (
            "/security-fix · security rigor (security gate + audit)".into(),
            Action::Run(Command::SelectTeam(Team::SecurityFix)),
        ),
        (
            "/code-review · code review without generated quality gates".into(),
            Action::Run(Command::SelectTeam(Team::CodeReview)),
        ),
        (
            "/no-quality-gates · requested work + final integration only".into(),
            Action::Run(Command::SelectTeam(Team::NoQualityGates)),
        ),
        (
            "/research · research rigor (evidence + audit)".into(),
            Action::Run(Command::SelectTeam(Team::Research)),
        ),
        (
            "/document · documentation rigor".into(),
            Action::Run(Command::SelectTeam(Team::Document)),
        ),
        (
            "/edit · edit selected task contract in-place (e)".into(),
            Action::Run(Command::EditTask),
        ),
        (
            "/add-task · add dependent task after selected (a)".into(),
            Action::Run(Command::AddDependentTask),
        ),
        (
            "/split-task · split selected task into two steps (S)".into(),
            Action::Run(Command::SplitTask),
        ),
        (
            "/hold · hold or release selected task".into(),
            Action::Run(Command::HoldTask),
        ),
        (
            "/retire · retire/remove selected task from flow (D)".into(),
            Action::Run(Command::RetireTask),
        ),
        (
            "/link · link or unlink dependency edge on graph (l)".into(),
            Action::Run(Command::LinkTask),
        ),
        (
            "/steer · live-steer selected task's agent (s)".into(),
            Action::Run(Command::SteerAgent),
        ),
        (
            "/plan · open current plan document (Ctrl+L)".into(),
            Action::Run(Command::Plan),
        ),
        (
            "/accept · accept proposed plan and start execution".into(),
            Action::Run(Command::AcceptPlan),
        ),
        (
            "/reject · reject proposed plan (F7)".into(),
            Action::Run(Command::Reject),
        ),
        (
            "/unblock · unblock or recover stopped task (F5)".into(),
            Action::Run(Command::TaskActions),
        ),
        (
            "/pause · pause or continue execution (F6)".into(),
            Action::Run(Command::PauseOrContinue),
        ),
        (
            "/approvals · answer pending CLI approval requests".into(),
            Action::Run(Command::PendingApprovals),
        ),
    ]
}

fn agent_items(view: &View, dot: &str) -> Vec<(String, Action)> {
    crate::lean::tui::work_units::WorkUnits::new(view)
        .units
        .into_iter()
        .flat_map(|unit| {
            unit.stages.into_iter().map(move |stage| {
                let label = if let Some((role, id)) = stage.id.0.split_once(':') {
                    let title = view
                        .tasks
                        .iter()
                        .find(|task| task.id == id)
                        .map_or(unit.title.as_str(), |task| task.title.as_str());
                    format!("{id} task {dot} {title} {dot} {role}")
                } else {
                    agent_name(&stage.id.0)
                };
                (label, Action::Select(stage.id.0))
            })
        })
        .collect()
}

fn history_items(view: &View) -> Vec<(String, Action)> {
    view.transcript
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, entry)| !entry.text.trim().is_empty())
        .map(|(index, entry)| (entry.text.trim().replace('\n', " "), Action::Show(index)))
        .collect()
}

fn conversation_items(items: &[ConversationItem]) -> Vec<(String, Action)> {
    items
        .iter()
        .map(|item| {
            (
                item.title.clone(),
                Action::SwitchConversation {
                    id: item.id.clone(),
                    project: item.project.clone(),
                },
            )
        })
        .collect()
}

fn project_items(items: &[ProjectItem]) -> Vec<(String, Action)> {
    items
        .iter()
        .map(|item| (item.title.clone(), Action::OpenProject(item.path.clone())))
        .collect()
}

fn recipient_items(view: &View, dot: &str) -> Vec<(String, Action)> {
    let mut items = vec![
        (
            format!("orchestrator {dot} default recipient"),
            Action::Address("orchestrator".into()),
        ),
        (
            format!("all {dot} broadcast to all active tasks"),
            Action::Address("all".into()),
        ),
        (
            format!("plan-reviewer {dot} plan reviewer"),
            Action::Address("plan-reviewer".into()),
        ),
        (
            format!("auditor {dot} final auditor"),
            Action::Address("auditor".into()),
        ),
    ];
    for role in [crate::lean::PLANNER, crate::lean::COMMENT_ORCHESTRATOR] {
        if view.agents.contains_key(role) {
            items.push((
                format!("{} {dot} session stage", agent_name(role)),
                Action::Address(role.into()),
            ));
        }
    }
    for task in &view.tasks {
        append_task_recipients(&mut items, task, dot);
    }
    items
}

fn append_task_recipients(
    items: &mut Vec<(String, Action)>,
    task: &super::super::view::TaskRow,
    dot: &str,
) {
    let access = if task.change_kind.has_delivery() {
        "modifies files"
    } else {
        "report"
    };
    items.push((
        format!("worker:{} {dot} {} ({access})", task.id, task.title),
        Action::Address(format!("worker:{}", task.id)),
    ));
    items.push((
        format!("reviewer:{} {dot} task reviewer {}", task.id, task.title),
        Action::Address(format!("reviewer:{}", task.id)),
    ));
}

impl Picker {
    pub fn new(kind: Kind) -> Self {
        Picker {
            status: None,
            kind,
            query: String::new(),
            selected: 0,
            conversations: Vec::new(),
            projects: Vec::new(),
            confirm_delete: false,
            delete_target: None,
            recommendation: None,
        }
    }

    pub(in crate::lean::tui) fn set_conversations(
        &mut self,
        items: Vec<ConversationItem>,
        view: &View,
    ) {
        let selected = self
            .items(view)
            .get(self.selected)
            .map(|(_, action)| action.clone());
        self.conversations = items;
        let choices = self.items(view);
        self.selected = selected
            .and_then(|selected| choices.iter().position(|(_, action)| *action == selected))
            .unwrap_or(self.selected.min(choices.len().saturating_sub(1)));
    }

    pub fn rigor(view: &View) -> Self {
        let mut picker = Self::new(Kind::Rigor);
        picker.selected = picker
            .items(view)
            .iter()
            .position(|(_, action)| {
                *action == Action::Run(Command::SelectTeam(view.active_rigor()))
            })
            .unwrap_or_default();
        if view.team_recommended {
            picker.recommendation = Some((view.active_rigor(), view.team_reason.clone()));
        }
        picker
    }

    /// The entries matching what was typed, each word anywhere in the label.
    pub fn items(&self, view: &View) -> Vec<(String, Action)> {
        let all = self.unfiltered_items(view, "·");
        self.matching_items(all)
    }

    fn unfiltered_items(&self, view: &View, dot: &str) -> Vec<(String, Action)> {
        match self.kind {
            Kind::Commands => {
                if self.query.trim().is_empty() {
                    command_items()
                } else {
                    extended_command_items(view)
                }
            }
            Kind::Agents => agent_items(view, dot),
            Kind::History => history_items(view),
            Kind::Conversations => conversation_items(&self.conversations),
            Kind::Projects => project_items(&self.projects),
            Kind::Recipients => recipient_items(view, dot),
            Kind::SlashCommands => slash_command_items(view),
            Kind::Rigor => rigor_items(view),
        }
    }

    fn matching_items(&self, all: Vec<(String, Action)>) -> Vec<(String, Action)> {
        let words: Vec<String> = self
            .query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        let mut matching: Vec<(String, Action)> = all
            .into_iter()
            .filter(|(label, _)| {
                let label = label.to_lowercase();
                words.iter().all(|word| label.contains(word.as_str()))
            })
            .collect();
        // A typed path can be opened even when it is in no list.
        let typed = self.query.trim();
        if self.kind == Kind::Projects && (typed.starts_with(['/', '~', '.'])) {
            matching.push((
                format!("Open {typed}"),
                Action::OpenProject(typed.to_owned()),
            ));
        }
        matching
    }

    pub fn draw(&mut self, frame: &mut Frame, view: &View) {
        self.draw_with_cursor(frame, view, true);
    }

    pub fn draw_with_cursor(&mut self, frame: &mut Frame, view: &View, show_cursor: bool) {
        self.draw_themed(frame, view, show_cursor, &Theme::from_env());
    }

    pub(in crate::lean::tui) fn draw_themed(
        &mut self,
        frame: &mut Frame,
        view: &View,
        show_cursor: bool,
        theme: &Theme,
    ) {
        let items = if matches!(self.kind, Kind::Agents | Kind::Recipients) {
            let canonical = self.unfiltered_items(view, "·");
            let themed = self.unfiltered_items(view, theme.glyphs().dot);
            let matching = self.items(view);
            canonical
                .into_iter()
                .zip(themed)
                .filter_map(|(original, themed)| matching.contains(&original).then_some(themed))
                .collect()
        } else {
            self.items(view)
        };
        self.selected = self.selected.min(items.len().saturating_sub(1));
        let header_rows = usize::from(self.status.is_some())
            + if self.recommendation.is_some() {
                1 + usize::from(frame.area().height.saturating_sub(5).min(2))
            } else {
                1
            };
        let Some(layout) = PickerLayout::new(frame.area(), items.len(), self.selected, header_rows)
        else {
            return;
        };
        let lines = self.draw_lines(&items, layout, theme);
        let title = if self.kind == Kind::Conversations && self.confirm_delete {
            self.delete_target.as_ref().map_or_else(
                || theme.chrome(" delete session? y confirm · n/Esc cancel "),
                |target| {
                    format!(
                        " delete {}? {}",
                        target.title,
                        theme.chrome("y confirm · n/Esc cancel ")
                    )
                },
            )
        } else if self.recommendation.is_some() {
            theme.chrome(" Recommended rigor · Enter confirm · Esc close ")
        } else {
            theme.chrome(picker_title(self.kind, layout.width))
        };
        draw_picker(frame, layout.box_area, lines, &title, theme);
        if show_cursor {
            self.place_cursor(frame, layout);
        }
    }

    fn draw_lines(
        &self,
        items: &[(String, Action)],
        layout: PickerLayout,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if let Some(status) = &self.status {
            lines.push(Line::styled(
                truncate(status, layout.inner),
                theme.metadata(),
            ));
        }
        if let Some((team, reason)) = &self.recommendation
            && layout.header_rows >= 2
        {
            lines.push(Line::raw(truncate(
                &format!("Orchestrator recommends: {}", team.name()),
                layout.inner,
            )));
            if layout.header_rows >= 3 {
                let reason = if reason.trim().is_empty() {
                    "Default rigor for this plan."
                } else {
                    reason
                };
                let reason = truncate(reason, layout.inner);
                lines.push(Line::styled(
                    truncate(&format!("Reason: {reason}"), layout.inner),
                    theme.dim(),
                ));
            }
        }
        lines.push(Line::raw(truncate(
            &format!("> {}", self.query),
            layout.inner,
        )));
        if items.is_empty() {
            lines.push(Line::styled(
                truncate(
                    empty_message(self.kind, self.query.is_empty()),
                    layout.inner,
                ),
                theme.dim(),
            ));
        }
        append_visible_items(
            &mut lines,
            items,
            self.selected,
            layout,
            theme,
            matches!(
                self.kind,
                Kind::Commands | Kind::Rigor | Kind::SlashCommands
            ),
        );
        lines.push(Line::styled(
            truncate(
                &theme.chrome("↑/↓ move · Enter select · Esc close"),
                layout.inner,
            ),
            theme.dim(),
        ));
        lines
    }

    fn place_cursor(&self, frame: &mut Frame, layout: PickerLayout) {
        if layout.box_area.width < 3
            || usize::from(layout.box_area.height) <= layout.header_rows + 1
        {
            return;
        }
        let col = (2 + self.query.width()).min(layout.inner.saturating_sub(1)) as u16;
        frame.set_cursor_position((
            layout.box_area.x + 1 + col,
            layout.box_area.y + layout.header_rows as u16,
        ));
    }
}

fn empty_message(kind: Kind, query_empty: bool) -> &'static str {
    if !query_empty {
        return "Nothing matches the query.";
    }
    match kind {
        Kind::Commands => "No commands available.",
        Kind::Agents => "No agents in graph.",
        Kind::History => "No entries in conversation history.",
        Kind::Conversations => "No other conversations in this project.",
        Kind::Recipients => "No recipients matching query.",
        Kind::Projects => "No other projects yet: type a directory path (/…, ~/…).",
        Kind::SlashCommands => "No slash commands matching query.",
        Kind::Rigor => "No rigor levels matching query.",
    }
}

fn append_visible_items(
    lines: &mut Vec<Line<'static>>,
    items: &[(String, Action)],
    selected: usize,
    layout: PickerLayout,
    theme: &Theme,
    chrome: bool,
) {
    for (index, (label, _)) in items
        .iter()
        .enumerate()
        .skip(layout.start)
        .take(layout.visible_rows)
    {
        let label = if chrome {
            theme.chrome(label)
        } else {
            label.clone()
        };
        let label = theme.fit(&label, layout.inner);
        let padding = layout.inner.saturating_sub(label.width());
        let line = format!("{label}{}", " ".repeat(padding));
        let style = if index == selected {
            theme.selected()
        } else {
            theme.text()
        };
        lines.push(Line::styled(line, style));
    }
}

fn picker_title(kind: Kind, width: u16) -> &'static str {
    match kind {
        Kind::Commands => "Actions",
        Kind::Agents if width >= 45 => " find agent · Enter select · Esc close ",
        Kind::Agents if width >= 20 => " agents · Esc ",
        Kind::Agents => " agents ",
        Kind::History if width >= 45 => " search conversation · Enter show · Esc close ",
        Kind::History if width >= 20 => " search · Esc ",
        Kind::History => " search ",
        Kind::Conversations if width >= 45 => {
            " switch conversation · Enter open · x delete · Esc close "
        }
        Kind::Conversations if width >= 20 => " conversations · Esc ",
        Kind::Conversations => " conversations ",
        Kind::Recipients if width >= 45 => " message recipient · Enter select · Esc close ",
        Kind::Recipients if width >= 20 => " recipients · Esc ",
        Kind::Recipients => " recipients ",
        Kind::Projects if width >= 50 => " projects · Enter open · type a path · Esc close ",
        Kind::Projects if width >= 20 => " projects · Esc ",
        Kind::Projects => " projects ",
        Kind::SlashCommands if width >= 45 => " slash commands (/) · Enter run · Esc close ",
        Kind::SlashCommands if width >= 20 => " slash commands · Esc ",
        Kind::SlashCommands => " commands ",
        Kind::Rigor if width >= 45 => " workflow rigor · Enter select · Esc close ",
        Kind::Rigor if width >= 20 => " rigor · Esc ",
        Kind::Rigor => " rigor ",
    }
}

fn draw_picker(
    frame: &mut Frame,
    area: Rect,
    lines: Vec<Line<'static>>,
    title: &str,
    theme: &Theme,
) {
    frame.render_widget(Clear, area);
    frame.render_widget(Block::default().style(theme.card()), area);
    frame.render_widget(
        Paragraph::new(theme.fit(title.trim(), usize::from(area.width.saturating_sub(2))))
            .style(theme.accent()),
        Rect::new(
            area.x + u16::from(area.width > 1),
            area.y,
            area.width.saturating_sub(2),
            area.height.min(1),
        ),
    );
    frame.render_widget(
        Paragraph::new(lines).style(theme.card()),
        Rect::new(
            area.x + u16::from(area.width > 1),
            area.y.saturating_add(1),
            area.width.saturating_sub(2),
            area.height.saturating_sub(1),
        ),
    );
}
