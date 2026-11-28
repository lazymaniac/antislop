//! Direct DAG flow editor (`D2` Plan & Flow Architecture): edit task contracts
//! in-place, add dependent tasks, split tasks, hold/unhold (`deferred`), or
//! retire tasks directly from the Graph without waiting for an LLM replan.
//! Supports both Fast-Path (instant deterministic DAG save) and optional AI
//! Plan Review, with task-local acceptance criteria.

use super::editor::InputBuffer;
use super::theme::Theme;
use super::view::{TaskMeta, TaskRow, View, clean, truncate};
use crate::config::WorkKind;
use crate::lean::store::PlanRevision;
use crate::lean::tasks::{ChangeKind, ReviewerSpec, TaskContract};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::collections::BTreeMap;
use unicode_width::UnicodeWidthStr;

pub const WORK_KINDS: [WorkKind; 17] = [
    WorkKind::Implementation,
    WorkKind::Refactoring,
    WorkKind::Testing,
    WorkKind::TestDesign,
    WorkKind::StaticAnalysis,
    WorkKind::Architecture,
    WorkKind::Requirements,
    WorkKind::Options,
    WorkKind::Planning,
    WorkKind::Dependencies,
    WorkKind::Security,
    WorkKind::Performance,
    WorkKind::Integration,
    WorkKind::Research,
    WorkKind::Writing,
    WorkKind::Analysis,
    WorkKind::Delivery,
];

pub fn work_kind_name(kind: WorkKind) -> &'static str {
    match kind {
        WorkKind::Orchestration => "orchestration",
        WorkKind::Context => "context",
        WorkKind::Requirements => "requirements",
        WorkKind::Options => "options",
        WorkKind::Planning => "planning",
        WorkKind::Architecture => "architecture",
        WorkKind::Dependencies => "dependencies",
        WorkKind::Implementation => "implementation",
        WorkKind::Refactoring => "refactoring",
        WorkKind::StaticAnalysis => "static_analysis",
        WorkKind::TestDesign => "test_design",
        WorkKind::Testing => "testing",
        WorkKind::Security => "security",
        WorkKind::Performance => "performance",
        WorkKind::Integration => "integration",
        WorkKind::Research => "research",
        WorkKind::Writing => "writing",
        WorkKind::Analysis => "analysis",
        WorkKind::Delivery => "delivery",
    }
}

pub fn parse_work_kind(name: &str) -> WorkKind {
    let normalized = name.trim().to_ascii_lowercase().replace('-', "_");
    WORK_KINDS
        .iter()
        .copied()
        .find(|kind| work_kind_name(*kind) == normalized)
        .unwrap_or(WorkKind::Implementation)
}

fn work_kind_index(name: &str) -> usize {
    let kind = parse_work_kind(name);
    WORK_KINDS
        .iter()
        .position(|candidate| *candidate == kind)
        .unwrap_or(0)
}

fn next_change_kind(kind: ChangeKind, forward: bool) -> ChangeKind {
    match (kind, forward) {
        (ChangeKind::None, true) => ChangeKind::Code,
        (ChangeKind::Code, true) => ChangeKind::Artifact,
        (ChangeKind::Artifact, true) => ChangeKind::None,
        (ChangeKind::None, false) => ChangeKind::Artifact,
        (ChangeKind::Code, false) => ChangeKind::None,
        (ChangeKind::Artifact, false) => ChangeKind::Code,
    }
}

