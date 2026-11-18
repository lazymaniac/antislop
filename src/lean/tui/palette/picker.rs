use super::catalog::COMMANDS;
use super::picker_layout::PickerLayout;
use super::*;

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
            "Flow editor: hold / release selected task (fast-path) · h".into(),
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
            "/hold · hold or release selected task via fast-path (h)".into(),
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

fn agent_items(view: &View) -> Vec<(String, Action)> {
    Graph::new(view)
        .nodes
        .iter()
        .filter(|node| node.visible)
        .map(|node| {
            let task = node
                .key
                .split_once(':')
                .and_then(|(role, id)| Some((role, view.tasks.iter().find(|task| task.id == id)?)));
            // A task's agents have cards of their own, an options task's
            // two share one.
            let label = task
                .map(|(role, task)| {
                    let role = if task.kind == "options" {
                        "worker reviewer"
                    } else {
                        role
                    };
                    format!("{} task · {} · {role}", task.id, task.title)
                })
                .unwrap_or_else(|| agent_name(&node.key));
            (label, Action::Select(node.key.clone()))
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
                Action::SwitchConversation(item.id.clone()),
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

fn recipient_items(view: &View) -> Vec<(String, Action)> {
    let mut items = vec![
        (
            "orchestrator · default recipient".into(),
            Action::Address("orchestrator".into()),
        ),
        (
            "all · broadcast to all active tasks".into(),
            Action::Address("all".into()),
        ),
        (
            "plan-reviewer · plan reviewer".into(),
            Action::Address("plan-reviewer".into()),
        ),
        (
            "auditor · final auditor".into(),
            Action::Address("auditor".into()),
        ),
    ];
    for task in &view.tasks {
        append_task_recipients(&mut items, task);
    }
    items
}

fn append_task_recipients(items: &mut Vec<(String, Action)>, task: &super::super::view::TaskRow) {
    let access = if task.change_kind.has_delivery() {
        "modifies files"
    } else {
        "report"
    };
    items.push((
        format!("worker:{} · {} ({access})", task.id, task.title),
        Action::Address(format!("worker:{}", task.id)),
    ));
    items.push((
        format!("reviewer:{} · task reviewer {}", task.id, task.title),
        Action::Address(format!("reviewer:{}", task.id)),
    ));
}

impl Picker {
    pub fn new(kind: Kind) -> Self {
        Picker {
            kind,
            query: String::new(),
            selected: 0,
            conversations: Vec::new(),
            projects: Vec::new(),
            confirm_delete: false,
        }
    }

    /// The entries matching what was typed, each word anywhere in the label.
    pub fn items(&self, view: &View) -> Vec<(String, Action)> {
        let all = self.unfiltered_items(view);
        self.matching_items(all)
    }

    fn unfiltered_items(&self, view: &View) -> Vec<(String, Action)> {
        match self.kind {
            Kind::Commands => {
                if self.query.trim().is_empty() {
                    command_items()
                } else {
                    extended_command_items(view)
                }
            }
            Kind::Agents => agent_items(view),
            Kind::History => history_items(view),
            Kind::Conversations => conversation_items(&self.conversations),
            Kind::Projects => project_items(&self.projects),
            Kind::Recipients => recipient_items(view),
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
        let items = self.items(view);
        self.selected = self.selected.min(items.len().saturating_sub(1));
        let Some(layout) = PickerLayout::new(frame.area(), items.len(), self.selected) else {
            return;
        };
        let lines = self.draw_lines(&items, layout);
        let title = if self.kind == Kind::Conversations && self.confirm_delete {
            " delete session? y confirm · n/Esc cancel "
        } else {
            picker_title(self.kind, layout.width)
        };
        draw_picker(frame, layout.box_area, lines, title);
        if show_cursor {
            self.place_cursor(frame, layout);
        }
    }

    fn draw_lines(&self, items: &[(String, Action)], layout: PickerLayout) -> Vec<Line<'static>> {
        let mut lines = vec![Line::raw(truncate(
            &format!("> {}", self.query),
            layout.inner,
        ))];
        if items.is_empty() {
            lines.push(Line::styled(
                truncate(
                    empty_message(self.kind, self.query.is_empty()),
                    layout.inner,
                ),
                dim(),
            ));
        }
        append_visible_items(&mut lines, items, self.selected, layout);
        lines
    }

    fn place_cursor(&self, frame: &mut Frame, layout: PickerLayout) {
        if layout.box_area.width < 3 || layout.box_area.height < 2 {
            return;
        }
        let col = (2 + self.query.width()).min(layout.inner.saturating_sub(1)) as u16;
        frame.set_cursor_position((layout.box_area.x + 1 + col, layout.box_area.y + 1));
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
) {
    for (index, (label, _)) in items
        .iter()
        .enumerate()
        .skip(layout.start)
        .take(layout.visible_rows)
    {
        let style = if index == selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::styled(truncate(label, layout.inner), style));
    }
}

fn picker_title(kind: Kind, width: u16) -> &'static str {
    match kind {
        Kind::Commands if width >= 42 => " commands · Enter run · Esc close ",
        Kind::Commands if width >= 20 => " commands · Esc ",
        Kind::Commands => " commands ",
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

fn draw_picker(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>, title: &str) {
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(super::super::theme::ayu::BLUE))
                .title(title),
        ),
        area,
    );
}
