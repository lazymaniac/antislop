//! Direct DAG flow editor (`D2` Plan & Flow Architecture): edit task contracts
//! in-place, add dependent tasks, split tasks, hold/unhold (`deferred`), or
//! retire tasks directly from the Graph without waiting for an LLM replan.
//! Supports both Fast-Path (instant deterministic DAG save) and optional AI
//! Plan Review, with interactive requirement coverage warnings.

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

pub const WORK_KINDS: [WorkKind; 18] = [
    WorkKind::Implementation,
    WorkKind::Refactoring,
    WorkKind::Testing,
    WorkKind::TestDesign,
    WorkKind::StaticAnalysis,
    WorkKind::Architecture,
    WorkKind::Context,
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
        allow_uncovered: bool,
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
    pub covers_input: InputBuffer,
    pub deferred: bool,
    pub request_ai_review: bool,
    pub split_title_a: InputBuffer,
    pub split_goal_a: InputBuffer,
    pub split_title_b: InputBuffer,
    pub split_goal_b: InputBuffer,
    pub sending: bool,
    pub error: Option<String>,
    pub coverage_warning: Option<String>,
    pub pending_revision: Option<PlanRevision>,
    pub pending_ai_review: bool,
    pub close_on_warning_cancel: bool,
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
            covers_input: InputBuffer::with_text(meta.covers.join(", ")),
            deferred: meta.deferred,
            request_ai_review: false,
            split_title_a: InputBuffer::new(),
            split_goal_a: InputBuffer::new(),
            split_title_b: InputBuffer::new(),
            split_goal_b: InputBuffer::new(),
            sending: false,
            error: None,
            coverage_warning: None,
            pending_revision: None,
            pending_ai_review: false,
            close_on_warning_cancel: false,
        }
    }

    pub fn add_dependent(view: &View, after_id: &str) -> Self {
        let next_id = suggest_next_task_id(view);
        let depends = if after_id.is_empty() || after_id == super::ORCHESTRATOR {
            String::new()
        } else {
            after_id.to_owned()
        };
        let inherited_covers = view
            .task_meta
            .get(after_id)
            .map(|meta| meta.covers.join(", "))
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
            covers_input: InputBuffer::with_text(inherited_covers),
            deferred: false,
            request_ai_review: false,
            split_title_a: InputBuffer::new(),
            split_goal_a: InputBuffer::new(),
            split_title_b: InputBuffer::new(),
            split_goal_b: InputBuffer::new(),
            sending: false,
            error: None,
            coverage_warning: None,
            pending_revision: None,
            pending_ai_review: false,
            close_on_warning_cancel: false,
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
            covers_input: InputBuffer::new(),
            deferred: false,
            request_ai_review: false,
            split_title_a: InputBuffer::new(),
            split_goal_a: InputBuffer::new(),
            split_title_b: InputBuffer::new(),
            split_goal_b: InputBuffer::new(),
            sending: false,
            error: None,
            coverage_warning: None,
            pending_revision: None,
            pending_ai_review: false,
            close_on_warning_cancel: false,
        }
    }

    pub fn coverage_warning_modal(
        task_id: String,
        task_title: String,
        revision: PlanRevision,
        request_ai_review: bool,
        warning: String,
    ) -> Self {
        let mut editor = Self::retire_task(&TaskRow {
            id: task_id,
            title: task_title,
            ..TaskRow::default()
        });
        editor.coverage_warning = Some(warning);
        editor.pending_revision = Some(revision);
        editor.pending_ai_review = request_ai_review;
        editor.close_on_warning_cancel = true;
        editor
    }

    fn field_count(&self) -> usize {
        match self.mode {
            FlowEditMode::Edit { .. } => 8,
            FlowEditMode::AddDependent { .. } => 9,
            FlowEditMode::Split { .. } => 5,
            FlowEditMode::Retire { .. } => 1,
        }
    }

    pub fn key(&mut self, key: KeyEvent, view: &View) -> FlowEditorOutcome {
        if self.sending {
            if key.code == KeyCode::Esc {
                self.sending = false;
                return FlowEditorOutcome::Close;
            }
            return FlowEditorOutcome::None;
        }

        if self.coverage_warning.is_some() {
            return self.warning_key(key);
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

    fn warning_key(&mut self, key: KeyEvent) -> FlowEditorOutcome {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y' | 'Y') => {
                if let Some(revision) = self.pending_revision.clone() {
                    self.coverage_warning = None;
                    self.sending = true;
                    return FlowEditorOutcome::Submit {
                        revision,
                        request_ai_review: self.pending_ai_review,
                        allow_uncovered: true,
                    };
                }
                FlowEditorOutcome::Close
            }
            KeyCode::Esc | KeyCode::Char('n' | 'N') => {
                self.coverage_warning = None;
                if self.close_on_warning_cancel {
                    FlowEditorOutcome::Close
                } else {
                    FlowEditorOutcome::None
                }
            }
            _ => FlowEditorOutcome::None,
        }
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
            7 => self.covers_input.edit_key(key),
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
        match self.build_revision(view) {
            Ok((revision, request_ai_review)) => {
                self.error = None;
                self.pending_revision = Some(revision.clone());
                self.pending_ai_review = request_ai_review;
                self.sending = true;
                FlowEditorOutcome::Submit {
                    revision,
                    request_ai_review,
                    allow_uncovered: false,
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
            .ok_or_else(|| "Brak aktywnego planu do edycji.".to_owned())?;
        let base_revision = view.plan_head_revision;

        match &self.mode {
            FlowEditMode::Edit { original_id } => {
                let contract = self.build_single_contract(view, original_id)?;
                let mut redo = Vec::new();
                if view
                    .tasks
                    .iter()
                    .any(|t| &t.id == original_id && t.status == "approved")
                {
                    redo.push(original_id.clone());
                }
                Ok((
                    PlanRevision {
                        plan,
                        base_revision,
                        reason: format!("Direct UI flow edit of task {original_id}"),
                        tasks: vec![contract],
                        retire: Vec::new(),
                        redo,
                        replacements: BTreeMap::new(),
                    },
                    self.request_ai_review,
                ))
            }
            FlowEditMode::AddDependent { after_id } => {
                let new_id = self.id_input.text.trim();
                if new_id.is_empty() {
                    return Err("ID zadania nie może być puste.".into());
                }
                if view.tasks.iter().any(|t| t.id == new_id) {
                    return Err(format!("Zadanie o ID '{new_id}' już istnieje w grafie."));
                }
                let contract = self.build_single_contract(view, new_id)?;
                let mut tasks = vec![contract];
                // Ensure any quality_gate task at the end of the graph also depends on the new task
                for row in &view.tasks {
                    let meta = view.task_meta.get(&row.id).cloned().unwrap_or_default();
                    if meta.quality_gate
                        && row.id != new_id
                        && !row.depends_on.contains(&new_id.to_owned())
                    {
                        let mut gate = contract_from_view(view, row);
                        gate.depends_on.push(new_id.to_owned());
                        tasks.push(gate);
                    }
                }
                Ok((
                    PlanRevision {
                        plan,
                        base_revision,
                        reason: format!("Added dependent task {new_id} after {after_id} via UI"),
                        tasks,
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
                    .ok_or_else(|| format!("Nie znaleziono zadania {original_id}"))?;
                if orig_row.status == "approved" {
                    return Err(
                        "Ukończonego (approved) zadania nie można podzielić; dodaj zadanie zależne (a)."
                            .into(),
                    );
                }
                let title_a = self.split_title_a.text.trim();
                let goal_a = self.split_goal_a.text.trim();
                let title_b = self.split_title_b.text.trim();
                let goal_b = self.split_goal_b.text.trim();
                if title_a.is_empty() || title_b.is_empty() {
                    return Err("Tytuły obu części dzielonego zadania są wymagane.".into());
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
            return Err("Tytuł zadania nie może być pusty.".into());
        }
        let goal_trimmed = self.goal_input.text.trim();
        let goal = if goal_trimmed.is_empty() {
            title.to_owned()
        } else {
            goal_trimmed.to_owned()
        };
        let depends_on = split_csv(&self.depends_input.text);
        if depends_on.iter().any(|dep| dep == id) {
            return Err("Zadanie nie może zależeć od samego siebie.".into());
        }
        let outputs = split_csv(&self.outputs_input.text);
        if self.change_kind == ChangeKind::Artifact && outputs.is_empty() {
            return Err(
                "Dla change_kind = artifact podaj przynajmniej jedną ścieżkę w Outputs.".into(),
            );
        }
        let covers = split_csv(&self.covers_input.text);
        let meta = view.task_meta.get(id).cloned().unwrap_or_default();
        let kind = WORK_KINDS
            .get(self.kind_index)
            .copied()
            .unwrap_or(WorkKind::Implementation);

        Ok(TaskContract {
            quality_gate: meta.quality_gate,
            options_for: meta.options_for,
            id: id.to_owned(),
            requirement_ids: Vec::new(),
            criterion_ids: Vec::new(),
            title: title.to_owned(),
            goal,
            summary: None,
            section: None,
            deferred: self.deferred,
            change_kind: self.change_kind,
            outputs,
            delivery: None,
            depends_on,
            kind,
            covers,
            reviewer: ReviewerSpec {
                summary: Some(format!("Verify {title}")),
            },
        })
    }

    pub fn draw(&self, frame: &mut Frame, theme: &Theme) {
        let area = frame.area();
        if area.width < 12 || area.height < 5 {
            return;
        }
        if let Some(warning) = &self.coverage_warning {
            self.draw_coverage_warning(frame, area, warning, theme);
            return;
        }
        match &self.mode {
            FlowEditMode::Edit { original_id } => {
                self.draw_contract_form(
                    frame,
                    area,
                    &format!(" Flow Editor · edit task {original_id} "),
                    false,
                    theme,
                );
            }
            FlowEditMode::AddDependent { after_id } => {
                self.draw_contract_form(
                    frame,
                    area,
                    &format!(" Flow Editor · add task after {after_id} "),
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

    fn draw_coverage_warning(&self, frame: &mut Frame, area: Rect, warning: &str, theme: &Theme) {
        let width = area.width.saturating_sub(4).clamp(12, 76);
        let height = 11.min(area.height);
        let boxed = centered_rect(area, width, height);
        frame.render_widget(Clear, boxed);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme.warning())
            .title(" ! Ostrzeżenie o pokryciu wymagań (Coverage Warning) ");
        let inner = block.inner(boxed);
        frame.render_widget(block, boxed);
        let text_w = usize::from(inner.width.saturating_sub(2)).max(1);
        let mut lines = vec![
            Line::from(Span::styled(
                truncate(
                    "Edycja pozostawia obowiązkowe kryteria bez przypisanego zadania:",
                    text_w,
                ),
                theme.title().add_modifier(Modifier::BOLD),
            )),
            Line::raw(""),
        ];
        for wrapped in super::view::wrap(&clean(warning), text_w)
            .into_iter()
            .take(4)
        {
            lines.push(Line::from(Span::styled(wrapped, theme.warning())));
        }
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled(" [Enter / y] Zapisz mimo to ", theme.button()),
            Span::raw("  "),
            Span::styled(" [Esc / n] Wróć do edycji ", theme.dim()),
        ]));
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn draw_retire_confirm(
        &self,
        frame: &mut Frame,
        area: Rect,
        task_id: &str,
        task_title: &str,
        theme: &Theme,
    ) {
        let width = area.width.saturating_sub(4).clamp(12, 72);
        let height = 10.min(area.height);
        let boxed = centered_rect(area, width, height);
        frame.render_widget(Clear, boxed);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme.warning())
            .title(format!(" Flow Editor · usuń zadanie {task_id} "));
        let inner = block.inner(boxed);
        frame.render_widget(block, boxed);
        let text_w = usize::from(inner.width.saturating_sub(2)).max(1);
        let mode_label = if self.request_ai_review {
            "AI Plan Review (pełna weryfikacja)"
        } else {
            "Fast-Path (natychmiastowy zapis DAG)"
        };
        let mut lines = vec![
            Line::from(Span::styled(
                truncate(
                    &format!("Wycofać zadanie {task_id}: {}?", clean(task_title)),
                    text_w,
                ),
                theme.title().add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                truncate(
                    "Zadania zależne zostaną automatycznie przepięte do poprzedników.",
                    text_w,
                ),
                theme.dim(),
            )),
            Line::raw(""),
            Line::from(vec![
                Span::styled("Tryb zapisu (Space): ", theme.dim()),
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
                "Enter potwierdź · Space przełącz Fast-Path/AI · Esc anuluj",
                text_w,
            ),
            theme.dim(),
        )));
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn draw_split_form(&self, frame: &mut Frame, area: Rect, original_id: &str, theme: &Theme) {
        let width = area.width.saturating_sub(4).clamp(12, 78);
        let height = 14.min(area.height);
        let boxed = centered_rect(area, width, height);
        frame.render_widget(Clear, boxed);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(super::theme::ayu::BLUE))
            .title(format!(
                " Flow Editor · podziel zadanie {original_id} na 2 kroki "
            ));
        let inner = block.inner(boxed);
        frame.render_widget(block, boxed);
        let text_w = usize::from(inner.width.saturating_sub(2)).max(1);
        let mode_label = if self.request_ai_review {
            "AI Plan Review"
        } else {
            "Fast-Path (natychmiast)"
        };
        let fields = [
            ("1. Tytuł kroku A", self.split_title_a.text.as_str()),
            ("2. Cel kroku A  ", self.split_goal_a.text.as_str()),
            ("3. Tytuł kroku B", self.split_title_b.text.as_str()),
            ("4. Cel kroku B  ", self.split_goal_b.text.as_str()),
            ("5. Tryb zapisu  ", mode_label),
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
                Span::styled(format!("{prefix}{label}: "), theme.dim()),
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
            truncate("Tab/↑/↓ pole · Enter/Ctrl+S zapisz · Esc zamknij", text_w),
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
            .border_style(Style::default().fg(super::theme::ayu::BLUE))
            .title(truncate(title, usize::from(width.saturating_sub(2))));
        let inner = block.inner(boxed);
        frame.render_widget(block, boxed);
        let text_w = usize::from(inner.width.saturating_sub(2)).max(1);

        let kind_str = work_kind_name(
            WORK_KINDS
                .get(self.kind_index)
                .copied()
                .unwrap_or(WorkKind::Implementation),
        );
        let change_str = change_kind_label(self.change_kind);
        let hold_str = if self.deferred {
            "⏸ wstrzymane (held / deferred)"
        } else {
            "▶ aktywne (schedulable)"
        };
        let mode_str = if self.request_ai_review {
            "AI Plan Review (pełny przegląd)"
        } else {
            "⚡ Fast-Path (natychmiastowy zapis bez replanowania)"
        };

        let mut rows: Vec<(&str, String)> = Vec::new();
        if has_id_field {
            rows.push(("ID zadania      ", self.id_input.text.clone()));
        }
        rows.push(("Tytuł           ", self.title_input.text.clone()));
        rows.push(("Cel (goal)      ", self.goal_input.text.clone()));
        rows.push(("Rodzaj (←/→)    ", kind_str.to_owned()));
        rows.push(("Dostawa (←/→)   ", change_str.to_owned()));
        rows.push(("Zależności (ID) ", self.depends_input.text.clone()));
        rows.push(("Pliki wyjściowe ", self.outputs_input.text.clone()));
        rows.push(("Kryteria (R1..) ", self.covers_input.text.clone()));
        rows.push(("Status (Space)  ", hold_str.to_owned()));
        rows.push(("Tryb zapisu     ", mode_str.to_owned()));

        let mut lines = Vec::new();
        for (idx, (label, value)) in rows.iter().enumerate() {
            let selected = self.field == idx;
            let marker = if selected { "▸ " } else { "  " };
            let val_style = if selected {
                Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                theme.text()
            };
            let display_val = if value.is_empty() && selected {
                " ".to_owned()
            } else {
                value.clone()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{marker}{label}: "), theme.dim()),
                Span::styled(
                    truncate(&display_val, text_w.saturating_sub(label.width() + 4)),
                    val_style,
                ),
            ]));
        }

        if let Some(error) = &self.error {
            lines.push(Line::from(Span::styled(
                truncate(&format!("✗ {error}"), text_w),
                theme.error(),
            )));
        } else if self.sending {
            lines.push(Line::from(Span::styled(
                truncate("Zapisywanie zmian w grafie…", text_w),
                theme.accent(),
            )));
        } else {
            lines.push(Line::raw(""));
        }
        lines.push(Line::from(Span::styled(
            truncate(
                "Tab/↑/↓ pole · ←/→/Space przełącz · Enter/Ctrl+S zapisz · Esc zamknij",
                text_w,
            ),
            theme.dim(),
        )));

        frame.render_widget(Paragraph::new(lines), inner);
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
        options_for: meta.options_for,
        id: row.id.clone(),
        requirement_ids: Vec::new(),
        criterion_ids: Vec::new(),
        title: row.title.clone(),
        goal,
        summary: None,
        section: None,
        deferred: meta.deferred,
        change_kind: row.change_kind,
        outputs: meta.outputs,
        delivery: None,
        depends_on: row.depends_on.clone(),
        kind: parse_work_kind(&row.kind),
        covers: meta.covers,
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
        .ok_or_else(|| "Brak aktywnego planu do edycji.".to_owned())?;
    if task.status == "approved" {
        return Err("Zadanie jest już ukończone (approved).".into());
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
        .ok_or_else(|| format!("Nie znaleziono zadania {task_id}"))?;
    if target.status == "approved" {
        return Err("Nie można wycofać ukończonego (approved) zadania.".into());
    }
    if view.tasks.len() <= 1 {
        return Err("Plan musi zawierać co najmniej jedno zadanie.".into());
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
        return Err("Zadanie nie może zależeć od samego siebie.".into());
    }
    let plan = view
        .plan
        .clone()
        .ok_or_else(|| "Brak aktywnego planu w tej konwersacji.".to_owned())?;
    let base_revision = view.plan_head_revision.max(1);
    let from_row = view
        .tasks
        .iter()
        .find(|t| t.id == from_id)
        .ok_or_else(|| format!("Nie znaleziono zadania źródłowego {from_id}"))?;
    let to_row = view
        .tasks
        .iter()
        .find(|t| t.id == to_id)
        .ok_or_else(|| format!("Nie znaleziono zadania docelowego {to_id}"))?;

    if to_row.depends_on.iter().any(|d| d == from_id) {
        if to_row.status == "approved" {
            return Err("Ukończonego (approved) zadania nie można przepinać.".into());
        }
        let mut contract = contract_from_view(view, to_row);
        contract.depends_on.retain(|d| d != from_id);
        return Ok((
            PlanRevision {
                plan,
                base_revision,
                reason: format!("Unlinked dependency {from_id} ↛ {to_id} via UI"),
                tasks: vec![contract],
                retire: Vec::new(),
                redo: Vec::new(),
                replacements: BTreeMap::new(),
            },
            format!("Rozłączono krawędź {from_id} ↛ {to_id}"),
        ));
    }

    if from_row.depends_on.iter().any(|d| d == to_id) {
        if from_row.status == "approved" {
            return Err("Ukończonego (approved) zadania nie można przepinać.".into());
        }
        let mut contract = contract_from_view(view, from_row);
        contract.depends_on.retain(|d| d != to_id);
        return Ok((
            PlanRevision {
                plan,
                base_revision,
                reason: format!("Unlinked dependency {to_id} ↛ {from_id} via UI"),
                tasks: vec![contract],
                retire: Vec::new(),
                redo: Vec::new(),
                replacements: BTreeMap::new(),
            },
            format!("Rozłączono krawędź {to_id} ↛ {from_id}"),
        ));
    }

    if to_row.status == "approved" {
        return Err("Ukończonego (approved) zadania nie można przepinać.".into());
    }
    if depends_transitively(view, from_id, to_id) {
        return Err(format!(
            "Nie można dodać krawędzi {from_id} → {to_id}: powstałby cykl w grafie."
        ));
    }
    let mut contract = contract_from_view(view, to_row);
    contract.depends_on.push(from_id.to_owned());
    Ok((
        PlanRevision {
            plan,
            base_revision,
            reason: format!("Linked dependency {from_id} → {to_id} via UI"),
            tasks: vec![contract],
            retire: Vec::new(),
            redo: Vec::new(),
            replacements: BTreeMap::new(),
        },
        format!("Połączono krawędź {from_id} → {to_id}"),
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
    use crate::lean::tui::{App, Command, Focus, Kind, Update};
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
                        "goal": "Core implementation",
                        "kind": "implementation",
                        "change_kind": "code",
                        "covers": ["R1.1"],
                        "depends_on": []
                    },
                    {
                        "id": "t2",
                        "title": "Run tests",
                        "goal": "Verify behavior",
                        "kind": "testing",
                        "change_kind": "none",
                        "covers": ["R1.2"],
                        "depends_on": ["t1"],
                        "quality_gate": true
                    }
                ]
            }
        }));
        view
    }

    #[test]
    fn default_effective_team_is_lighter_small_change_and_updates_on_team_selected() {
        let mut view = View::default();
        assert_eq!(view.active_rigor(), Team::SmallChange);
        view.apply(&json!({
            "seq": 1,
            "kind": "team_selected",
            "payload": {
                "team": "feature",
                "reason": "Full ceremony requested"
            }
        }));
        assert_eq!(view.active_rigor(), Team::Feature);
        assert_eq!(view.team_reason, "Full ceremony requested");
    }

    #[test]
    fn slash_in_empty_chat_opens_slash_commands_and_preserves_non_matching_slash_input() {
        let mut app = App::test_app();
        app.focus = Focus::Chat;
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

        // 2. Add dependent task after t1 (automatically wires quality_gate t2 to depend on t3)
        let mut add = FlowEditor::add_dependent(&view, "t1");
        add.title_input = InputBuffer::with_text("Extra step");
        let (rev_add, _) = add.build_revision(&view).unwrap();
        assert!(
            rev_add
                .tasks
                .iter()
                .any(|t| t.id == "t3" && t.depends_on == vec!["t1"])
        );
        assert!(
            rev_add
                .tasks
                .iter()
                .any(|t| t.id == "t2" && t.depends_on.contains(&"t3".to_owned()))
        );

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
    fn coverage_warning_modal_allows_user_to_confirm_or_return_to_editing() {
        let mut app = App::test_app();
        app.view = sample_view_with_plan();
        app.focus = Focus::Graph;
        app.selected = "worker:t1".into();
        app.key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        assert!(app.flow_editor.is_some());

        let (revision, _) = app
            .flow_editor
            .as_ref()
            .unwrap()
            .build_revision(&app.view)
            .unwrap();
        app.apply_update(Update::PlanRevisionResult {
            conversation: app.conversation.clone(),
            generation: app.generation,
            revision: revision.clone(),
            request_ai_review: false,
            task_id: "t1".into(),
            task_title: "Implement core".into(),
            reply: Ok(json!({
                "status": "coverage_warning",
                "warning": "Criterion R1.1 is not covered by any active task."
            })),
        });

        let editor = app.flow_editor.as_ref().unwrap();
        assert!(editor.coverage_warning.as_deref().unwrap().contains("R1.1"));

        // Esc cancels warning and returns to the editor form without losing draft
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.flow_editor.is_some());
        assert!(app.flow_editor.as_ref().unwrap().coverage_warning.is_none());

        // Trigger warning again and confirm with 'y' -> submits with allow_uncovered: true
        let editor = app.flow_editor.as_mut().unwrap();
        editor.coverage_warning = Some("Criterion R1.1 uncovered".into());
        editor.pending_revision = Some(revision);
        let outcome = editor.key(
            KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
            &app.view,
        );
        match outcome {
            FlowEditorOutcome::Submit {
                allow_uncovered, ..
            } => assert!(allow_uncovered),
            other => panic!("expected Submit with allow_uncovered=true, got {other:?}"),
        }
    }

    #[test]
    fn docked_workbench_and_flow_editor_render_safely_at_wide_and_tiny_sizes() {
        let mut app = App::test_app();
        app.view = sample_view_with_plan();
        app.focus = Focus::Graph;
        app.selected = "worker:t1".into();
        app.run_workflow(Command::ToggleWorkbench);
        assert!(app.workbench_docked);

        for (w, h) in [(140, 30), (120, 24), (80, 24), (12, 5), (4, 3)] {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| app.draw(f)).unwrap();
        }

        let mut term = Terminal::new(TestBackend::new(140, 30)).unwrap();
        term.draw(|f| app.draw(f)).unwrap();
        let rendered: String = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(rendered.contains("workbench · t1"));
        assert!(rendered.contains("unlocks -> t2"));
    }

    #[test]
    fn visual_edge_linking_toggles_dependencies_and_rejects_cycles() {
        let view = sample_view_with_plan();
        // t2 currently depends on t1: toggling (t1, t2) unlinks t1 ↛ t2
        let (rev_unlink, msg) = toggle_dependency_edge_revision(&view, "t1", "t2").unwrap();
        assert!(msg.contains("Rozłączono"));
        assert!(rev_unlink.tasks[0].depends_on.is_empty());

        // Self-loop is rejected
        assert!(toggle_dependency_edge_revision(&view, "t1", "t1").is_err());

        // Interactive keyboard flow on Graph (`l` -> Esc, and `l` -> navigate -> Enter)
        let mut app = App::test_app();
        app.view = sample_view_with_plan();
        app.focus = Focus::Graph;
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
        assert_eq!(app.focus, Focus::Graph);
    }

    #[test]
    fn workbench_shows_live_pty_and_mini_diff_and_handles_mouse_clicks() {
        use crate::lean::terminal::{Cell, Screen};
        use crate::lean::tui::native_view::Snapshot;
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut app = App::test_app();
        app.view = sample_view_with_plan();
        app.focus = Focus::Graph;
        app.selected = "worker:t1".into();
        app.workbench_docked = true;

        // Populate live PTY screen in details snapshot
        let mut details = crate::lean::tui::Details::new("worker:t1".into());
        details.native.insert(
            "worker:t1".into(),
            Snapshot {
                screen: Some(Screen {
                    handle: "h1".into(),
                    revision: 1,
                    columns: 40,
                    rows: 5,
                    cursor: None,
                    cells: vec![Cell {
                        x: 0,
                        y: 0,
                        text: "cargo test --all-targets PASS".into(),
                        width: 29,
                        foreground: None,
                        background: None,
                        modifiers: 0,
                    }],
                    error: None,
                }),
                error: None,
            },
        );
        app.details = Some(details);

        // Populate DiffView for t1
        let mut diff = crate::lean::tui::DiffView::new("t1".into());
        diff.load(Ok(json!({
            "base": "aaaa1111",
            "head": "bbbb2222",
            "truncated": false,
            "files": [{
                "path": "src/main.rs",
                "old_path": null,
                "status": "modified",
                "added": 1,
                "removed": 0,
                "binary": false,
                "lines": [
                    {"kind": "add", "old": null, "new": 10, "text": "fn fast_path() {}"}
                ]
            }]
        })));
        app.diff = Some(diff);

        // Render main screen (close modal details/diff first or test live_pty_tail_for_task + mini_preview)
        assert_eq!(
            app.live_pty_tail_for_task("t1", 3),
            vec!["cargo test --all-targets PASS"]
        );
        let preview = app.diff.as_ref().unwrap().mini_preview(4);
        assert_eq!(preview.len(), 2);
        assert!(preview[0].1.contains("src/main.rs (+1/-0)"));
        assert!(preview[1].1.contains("+fn fast_path() {}"));

        // Now test mouse clicks on the 3 workbench sections when main view is drawn
        app.details = None;
        app.diff = None;
        let mut term = Terminal::new(TestBackend::new(140, 30)).unwrap();
        term.draw(|f| app.draw(f)).unwrap();
        assert!(app.workbench_area.is_some());

        let sec1 = app.workbench_sections[1];
        app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: sec1.x + 2,
            row: sec1.y + 1,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.graph_expanded.as_deref(), Some("worker:t1"));
        assert_eq!(app.graph_tab, 1);
    }

    #[test]
    fn attention_inbox_mouse_click_opens_stopped_task_unblock_in_chat_and_graph_views() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut app = App::test_app();
        app.view = sample_view_with_plan();
        app.view.tasks[0].status = "failed".into();
        app.view.tasks[0].reason = Some("compile error in lib.rs".into());
        app.focus = Focus::Graph;
        app.selected = "orchestrator".into();

        // Even in narrow Graph-only view (width 70 < 85), Attention Inbox is rendered and clickable
        let mut term = Terminal::new(TestBackend::new(70, 24)).unwrap();
        term.draw(|f| app.draw(f)).unwrap();
        let area = app
            .phase_action_area
            .expect("phase_action_area rendered in graph view");

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
        assert_eq!(app.focus, Focus::Chat);
    }
}