pub fn change_kind_label(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::None => "none (report only)",
        ChangeKind::Code => "code (isolated git worktree)",
        ChangeKind::Artifact => "artifact (declared output files)",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowEditMode {
    Edit { original_id: String },
    AddDependent { after_id: String },
    Split { original_id: String },
    Retire { task_id: String, task_title: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum FlowEditorOutcome {
    None,
    Close,
    Submit {
        revision: PlanRevision,
        request_ai_review: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlowEditor {
    pub mode: FlowEditMode,
    pub field: usize,
    pub id_input: InputBuffer,
    pub title_input: InputBuffer,
    pub goal_input: InputBuffer,
    pub kind_index: usize,
    pub change_kind: ChangeKind,
    pub depends_input: InputBuffer,
    pub outputs_input: InputBuffer,
    pub criteria_input: InputBuffer,
    pub deferred: bool,
    pub request_ai_review: bool,
    pub split_title_a: InputBuffer,
    pub split_goal_a: InputBuffer,
    pub split_title_b: InputBuffer,
    pub split_goal_b: InputBuffer,
    pub sending: bool,
    pub error: Option<String>,
    status_scroll: std::cell::Cell<usize>,
}

impl FlowEditor {
    pub fn edit_task(view: &View, task: &TaskRow) -> Self {
        let meta = view.task_meta.get(&task.id).cloned().unwrap_or_default();
        let goal = if task.goal.trim().is_empty() {
            task.title.clone()
        } else {
            task.goal.clone()
        };
        Self {
            mode: FlowEditMode::Edit {
                original_id: task.id.clone(),
            },
            field: 0,
            id_input: InputBuffer::with_text(task.id.clone()),
            title_input: InputBuffer::with_text(task.title.clone()),
            goal_input: InputBuffer::with_text(goal),
            kind_index: work_kind_index(&task.kind),
            change_kind: task.change_kind,
            depends_input: InputBuffer::with_text(task.depends_on.join(", ")),
            outputs_input: InputBuffer::with_text(meta.outputs.join(", ")),
            criteria_input: InputBuffer::with_text(
                meta.acceptance_criteria
                    .iter()
                    .map(|c| format!("{}: {}", c.id, c.text))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            deferred: meta.deferred,
            request_ai_review: false,
            split_title_a: InputBuffer::new(),
            split_goal_a: InputBuffer::new(),
            split_title_b: InputBuffer::new(),
            split_goal_b: InputBuffer::new(),
            sending: false,
            error: None,
            status_scroll: std::cell::Cell::new(0),
        }
    }

    pub fn add_dependent(view: &View, after_id: &str) -> Self {
        let next_id = suggest_next_task_id(view);
        let depends = if after_id.is_empty() || after_id == super::ORCHESTRATOR {
            String::new()
        } else {
            after_id.to_owned()
        };
        let inherited_criteria = view
            .task_meta
            .get(after_id)
            .map(|meta| {
                meta.acceptance_criteria
                    .iter()
                    .map(|c| format!("{}: {}", c.id, c.text))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        Self {
            mode: FlowEditMode::AddDependent {
                after_id: after_id.to_owned(),
            },
            field: 1, // Start directly on Title for fast entry
            id_input: InputBuffer::with_text(next_id),
            title_input: InputBuffer::new(),
            goal_input: InputBuffer::new(),
            kind_index: work_kind_index("implementation"),
            change_kind: ChangeKind::Code,
            depends_input: InputBuffer::with_text(depends),
            outputs_input: InputBuffer::new(),
            criteria_input: InputBuffer::with_text(inherited_criteria),
            deferred: false,
            request_ai_review: false,
            split_title_a: InputBuffer::new(),
            split_goal_a: InputBuffer::new(),
            split_title_b: InputBuffer::new(),
            split_goal_b: InputBuffer::new(),
            sending: false,
            error: None,
            status_scroll: std::cell::Cell::new(0),
        }
    }

    pub fn split_task(view: &View, task: &TaskRow) -> Self {
        let mut editor = Self::edit_task(view, task);
        editor.mode = FlowEditMode::Split {
            original_id: task.id.clone(),
        };
        editor.field = 0;
        let base_goal = if task.goal.trim().is_empty() {
            task.title.clone()
        } else {
            task.goal.clone()
        };
        editor.split_title_a = InputBuffer::with_text(format!("{} (part 1)", task.title));
        editor.split_goal_a = InputBuffer::with_text(base_goal.clone());
        editor.split_title_b = InputBuffer::with_text(format!("{} (part 2)", task.title));
        editor.split_goal_b = InputBuffer::with_text(base_goal);
        editor
    }

    pub fn retire_task(task: &TaskRow) -> Self {
        Self {
            mode: FlowEditMode::Retire {
                task_id: task.id.clone(),
                task_title: task.title.clone(),
            },
            field: 0,
            id_input: InputBuffer::with_text(task.id.clone()),
            title_input: InputBuffer::with_text(task.title.clone()),
            goal_input: InputBuffer::new(),
            kind_index: 0,
            change_kind: task.change_kind,
            depends_input: InputBuffer::new(),
            outputs_input: InputBuffer::new(),
            criteria_input: InputBuffer::new(),
            deferred: false,
            request_ai_review: false,
            split_title_a: InputBuffer::new(),
            split_goal_a: InputBuffer::new(),
            split_title_b: InputBuffer::new(),
            split_goal_b: InputBuffer::new(),
            sending: false,
            error: None,
            status_scroll: std::cell::Cell::new(0),
        }
    }

    fn field_count(&self) -> usize {
        match self.mode {
            FlowEditMode::Edit { .. } => 9,
            FlowEditMode::AddDependent { .. } => 10,
            FlowEditMode::Split { .. } => 5,
            FlowEditMode::Retire { .. } => 1,
        }
    }

    pub fn key(&mut self, key: KeyEvent, view: &View) -> FlowEditorOutcome {
        if self.error.is_some() && matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
            let offset = self.status_scroll.get();
            self.status_scroll.set(if key.code == KeyCode::PageUp {
                offset.saturating_sub(1)
            } else {
                offset.saturating_add(1)
            });
            return FlowEditorOutcome::None;
        }
        if self.sending {
            if key.code == KeyCode::Esc {
                self.sending = false;
                return FlowEditorOutcome::Close;
            }
            return FlowEditorOutcome::None;
        }

        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => return FlowEditorOutcome::Close,
            KeyCode::Char('s') if control => return self.submit(view),
            KeyCode::Enter if !alt => return self.submit(view),
            KeyCode::Tab | KeyCode::Down => {
                let count = self.field_count();
                self.field = (self.field + 1) % count;
                return FlowEditorOutcome::None;
            }
            KeyCode::BackTab | KeyCode::Up => {
                let count = self.field_count();
                self.field = (self.field + count - 1) % count;
                return FlowEditorOutcome::None;
            }
            _ => {}
        }

        self.error = None;
        self.status_scroll.set(0);
        match &self.mode {
            FlowEditMode::Edit { .. } => self.edit_field_key(key, false),
            FlowEditMode::AddDependent { .. } => self.edit_field_key(key, true),
            FlowEditMode::Split { .. } => self.split_field_key(key),
            FlowEditMode::Retire { .. } => {
                if matches!(
                    key.code,
                    KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right
                ) {
                    self.request_ai_review = !self.request_ai_review;
                }
            }
        }
        FlowEditorOutcome::None
    }

    fn edit_field_key(&mut self, key: KeyEvent, has_id_field: bool) {
        let index = if has_id_field {
            self.field
        } else {
            self.field + 1
        };
        match index {
            0 => self.id_input.edit_key(key),
            1 => self.title_input.edit_key(key),
            2 => self.goal_input.edit_key(key),
            3 => match key.code {
                KeyCode::Right | KeyCode::Char(' ') => {
                    self.kind_index = (self.kind_index + 1) % WORK_KINDS.len();
                }
                KeyCode::Left => {
                    self.kind_index = (self.kind_index + WORK_KINDS.len() - 1) % WORK_KINDS.len();
                }
                _ => {}
            },
            4 => match key.code {
                KeyCode::Right | KeyCode::Char(' ') => {
                    self.change_kind = next_change_kind(self.change_kind, true);
                }
                KeyCode::Left => {
                    self.change_kind = next_change_kind(self.change_kind, false);
                }
                _ => {}
            },
            5 => self.depends_input.edit_key(key),
            6 => self.outputs_input.edit_key(key),
            7 => self.criteria_input.edit_key(key),
            8 => {
                if matches!(
                    key.code,
                    KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right
                ) {
                    self.deferred = !self.deferred;
                }
            }
            9 => {
                if matches!(
                    key.code,
                    KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right
                ) {
                    self.request_ai_review = !self.request_ai_review;
                }
            }
            _ => {}
        }
    }

    fn split_field_key(&mut self, key: KeyEvent) {
        match self.field {
            0 => self.split_title_a.edit_key(key),
            1 => self.split_goal_a.edit_key(key),
            2 => self.split_title_b.edit_key(key),
            3 => self.split_goal_b.edit_key(key),
            4 => {
                if matches!(
                    key.code,
                    KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right
                ) {
                    self.request_ai_review = !self.request_ai_review;
                }
            }
            _ => {}
        }
    }

    fn submit(&mut self, view: &View) -> FlowEditorOutcome {
        self.status_scroll.set(0);
        match self.build_revision(view) {
            Ok((revision, request_ai_review)) => {
                self.error = None;
                self.sending = true;
                FlowEditorOutcome::Submit {
                    revision,
                    request_ai_review,
                }
            }
            Err(error) => {
                self.error = Some(error);
                FlowEditorOutcome::None
            }
        }
    }

    pub fn build_revision(&self, view: &View) -> std::result::Result<(PlanRevision, bool), String> {
        let plan = view
            .plan
            .clone()
            .ok_or_else(|| "No active plan do edycji.".to_owned())?;
        let base_revision = view.plan_head_revision;

        match &self.mode {
            FlowEditMode::Edit { original_id } => {
                let contract = self.build_single_contract(view, original_id)?;
                Ok((
                    PlanRevision {
                        scope: None,
                        plan,
                        base_revision,
                        reason: format!("Direct UI flow edit of task {original_id}"),
                        tasks: vec![contract],
                        retire: Vec::new(),
                        redo: Vec::new(),
                        replacements: BTreeMap::new(),
                    },
                    self.request_ai_review,
                ))
            }
            FlowEditMode::AddDependent { after_id } => {
                let new_id = self.id_input.text.trim();
                if new_id.is_empty() {
                    return Err("Task ID must not be empty.".into());
                }
                if view.tasks.iter().any(|t| t.id == new_id) {
                    return Err(format!("Task ID '{new_id}' already exists in the graph."));
                }
                let contract = self.build_single_contract(view, new_id)?;
                Ok((
                    PlanRevision {
                        scope: None,
                        plan,
                        base_revision,
                        reason: format!("Added dependent task {new_id} after {after_id} via UI"),
                        tasks: vec![contract],
                        retire: Vec::new(),
                        redo: Vec::new(),
                        replacements: BTreeMap::new(),
                    },
                    self.request_ai_review,
                ))
            }
            FlowEditMode::Split { original_id } => {
                let orig_row = view
                    .tasks
                    .iter()
                    .find(|t| &t.id == original_id)
                    .ok_or_else(|| format!("Task {original_id} was not found"))?;
                if orig_row.status == "approved" {
                    return Err("Approved tasks cannot be split; add a dependent task (a).".into());
                }
                let title_a = self.split_title_a.text.trim();
                let goal_a = self.split_goal_a.text.trim();
                let title_b = self.split_title_b.text.trim();
                let goal_b = self.split_goal_b.text.trim();
                if title_a.is_empty() || title_b.is_empty() {
                    return Err("Both split tasks need a title.".into());
                }
                let id_a = unique_split_id(view, original_id, "a");
                let id_b = unique_split_id(view, original_id, "b");
                let base = contract_from_view(view, orig_row);
                let mut part_a = base.clone();
                part_a.id = id_a.clone();
                part_a.title = title_a.to_owned();
                part_a.goal = if goal_a.is_empty() {
                    title_a.to_owned()
                } else {
                    goal_a.to_owned()
                };
                part_a.summary = None;
                part_a.reviewer = ReviewerSpec {
                    summary: Some(format!("Verify {title_a}")),
                };

                let mut part_b = base;
                part_b.id = id_b.clone();
                part_b.title = title_b.to_owned();
                part_b.goal = if goal_b.is_empty() {
                    title_b.to_owned()
                } else {
                    goal_b.to_owned()
                };
                part_b.depends_on = vec![id_a];
                part_b.summary = None;
                part_b.reviewer = ReviewerSpec {
                    summary: Some(format!("Verify {title_b}")),
                };

                let mut replacements = BTreeMap::new();
                replacements.insert(id_b, original_id.clone());
                Ok((
                    PlanRevision {
                        scope: None,
                        plan,
                        base_revision,
                        reason: format!("Split task {original_id} into two steps via UI"),
                        tasks: vec![part_a, part_b],
                        retire: vec![original_id.clone()],
                        redo: Vec::new(),
                        replacements,
                    },
                    self.request_ai_review,
                ))
            }
            FlowEditMode::Retire { task_id, .. } => {
                build_retire_revision(view, &plan, base_revision, task_id, self.request_ai_review)
            }
        }
    }

    fn build_single_contract(
        &self,
        view: &View,
        id: &str,
    ) -> std::result::Result<TaskContract, String> {
        let title = self.title_input.text.trim();
        if title.is_empty() {
            return Err("Task title must not be empty.".into());
        }
        let goal_trimmed = self.goal_input.text.trim();
        let goal = if goal_trimmed.is_empty() {
            title.to_owned()
        } else {
            goal_trimmed.to_owned()
        };
        let depends_on = split_csv(&self.depends_input.text);
        if depends_on.iter().any(|dep| dep == id) {
            return Err("A task cannot depend on itself.".into());
        }
        let outputs = split_csv(&self.outputs_input.text);
        if self.change_kind == ChangeKind::Artifact && outputs.is_empty() {
            return Err("Artifact delivery needs at least one output path.".into());
        }
        let acceptance_criteria = self
            .criteria_input
            .text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let (id, text) = line
                    .split_once(':')
                    .ok_or("Use C1: description for each criterion")?;
                if id.trim().is_empty() || text.trim().is_empty() {
                    return Err("Enter a criterion ID and description");
                }
                Ok(crate::lean::tasks::AcceptanceCriterion {
                    id: id.trim().into(),
                    text: text.trim().into(),
                })
            })
            .collect::<Result<Vec<_>, &str>>()
            .map_err(str::to_owned)?;
        if acceptance_criteria.is_empty() {
            return Err("Podaj przynajmniej jedno kryterium akceptacji".into());
        }
        let meta = view.task_meta.get(id).cloned().unwrap_or_default();
        let kind = WORK_KINDS
            .get(self.kind_index)
            .copied()
            .unwrap_or(WorkKind::Implementation);

        Ok(TaskContract {
            quality_gate: meta.quality_gate,
            integration_batch: meta.integration_batch,
            options_for: meta.options_for,
            open_decisions: if view.task_meta.contains_key(id) {
                meta.open_decisions
            } else {
                Some(Vec::new())
            },
            planning_inputs: meta.planning_inputs,
            id: id.to_owned(),
            checkout_name: self
                .change_kind
                .has_delivery()
                .then(|| crate::lean::tasks::checkout_slug(title)),

            title: title.to_owned(),
            goal,
            summary: None,
            section: None,
            deferred: self.deferred,
            change_kind: self.change_kind,
            outputs,
            delivery: None,
            depends_on,
            result_inputs: meta.result_inputs,
            kind,
            acceptance_criteria,
            reviewer: ReviewerSpec {
                summary: Some(format!("Verify {title}")),
            },
        })
    }

    pub fn draw(&self, frame: &mut Frame, theme: &Theme) {
        let separator = theme.chrome("·");
        let area = frame.area();
        if area.width < 12 || area.height < 5 {
            return;
        }

        match &self.mode {
            FlowEditMode::Edit { original_id } => {
                self.draw_contract_form(
                    frame,
                    area,
                    &format!(" Flow Editor {separator} edit task {original_id} "),
                    false,
                    theme,
                );
            }
            FlowEditMode::AddDependent { after_id } => {
                self.draw_contract_form(
                    frame,
                    area,
                    &format!(" Flow Editor {separator} add task after {after_id} "),
                    true,
                    theme,
                );
            }
            FlowEditMode::Split { original_id } => {
                self.draw_split_form(frame, area, original_id, theme);
            }
            FlowEditMode::Retire {
                task_id,
                task_title,
            } => {
                self.draw_retire_confirm(frame, area, task_id, task_title, theme);
            }
        }
    }

    fn draw_retire_confirm(
        &self,
        frame: &mut Frame,
        area: Rect,
        task_id: &str,
        task_title: &str,
        theme: &Theme,
    ) {
        let separator = theme.chrome("·");
        let width = area.width.saturating_sub(4).clamp(12, 72);
        let height = 10.min(area.height);
        let boxed = centered_rect(area, width, height);
        frame.render_widget(Clear, boxed);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(theme.borders())
            .border_style(theme.warning())
            .title(format!(" Flow Editor {separator} retire task {task_id} "));
        let inner = block.inner(boxed);
        frame.render_widget(block, boxed);
        let text_w = usize::from(inner.width.saturating_sub(2)).max(1);
        let mode_label = if self.request_ai_review {
            "AI plan review"
        } else {
            "Apply graph changes immediately"
        };
        let mut lines = vec![
            Line::from(Span::styled(
                truncate(
                    &format!("Retire task {task_id}: {}?", clean(task_title)),
                    text_w,
                ),
                theme.title().add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                truncate(
                    "Dependent tasks will reconnect to this task’s predecessors.",
                    text_w,
                ),
                theme.dim(),
            )),
            Line::raw(""),
            Line::from(vec![
                Span::styled("Save mode (Space): ", theme.dim()),
                Span::styled(
                    truncate(mode_label, text_w.saturating_sub(21)),
                    theme.accent().add_modifier(Modifier::BOLD),
                ),
            ]),
        ];
        if let Some(error) = &self.error {
            lines.push(Line::from(Span::styled(
                truncate(&format!("✗ {error}"), text_w),
                theme.error(),
            )));
        } else {
            lines.push(Line::raw(""));
        }
        lines.push(Line::from(Span::styled(
            truncate(
                &theme.chrome("Enter confirm · Space immediate/AI review · Esc cancel"),
                text_w,
            ),
            theme.dim(),
        )));
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn draw_split_form(&self, frame: &mut Frame, area: Rect, original_id: &str, theme: &Theme) {
        let separator = theme.chrome("·");
        let width = area.width.saturating_sub(4).clamp(12, 78);
        let height = 14.min(area.height);
        let boxed = centered_rect(area, width, height);
        frame.render_widget(Clear, boxed);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(theme.borders())
            .border_style(theme.separator(true))
            .title(format!(
                " Flow Editor {separator} split task {original_id} into two steps "
            ));
        let inner = block.inner(boxed);
        frame.render_widget(block, boxed);
        let text_w = usize::from(inner.width.saturating_sub(2)).max(1);
        let mode_label = if self.request_ai_review {
            "AI Plan Review"
        } else {
            "Apply immediately"
        };
        let fields = [
            ("1. Step A title", self.split_title_a.text.as_str()),
            ("2. Step A goal ", self.split_goal_a.text.as_str()),
            ("3. Step B title", self.split_title_b.text.as_str()),
            ("4. Step B goal ", self.split_goal_b.text.as_str()),
            ("5. Save mode   ", mode_label),
        ];
        let mut lines = Vec::new();
        for (idx, (label, val)) in fields.iter().enumerate() {
            let selected = self.field == idx;
            let prefix = if selected { "▸ " } else { "  " };
            let style = if selected {
                Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                theme.text()
            };
            lines.push(Line::from(vec![
                Span::styled(theme.chrome(&format!("{prefix}{label}: ")), theme.dim()),
                Span::styled(
                    truncate(val, text_w.saturating_sub(label.width() + 4)),
                    style,
                ),
            ]));
        }
        if let Some(error) = &self.error {
            lines.push(Line::from(Span::styled(
                truncate(&format!("✗ {error}"), text_w),
                theme.error(),
            )));
        } else {
            lines.push(Line::raw(""));
        }
        lines.push(Line::from(Span::styled(
            truncate(
                &theme.chrome("Tab/↑/↓ field · Enter/Ctrl+S save · Esc close"),
                text_w,
            ),
            theme.dim(),
        )));
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn draw_contract_form(
        &self,
        frame: &mut Frame,
        area: Rect,
        title: &str,
        has_id_field: bool,
        theme: &Theme,
    ) {
        let width = area.width.saturating_sub(4).clamp(12, 82);
        let height = 16.min(area.height);
        let boxed = centered_rect(area, width, height);
        frame.render_widget(Clear, boxed);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(theme.borders())
            .border_style(theme.separator(true))
            .title(truncate(title, usize::from(width.saturating_sub(2))));
        let inner = block.inner(boxed);
        frame.render_widget(block, boxed);

        let kind_str = work_kind_name(
            WORK_KINDS
                .get(self.kind_index)
                .copied()
                .unwrap_or(WorkKind::Implementation),
        );
        let change_str = change_kind_label(self.change_kind);
        let hold_str = if self.deferred {
            "Paused (held / deferred)"
        } else {
            "Active (schedulable)"
        };
        let mode_str = if self.request_ai_review {
            "AI plan review"
        } else {
            "Apply immediately without replanning"
        };

        let mut rows: Vec<(&str, String)> = Vec::new();
        if has_id_field {
            rows.push(("Task ID         ", self.id_input.text.clone()));
        }
        rows.push(("Title           ", self.title_input.text.clone()));
        rows.push(("Goal            ", self.goal_input.text.clone()));
        rows.push(("Kind (←/→)      ", kind_str.to_owned()));
        rows.push(("Delivery (←/→)  ", change_str.to_owned()));
        rows.push(("Dependencies    ", self.depends_input.text.clone()));
        rows.push(("Output paths    ", self.outputs_input.text.clone()));
        rows.push(("Criteria (C1: text) ", self.criteria_input.text.clone()));
        rows.push(("Status (Space)  ", hold_str.to_owned()));
        rows.push(("Save mode       ", mode_str.to_owned()));

        let compact = inner.height < rows.len() as u16 + 3 || inner.width < 40;
        let status_height = if self.error.is_some() { 2 } else { 1 };
        let fields_height = inner.height.saturating_sub(status_height + 1).max(1);
        let first = if compact { self.field } else { 0 };
        let mut lines = Vec::new();
        for (idx, (label, value)) in rows
            .iter()
            .enumerate()
            .skip(first)
            .take(usize::from(fields_height))
        {
            let selected = self.field == idx;
            let label = if compact {
                [
                    "ID", "Title", "Goal", "Kind", "Delivery", "Deps", "Paths", "Criteria",
                    "State", "Mode",
                ][idx + usize::from(!has_id_field)]
            } else {
                label.trim()
            };
            let prefix = format!("{label}: ");
            lines.push(Line::from(vec![
                Span::styled(prefix.clone(), theme.dim()),
                Span::styled(
                    truncate(
                        value,
                        usize::from(inner.width).saturating_sub(prefix.width()),
                    ),
                    if selected {
                        theme
                            .text()
                            .add_modifier(Modifier::BOLD | Modifier::REVERSED)
                    } else {
                        theme.text()
                    },
                ),
            ]));
        }
        frame.render_widget(
            Paragraph::new(lines),
            Rect {
                height: fields_height,
                ..inner
            },
        );
        let status = Rect {
            y: inner.y + fields_height,
            height: inner.height.saturating_sub(fields_height),
            ..inner
        };
        if let Some(error) = &self.error {
            let wrapped = super::theme::wrap(error, usize::from(inner.width).max(1));
            let visible = usize::from(status.height.saturating_sub(1)).max(1);
            let offset = self
                .status_scroll
                .get()
                .min(wrapped.len().saturating_sub(visible));
            self.status_scroll.set(offset);
            let mut lines = vec![Line::styled("Err PgUp/Dn", theme.error())];
            lines.extend(
                wrapped
                    .into_iter()
                    .skip(offset)
                    .take(visible)
                    .map(|line| Line::styled(line, theme.error())),
            );
            frame.render_widget(Paragraph::new(lines), status);
        } else {
            self.status_scroll.set(0);
            frame.render_widget(
                Paragraph::new(vec![
                    Line::styled(
                        if self.sending {
                            if inner.width >= 28 {
                                "Saving graph changes..."
                            } else {
                                "Saving..."
                            }
                        } else {
                            ""
                        },
                        theme.accent(),
                    ),
                    Line::styled(
                        truncate(
                            &theme.chrome("Tab field · Enter/Ctrl+S save · Esc close"),
                            usize::from(inner.width),
                        ),
                        theme.dim(),
                    ),
                ]),
                status,
            );
        }
    }
}

pub fn contract_from_view(view: &View, row: &TaskRow) -> TaskContract {
    let meta: TaskMeta = view.task_meta.get(&row.id).cloned().unwrap_or_default();
    let goal = if row.goal.trim().is_empty() {
        row.title.clone()
    } else {
        row.goal.clone()
    };
    TaskContract {
        quality_gate: meta.quality_gate,
        integration_batch: meta.integration_batch,
        options_for: meta.options_for,
        open_decisions: meta.open_decisions,
        planning_inputs: meta.planning_inputs,
        id: row.id.clone(),
        checkout_name: row
            .change_kind
            .has_delivery()
            .then(|| crate::lean::tasks::checkout_slug(&row.title)),

        acceptance_criteria: meta.acceptance_criteria,
        title: row.title.clone(),
        goal,
        summary: None,
        section: None,
        deferred: meta.deferred,
        change_kind: row.change_kind,
        outputs: meta.outputs,
        delivery: None,
        result_inputs: meta.result_inputs,
        depends_on: row.depends_on.clone(),
        kind: parse_work_kind(&row.kind),

        reviewer: ReviewerSpec {
            summary: Some(format!("Verify {}", row.title)),
        },
    }
}

pub fn toggle_hold_revision(
    view: &View,
    task: &TaskRow,
) -> std::result::Result<(PlanRevision, bool), String> {
    let plan = view
        .plan
        .clone()
        .ok_or_else(|| "No active plan do edycji.".to_owned())?;
    if task.status == "approved" {
        return Err("The task is already approved.".into());
    }
    let mut contract = contract_from_view(view, task);
    contract.deferred = !contract.deferred;
    let action = if contract.deferred {
        "Held"
    } else {
        "Released"
    };
    Ok((
        PlanRevision {
            scope: None,
            plan,
            base_revision: view.plan_head_revision,
            reason: format!("{action} task {} via UI fast-path", task.id),
            tasks: vec![contract],
            retire: Vec::new(),
            redo: Vec::new(),
            replacements: BTreeMap::new(),
        },
        false,
    ))
}

fn build_retire_revision(
    view: &View,
    plan: &str,
    base_revision: i64,
    task_id: &str,
    request_ai_review: bool,
) -> std::result::Result<(PlanRevision, bool), String> {
    let target = view
        .tasks
        .iter()
        .find(|t| t.id == task_id)
        .ok_or_else(|| format!("Task not found: {task_id}"))?;
    if target.status == "approved" {
        return Err("An approved task cannot be retired.".into());
    }
    if view.tasks.len() <= 1 {
        return Err("A plan must contain at least one task.".into());
    }
    let parent_deps = target.depends_on.clone();
    let mut rewired_tasks = Vec::new();
    for row in &view.tasks {
        if row.id == task_id || !row.depends_on.iter().any(|d| d == task_id) {
            continue;
        }
        let mut contract = contract_from_view(view, row);
        let mut new_deps = Vec::new();
        for dep in &row.depends_on {
            if dep == task_id {
                for p in &parent_deps {
                    if p != &row.id && !new_deps.contains(p) {
                        new_deps.push(p.clone());
                    }
                }
            } else if !new_deps.contains(dep) {
                new_deps.push(dep.clone());
            }
        }
        contract.depends_on = new_deps;
        rewired_tasks.push(contract);
    }
    Ok((
        PlanRevision {
            scope: None,
            plan: plan.to_owned(),
            base_revision,
            reason: format!("Retired task {task_id} via UI Flow Editor"),
            tasks: rewired_tasks,
            retire: vec![task_id.to_owned()],
            redo: Vec::new(),
            replacements: BTreeMap::new(),
        },
        request_ai_review,
    ))
}

/// Toggles a dependency edge between `from_id` and `to_id` directly on the graph (`l` key).
/// - If `to_id` already depends on `from_id`, removes `from_id` from `to_id.depends_on`.
/// - If `from_id` already depends on `to_id`, removes `to_id` from `from_id.depends_on`.
/// - Otherwise adds `from_id` to `to_id.depends_on` (creating edge `from_id → to_id`), rejecting cycles.
pub fn toggle_dependency_edge_revision(
    view: &View,
    from_id: &str,
    to_id: &str,
) -> std::result::Result<(PlanRevision, String), String> {
    if from_id == to_id {
        return Err("A task cannot depend on itself.".into());
    }
    let plan = view
        .plan
        .clone()
        .ok_or_else(|| "No active plan w tej konwersacji.".to_owned())?;
    let base_revision = view.plan_head_revision.max(1);
    let from_row = view
        .tasks
        .iter()
        .find(|t| t.id == from_id)
        .ok_or_else(|| format!("Source task not found: {from_id}"))?;
    let to_row = view
        .tasks
        .iter()
        .find(|t| t.id == to_id)
        .ok_or_else(|| format!("Task not found: docelowego {to_id}"))?;

    if to_row.depends_on.iter().any(|d| d == from_id) {
        if to_row.status == "approved" {
            return Err("Dependencies of approved tasks cannot be changed.".into());
        }
        let mut contract = contract_from_view(view, to_row);
        contract.depends_on.retain(|d| d != from_id);
        return Ok((
            PlanRevision {
                scope: None,
                plan,
                base_revision,
                reason: format!("Unlinked dependency {from_id} ↛ {to_id} via UI"),
                tasks: vec![contract],
                retire: Vec::new(),
                redo: Vec::new(),
                replacements: BTreeMap::new(),
            },
            format!("Unlinked dependency {from_id} ↛ {to_id}"),
        ));
    }

    if from_row.depends_on.iter().any(|d| d == to_id) {
        if from_row.status == "approved" {
            return Err("Dependencies of approved tasks cannot be changed.".into());
        }
        let mut contract = contract_from_view(view, from_row);
        contract.depends_on.retain(|d| d != to_id);
        return Ok((
            PlanRevision {
                scope: None,
                plan,
                base_revision,
                reason: format!("Unlinked dependency {to_id} ↛ {from_id} via UI"),
                tasks: vec![contract],
                retire: Vec::new(),
                redo: Vec::new(),
                replacements: BTreeMap::new(),
            },
            format!("Unlinked dependency {to_id} ↛ {from_id}"),
        ));
    }

    if to_row.status == "approved" {
        return Err("Dependencies of approved tasks cannot be changed.".into());
    }
    if depends_transitively(view, from_id, to_id) {
        return Err(format!(
            "Cannot link {from_id} → {to_id}: this would create a dependency cycle."
        ));
    }
    let mut contract = contract_from_view(view, to_row);
    contract.depends_on.push(from_id.to_owned());
    Ok((
        PlanRevision {
            scope: None,
            plan,
            base_revision,
            reason: format!("Linked dependency {from_id} → {to_id} via UI"),
            tasks: vec![contract],
            retire: Vec::new(),
            redo: Vec::new(),
            replacements: BTreeMap::new(),
        },
        format!("Linked dependency {from_id} → {to_id}"),
    ))
}

fn depends_transitively(view: &View, start_id: &str, target_id: &str) -> bool {
    let mut visited = std::collections::HashSet::new();
    let mut stack = vec![start_id];
    while let Some(current) = stack.pop() {
        if current == target_id {
            return true;
        }
        if !visited.insert(current) {
            continue;
        }
        if let Some(row) = view.tasks.iter().find(|t| t.id == current) {
            for dep in &row.depends_on {
                stack.push(dep.as_str());
            }
        }
    }
    false
}

pub fn is_unfinished_task(status: &str) -> bool {
    !matches!(status, "approved" | "skipped" | "cancelled")
}

pub fn downstream_unfinished_dependents(view: &View, task_id: &str) -> Vec<String> {
    let Some(task) = view.tasks.iter().find(|t| t.id == task_id) else {
        return Vec::new();
    };
    if !is_unfinished_task(&task.status) {
        return Vec::new();
    }
    view.tasks
        .iter()
        .filter(|other| {
            other.id != task_id
                && is_unfinished_task(&other.status)
                && depends_transitively(view, &other.id, task_id)
        })
        .map(|other| other.id.clone())
        .collect()
}

pub fn critical_path_tasks(view: &View) -> Vec<String> {
    let unfinished: Vec<&TaskRow> = view
        .tasks
        .iter()
        .filter(|t| is_unfinished_task(&t.status))
        .collect();
    if unfinished.len() < 2 {
        return Vec::new();
    }

    let mut children: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::with_capacity(unfinished.len());
    for task in &unfinished {
        children.entry(task.id.clone()).or_default();
    }
    for child in &unfinished {
        for parent in &child.depends_on {
            if child.id != *parent
                && let Some(parent_children) = children.get_mut(parent)
                && !parent_children.iter().any(|id| id == &child.id)
            {
                parent_children.push(child.id.clone());
            }
        }
    }

    // Compute each task's downstream count once. Calling the general-purpose
    // reachability helper for every candidate repeatedly rescans the graph,
    // which makes rendering a large plan prohibitively expensive.
    let mut downstream_counts = std::collections::HashMap::with_capacity(unfinished.len());
    for task in &unfinished {
        let mut seen = std::collections::HashSet::new();
        let mut pending = children.get(&task.id).cloned().unwrap_or_default();
        while let Some(child) = pending.pop() {
            if child == task.id {
                continue;
            }
            if seen.insert(child.clone())
                && let Some(grandchildren) = children.get(&child)
            {
                pending.extend(grandchildren.iter().cloned());
            }
        }
        downstream_counts.insert(task.id.clone(), seen.len());
    }

    let mut memo: std::collections::HashMap<String, (usize, Option<String>)> =
        std::collections::HashMap::new();
    let mut visiting: std::collections::HashSet<String> = std::collections::HashSet::new();

    fn longest_from(
        id: &str,
        children: &std::collections::HashMap<String, Vec<String>>,
        downstream_counts: &std::collections::HashMap<String, usize>,
        memo: &mut std::collections::HashMap<String, (usize, Option<String>)>,
        visiting: &mut std::collections::HashSet<String>,
    ) -> (usize, Option<String>) {
        if let Some(res) = memo.get(id) {
            return res.clone();
        }
        if !visiting.insert(id.to_owned()) {
            return (1, None);
        }
        let mut best_len = 1usize;
        let mut best_next: Option<String> = None;
        let mut best_downstream = 0usize;
        if let Some(children_for_id) = children.get(id) {
            for child in children_for_id {
                let (child_len, _) =
                    longest_from(child, children, downstream_counts, memo, visiting);
                let cand_len = 1 + child_len;
                let cand_down = downstream_counts.get(child).copied().unwrap_or_default();
                if (cand_len, cand_down) > (best_len, best_downstream) {
                    best_len = cand_len;
                    best_downstream = cand_down;
                    best_next = Some(child.clone());
                }
            }
        }
        visiting.remove(id);
        let res = (best_len, best_next);
        memo.insert(id.to_owned(), res.clone());
        res
    }

    let mut best_start: Option<String> = None;
    let mut best_score = (0usize, 0usize);
    for task in &unfinished {
        let (len, _) = longest_from(
            &task.id,
            &children,
            &downstream_counts,
            &mut memo,
            &mut visiting,
        );
        let down = downstream_counts.get(&task.id).copied().unwrap_or_default();
        if (len, down) > best_score {
            best_score = (len, down);
            best_start = Some(task.id.clone());
        }
    }
    if best_score.0 < 2 {
        return Vec::new();
    }
    let mut chain = Vec::new();
    let mut current = best_start;
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = current {
        if !seen.insert(id.clone()) {
            break;
        }
        let next = memo.get(&id).and_then(|(_, next)| next.clone());
        chain.push(id);
        current = next;
    }
    chain
}

pub fn format_clock_mm_ss(ms: u64) -> String {
    let total_secs = ms / 1000;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    if hours > 0 {
        format!("{hours:02}:{mins:02}:{secs:02}")
    } else {
        format!("{mins:02}:{secs:02}")
    }
}

fn suggest_next_task_id(view: &View) -> String {
    for n in 1..=999 {
        let candidate = format!("t{n}");
        if !view.tasks.iter().any(|t| t.id == candidate) {
            return candidate;
        }
    }
    format!("t{}", view.tasks.len() + 1)
}

fn unique_split_id(view: &View, base: &str, suffix: &str) -> String {
    let first = format!("{base}_{suffix}");
    if !view.tasks.iter().any(|t| t.id == first) {
        return first;
    }
    for n in 2..=99 {
        let candidate = format!("{base}_{suffix}{n}");
        if !view.tasks.iter().any(|t| t.id == candidate) {
            return candidate;
        }
    }
    format!("{base}_{suffix}_split")
}

fn split_csv(text: &str) -> Vec<String> {
    text.split([',', ' '])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let _ = Layout::vertical([Constraint::Min(0)]);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lean::tasks::Team;
    use crate::lean::tui::{App, Command, Focus, Kind};
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;

    fn sample_view_with_plan() -> View {
        let mut view = View::default();
        view.apply(&json!({
            "seq": 1,
            "kind": "plan_proposed",
            "payload": {
                "plan": "p1",
                "tasks": [
                    {
                        "id": "t1",
                        "title": "Implement core",
                        "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Core implementation",
                        "kind": "implementation",
                        "change_kind": "code",

                        "depends_on": []
                    },
                    {
                        "id": "t2",
                        "title": "Run tests",
                        "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Verify behavior",
                        "kind": "testing",
                        "change_kind": "none",

                        "depends_on": ["t1"],
                        "quality_gate": true
                    }
                ]
            }
        }));
        view
    }

    #[test]
    fn effective_team_uses_runtime_default_until_orchestrator_selects_one() {
        let mut view = View::default();
        assert_eq!(view.active_rigor(), Team::Feature);
        assert_eq!(view.rigor_label(), "orchestrator deciding");
        view.apply(&json!({
            "seq": 1,
            "kind": "team_selected",
            "payload": {
                "team": "feature",
                "reason": "Full ceremony requested"
            }
        }));
        assert_eq!(view.active_rigor(), Team::Feature);
        assert_eq!(view.rigor_label(), "feature");
        assert_eq!(view.team_reason, "Full ceremony requested");
    }

    #[test]
    fn slash_in_empty_chat_opens_slash_commands_and_preserves_non_matching_slash_input() {
        let mut app = App::test_app();
        app.set_focus(Focus::Chat);
        app.key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        assert_eq!(app.picker.as_ref().unwrap().kind, Kind::SlashCommands);

        // Typing a non-command path and pressing Enter inserts it into chat input
        for ch in "tmp/path".chars() {
            app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.picker.is_none());
        assert_eq!(app.input.text, "/tmp/path");
    }

    #[test]
    fn flow_editor_edits_adds_splits_and_retires_with_dependency_rewiring() {
        let view = sample_view_with_plan();
        let t1 = &view.tasks[0];

        // 1. Edit task in-place
        let mut edit = FlowEditor::edit_task(&view, t1);
        edit.title_input = InputBuffer::with_text("Updated core");
        edit.deferred = true;
        let (rev_edit, ai_review) = edit.build_revision(&view).unwrap();
        assert!(!ai_review);
        assert_eq!(rev_edit.tasks.len(), 1);
        assert_eq!(rev_edit.tasks[0].title, "Updated core");
        assert!(rev_edit.tasks[0].deferred);

        let mut completed = sample_view_with_plan();
        completed.tasks[0].status = "approved".into();
        let mut rename = FlowEditor::edit_task(&completed, &completed.tasks[0]);
        rename.title_input = InputBuffer::with_text("Clearer title");
        assert!(rename.build_revision(&completed).unwrap().0.redo.is_empty());

        // 2. Add an authored task; runtime-owned gates are regenerated by the core.
        let mut add = FlowEditor::add_dependent(&view, "t1");
        add.title_input = InputBuffer::with_text("Extra step");
        let (rev_add, _) = add.build_revision(&view).unwrap();
        assert!(
            rev_add
                .tasks
                .iter()
                .any(|t| t.id == "t3" && t.depends_on == vec!["t1"])
        );
        assert_eq!(rev_add.tasks.len(), 1);
        assert!(
            rev_add
                .tasks
                .iter()
                .all(|task| !task.quality_gate && task.options_for.is_none())
        );
        let directory = tempfile::tempdir().unwrap();
        let store = crate::lean::store::Store::open(&directory.path().join("core.db")).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        let specs = view
            .tasks
            .iter()
            .map(|row| serde_json::to_value(contract_from_view(&view, row)).unwrap())
            .collect::<Vec<_>>();
        let baseline = store.add_plan(&conversation.id, &specs).unwrap();
        assert!(rev_add.merge(&baseline).is_ok());

        // 3. Split task t1 into t1_a and t1_b
        let split = FlowEditor::split_task(&view, t1);
        let (rev_split, _) = split.build_revision(&view).unwrap();
        assert_eq!(rev_split.retire, vec!["t1"]);
        assert_eq!(rev_split.tasks.len(), 2);
        assert_eq!(rev_split.tasks[0].id, "t1_a");
        assert_eq!(rev_split.tasks[1].id, "t1_b");
        assert_eq!(rev_split.tasks[1].depends_on, vec!["t1_a"]);
        assert_eq!(
            rev_split.replacements.get("t1_b").map(String::as_str),
            Some("t1")
        );

        // 4. Retire task t1 rewires t2's dependency on t1
        let retire = FlowEditor::retire_task(t1);
        let (rev_retire, _) = retire.build_revision(&view).unwrap();
        assert_eq!(rev_retire.retire, vec!["t1"]);
        assert_eq!(rev_retire.tasks.len(), 1);
        assert_eq!(rev_retire.tasks[0].id, "t2");
        assert!(rev_retire.tasks[0].depends_on.is_empty());
    }

    #[test]
    fn visual_edge_linking_toggles_dependencies_and_rejects_cycles() {
        let view = sample_view_with_plan();
        // t2 currently depends on t1: toggling (t1, t2) unlinks t1 ↛ t2
        let (rev_unlink, msg) = toggle_dependency_edge_revision(&view, "t1", "t2").unwrap();
        assert!(msg.contains("Unlinked"));
        assert!(rev_unlink.tasks[0].depends_on.is_empty());

        // Self-loop is rejected
        assert!(toggle_dependency_edge_revision(&view, "t1", "t1").is_err());

        // Interactive keyboard flow on Graph (`l` -> Esc, and `l` -> navigate -> Enter)
        let mut app = App::test_app();
        app.view = sample_view_with_plan();
        app.set_focus(Focus::Graph);
        app.selected = "worker:t1".into();

        app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
        assert_eq!(app.graph_link_source.as_deref(), Some("t1"));

        let mut term = Terminal::new(TestBackend::new(120, 24)).unwrap();
        term.draw(|f| app.draw(f)).unwrap();
        let rendered: String = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(rendered.contains("LINKING FROM t1"));

        // Esc cancels link mode without leaving Graph focus
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.graph_link_source.is_none());
        assert_eq!(app.navigation.background_focus, Focus::Graph);
    }

    #[test]
    fn attention_inbox_mouse_click_opens_stopped_task_unblock_in_chat_and_graph_views() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut app = App::test_app();
        app.view = sample_view_with_plan();
        app.view.tasks[0].status = "failed".into();
        app.view.tasks[0].reason = Some("compile error in lib.rs".into());
        app.set_focus(Focus::Graph);
        app.selected = "worker:t1".into();

        // Even in narrow Graph-only view (width 70 < 85), Attention Inbox is rendered and clickable
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| app.draw(f)).unwrap();
        let area = app
            .inspector_hits
            .iter()
            .find_map(|(area, action)| {
                matches!(
                    action,
                    crate::lean::tui::work_units::InspectorAction::Recovery(_)
                )
                .then_some(*area)
            })
            .expect("recovery action rendered in inspector");

        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: area.x + 2,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.selected, "worker:t1");
        let unblock = app.unblock.as_ref().expect("unblock modal opened");
        assert_eq!(unblock.task, "t1");
    }

    #[test]
    fn direct_execute_command_inserts_slash_direct_when_input_is_empty() {
        let mut app = App::test_app();
        app.run_workflow(Command::DirectExecute);
        assert_eq!(app.input.text, "/direct ");
        assert_eq!(app.navigation.background_focus, Focus::Chat);
    }

    #[test]
    fn critical_path_bottleneck_and_live_timer_render_on_graph() {
        let mut app = App::test_app();
        app.view = sample_view_with_plan();
        app.view.tasks[0].status = "working".into();
        app.view.tasks[0].round = 1;
        let worker = app.view.agents.entry("worker:t1".into()).or_default();
        worker.running = true;
        worker.started_ms = Some(crate::lean::store::now_ms().saturating_sub(75_000));

        // t1 -> t2 is a critical path of length 2 where t1 blocks 1 downstream task (t2)
        assert_eq!(app.view.critical_path_chain(), vec!["t1", "t2"]);
        assert_eq!(app.view.task_bottleneck_count("t1"), 1);
        let clock = app
            .view
            .agent_live_clock("worker:t1", false)
            .expect("live clock available");
        assert!(clock.contains("01:1"));

        app.set_focus(Focus::Graph);
        app.selected = "worker:t1".into();
        let mut term = Terminal::new(TestBackend::new(140, 32)).unwrap();
        term.draw(|f| app.draw(f)).unwrap();
        let rendered: String = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            rendered.contains("Critical path · 1 downstream"),
            "{rendered}"
        );
        assert!(rendered.contains("01:"), "{rendered}");
    }
}
