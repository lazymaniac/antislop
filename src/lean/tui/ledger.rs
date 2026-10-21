//! Full-screen obligation browser and field editor. Drafts survive closing the
//! editor, record switches, conversation switches, and failed asynchronous saves.
use super::{InputBuffer, Theme, sanitize, view::wrap};
use crate::lean::{daemon::Request, ledger::*};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Block, Borders, Paragraph},
};
use serde_json::Value;
use std::collections::HashMap;

const LABELS: [&str; 17] = [
    "ID",
    "Interpretation",
    "Qualifiers (one per line)",
    "Criteria (ID | required/optional | integration/check | text)",
    "Parent ID",
    "Dependency IDs (comma separated)",
    "Implementation owners",
    "Verification owners",
    "Requirement IDs",
    "Task IDs",
    "Technical goal",
    "Execution check",
    "Disposition (active/superseded_by_user/replaced_by_user/withdrawn_by_user)",
    "Reason for change",
    "New source / user correction (immutable)",
    "Mandatory (yes/no)",
    "Position (0-based order)",
];
const TABS: [&str; 5] = ["Source", "Criteria", "Tasks", "Evidence", "History"];

#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub base: EntrySpec,
    pub revision: i64,
    pub fields: Vec<InputBuffer>,
    pub field: usize,
    pub operation_id: String,
}
impl Form {
    pub fn new(spec: EntrySpec, revision: i64) -> Self {
        let values = vec![
            spec.id.clone(),
            spec.interpretation.clone(),
            spec.qualifiers.join("\n"),
            spec.criteria
                .iter()
                .map(|c| {
                    format!(
                        "{} | {} | {} | {}",
                        c.id,
                        if c.mandatory { "required" } else { "optional" },
                        if c.integration {
                            "integration"
                        } else {
                            "check"
                        },
                        c.text
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            spec.parent.clone().unwrap_or_default(),
            spec.depends_on.join(", "),
            spec.implementation_owners.join(", "),
            spec.verification_owners.join(", "),
            spec.requirement_ids.join(", "),
            spec.task_ids.join(", "),
            spec.technical_goal.clone().unwrap_or_default(),
            spec.check.clone().unwrap_or_default(),
            serde_json::to_value(spec.disposition)
                .unwrap()
                .as_str()
                .unwrap()
                .into(),
            String::new(),
            String::new(),
            if spec.mandatory { "yes" } else { "no" }.into(),
            spec.position.to_string(),
        ];
        Self {
            base: spec,
            revision,
            fields: values
                .into_iter()
                .map(|text| InputBuffer::with_text(super::sanitize_paste(&text)))
                .collect(),
            field: 1,
            operation_id: crate::id(),
        }
    }
    pub fn spec(&self) -> crate::Result<EntrySpec> {
        let text = |i: usize| self.fields[i].text.trim().to_owned();
        let ids = |i: usize| {
            text(i)
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let mut spec = self.base.clone();
        spec.id = text(0);
        crate::require(
            self.base.source.source_id == "new" || spec.id == self.base.id,
            "existing record identity is immutable",
        )?;
        spec.interpretation = text(1);
        spec.qualifiers = text(2)
            .lines()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        spec.criteria = text(3)
            .lines()
            .filter(|s| !s.trim().is_empty())
            .map(|line| {
                let parts: Vec<_> = line.splitn(4, '|').map(str::trim).collect();
                crate::require(
                    parts.len() == 4
                        && matches!(parts[1], "required" | "optional")
                        && matches!(parts[2], "integration" | "check"),
                    "criterion format: ID | required/optional | integration/check | text",
                )?;
                Ok(Criterion {
                    id: parts[0].into(),
                    text: parts[3].into(),
                    mandatory: parts[1] == "required",
                    integration: parts[2] == "integration",
                })
            })
            .collect::<crate::Result<_>>()?;
        spec.parent = if text(4).is_empty() {
            None
        } else {
            Some(text(4))
        };
        spec.depends_on = ids(5);
        spec.implementation_owners = ids(6);
        spec.verification_owners = ids(7);
        spec.requirement_ids = ids(8);
        spec.task_ids = ids(9);
        spec.technical_goal = if text(10).is_empty() {
            None
        } else {
            Some(text(10))
        };
        spec.check = if text(11).is_empty() {
            None
        } else {
            Some(text(11))
        };
        spec.disposition = serde_json::from_value(Value::String(text(12)))?;
        crate::require(
            matches!(text(15).as_str(), "yes" | "no"),
            "mandatory must be yes or no",
        )?;
        spec.mandatory = text(15) == "yes";
        spec.position = text(16)
            .parse()
            .map_err(|_| crate::Error::Invalid("position must be a nonnegative integer".into()))?;
        Ok(spec)
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct LedgerView {
    pub visible: bool,
    pub kind: LedgerKind,
    pub page: Option<LedgerPage>,
    pub selected: usize,
    pub query: InputBuffer,
    pub searching: bool,
    pub task: Option<String>,
    pub missing_only: bool,
    pub status: InputBuffer,
    pub status_edit: bool,
    pub task_filter: InputBuffer,
    pub task_filter_edit: bool,
    pub tab: usize,
    pub scroll: usize,
    pub drafts: HashMap<(String, String), Form>,
    pub editing: Option<(String, String)>,
    pub preview: Option<Value>,
    pub proposed_change: Option<LedgerChange>,
    pub detail: Option<Value>,
    pub source: Option<String>,
    pub error: Option<String>,
    pub pending: bool,
    pub request_id: u64,
    pub area: Rect,
    pub runs: Vec<Run>,
    pub edit_scroll: usize,
    pub pending_kind: String,
    pub prepared_draft: Option<((String, String), Form)>,
    pub run_filters: HashMap<(String, String), RunFilter>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RunFilter {
    query: InputBuffer,
    status: InputBuffer,
    missing: bool,
    task: Option<String>,
    selected: usize,
    tab: usize,
    scroll: usize,
}
#[derive(Debug, Clone)]
pub enum Action {
    None,
    Fetch,
    Detail,
    Source,
    Prepare {
        change: LedgerChange,
        source: String,
    },
    Apply {
        proposal: String,
        hash: String,
        change: LedgerChange,
    },
    Create,
    Runs,
    Proposal,
}
impl LedgerView {
    pub fn new(kind: LedgerKind) -> Self {
        Self {
            visible: true,
            kind,
            page: None,
            selected: 0,
            query: InputBuffer::new(),
            searching: false,
            task: None,
            missing_only: false,
            status: InputBuffer::new(),
            status_edit: false,
            task_filter: InputBuffer::new(),
            task_filter_edit: false,
            tab: 0,
            scroll: 0,
            drafts: HashMap::new(),
            editing: None,
            preview: None,
            proposed_change: None,
            detail: None,
            source: None,
            error: None,
            pending: false,
            request_id: 0,
            area: Rect::default(),
            runs: vec![],
            edit_scroll: 0,
            pending_kind: String::new(),
            prepared_draft: None,
            run_filters: HashMap::new(),
        }
    }
    pub fn request(&self, conversation: &str) -> Request {
        let query = LedgerQuery {
            conversation: conversation.into(),
            run_id: self
                .page
                .as_ref()
                .map(|p| p.run.id.clone())
                .unwrap_or_default(),
            status: if self.status.text.trim().is_empty() {
                None
            } else {
                Some(self.status.text.trim().into())
            },
            missing_coverage: self.missing_only,
            offset: self.page.as_ref().map_or(0, |p| p.offset),
            limit: Some(100),
            task: self.task.clone(),
            ..Default::default()
        };
        if self.kind == LedgerKind::Requirement {
            Request::ListRequirements { query }
        } else {
            Request::ListPlanItems { query }
        }
    }
    pub fn filtered(&self) -> Vec<&LedgerEntry> {
        self.page
            .as_ref()
            .map(|p| {
                p.entries
                    .iter()
                    .filter(|e| {
                        self.query.text.to_lowercase().split_whitespace().all(|q| {
                            format!("{} {} {}", e.spec.id, e.spec.interpretation, e.status)
                                .to_lowercase()
                                .contains(q)
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn selected_entry(&self) -> Option<&LedgerEntry> {
        self.filtered().get(self.selected).copied()
    }
    pub fn form_mut(&mut self) -> Option<&mut Form> {
        self.editing
            .as_ref()
            .and_then(|key| self.drafts.get_mut(key))
    }
    pub fn paste(&mut self, text: &str) {
        let text = super::sanitize_paste(text);
        if let Some(form) = self.form_mut() {
            let field = form.field;
            form.fields[field].insert_str(&text);
        } else if self.searching {
            self.query.insert_str(&text.replace('\n', " "));
        } else if self.status_edit {
            self.status.insert_str(&text.replace('\n', " "));
        } else if self.task_filter_edit {
            self.task_filter.insert_str(&text.replace('\n', " "));
        }
    }
    fn edit(&mut self) {
        if let Some(e) = self.selected_entry().cloned() {
            let key = (e.run_id, e.spec.id.clone());
            let revision = self.page.as_ref().unwrap().run.spec_revision;
            self.drafts
                .entry(key.clone())
                .or_insert_with(|| Form::new(e.spec, revision));
            self.editing = Some(key);
            self.preview = None;
        }
    }
    fn preserve_filter(&mut self) {
        if let Some(page) = &self.page {
            self.run_filters.insert(
                (page.run.id.clone(), self.kind.name().into()),
                RunFilter {
                    query: self.query.clone(),
                    status: self.status.clone(),
                    missing: self.missing_only,
                    task: self.task.clone(),
                    selected: self.selected,
                    tab: self.tab,
                    scroll: self.scroll,
                },
            );
        }
    }
    fn restore_filter(&mut self) {
        let key = self
            .page
            .as_ref()
            .map(|p| (p.run.id.clone(), self.kind.name().into()));
        let state = key
            .and_then(|key| self.run_filters.get(&key).cloned())
            .unwrap_or_else(|| RunFilter {
                query: InputBuffer::new(),
                status: InputBuffer::new(),
                missing: false,
                task: None,
                selected: 0,
                tab: 0,
                scroll: 0,
            });
        self.query = state.query;
        self.status = state.status;
        self.missing_only = state.missing;
        self.task = state.task;
        self.selected = state.selected;
        self.tab = state.tab;
        self.scroll = state.scroll;
        self.detail = None;
        self.source = None;
    }
    fn invalidate_read(&mut self) {
        if !matches!(
            self.pending_kind.as_str(),
            "preview" | "applied" | "created"
        ) {
            self.request_id = self.request_id.wrapping_add(1);
            self.pending = false;
        }
    }
    pub fn key(&mut self, key: KeyEvent, conversation: &str) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.preview.is_some() {
            match key.code {
                KeyCode::Esc => self.preview = None,
                KeyCode::Enter if !self.pending => {
                    let p = self.preview.as_ref().unwrap();
                    return Action::Apply {
                        proposal: p["proposal_id"].as_str().unwrap_or_default().into(),
                        hash: p["diff_hash"].as_str().unwrap_or_default().into(),
                        change: self.proposed_change.clone().unwrap(),
                    };
                }
                KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
                KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                _ => {}
            }
            return Action::None;
        }
        if self.editing.is_some() {
            if key.code == KeyCode::Esc {
                self.editing = None;
                return Action::None;
            }
            if ctrl && key.code == KeyCode::Char('s') && !self.pending {
                if let Some((origin, snapshot)) = &self.prepared_draft
                    && self.editing.as_ref() == Some(origin)
                    && self.drafts.get(origin).is_some_and(|form| {
                        form.fields != snapshot.fields || form.revision != snapshot.revision
                    })
                    && let Some(form) = self.form_mut()
                {
                    form.operation_id = crate::id();
                }
                let Some(page) = self.page.as_ref() else {
                    return Action::None;
                };
                let form = self.drafts.get(self.editing.as_ref().unwrap()).unwrap();
                match form.spec() {
                    Ok(spec) => {
                        return Action::Prepare {
                            change: LedgerChange {
                                conversation: conversation.into(),
                                run_id: page.run.id.clone(),
                                expected_spec_revision: form.revision,
                                operation_id: form.operation_id.clone(),
                                reason: form.fields[13].text.clone(),
                                entries: vec![spec],
                            },
                            source: form.fields[14].text.clone(),
                        };
                    }
                    Err(error) => {
                        self.error = Some(error.to_string());
                        return Action::None;
                    }
                }
            }
            if ctrl && key.code == KeyCode::Char('r') {
                return Action::Fetch;
            }
            let width = self.form_area().width as usize;
            if let Some(form) = self.form_mut() {
                match key.code {
                    KeyCode::Tab => form.field = (form.field + 1) % LABELS.len(),
                    KeyCode::BackTab => form.field = (form.field + LABELS.len() - 1) % LABELS.len(),
                    KeyCode::Up => form.fields[form.field].move_up(width),
                    KeyCode::Down => form.fields[form.field].move_down(width),
                    KeyCode::Enter => form.fields[form.field].insert_char('\n'),
                    _ => form.fields[form.field].edit_key(key),
                }
            }
            return Action::None;
        }
        if self.pending
            && matches!(
                self.pending_kind.as_str(),
                "preview" | "applied" | "created"
            )
        {
            return Action::None;
        }
        if self.pending
            && self.pending_kind == "page"
            && matches!(key.code, KeyCode::Char('e' | 'n' | 'N') | KeyCode::Enter)
        {
            return Action::None;
        }
        if self.searching || self.status_edit || self.task_filter_edit {
            if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
                let status = self.status_edit || self.task_filter_edit;
                if self.task_filter_edit {
                    self.task = if self.task_filter.text.trim().is_empty() {
                        None
                    } else {
                        Some(self.task_filter.text.trim().into())
                    };
                }
                self.searching = false;
                self.status_edit = false;
                self.task_filter_edit = false;
                return if status { Action::Fetch } else { Action::None };
            }
            if self.searching {
                self.query.edit_key(key);
            } else if self.task_filter_edit {
                self.task_filter.edit_key(key);
            } else {
                self.status.edit_key(key);
            }
            self.selected = 0;
            return Action::None;
        }
        match key.code {
            KeyCode::Esc => self.visible = false,
            KeyCode::Char('/') => self.searching = true,
            KeyCode::Char('f') => self.status_edit = true,
            KeyCode::Char('t') => {
                self.task_filter = InputBuffer::with_text(self.task.clone().unwrap_or_default());
                self.task_filter_edit = true;
            }
            KeyCode::Char('m') => {
                self.missing_only = !self.missing_only;
                return Action::Fetch;
            }
            KeyCode::Tab => {
                self.invalidate_read();
                self.preserve_filter();
                self.kind = if self.kind == LedgerKind::Requirement {
                    LedgerKind::PlanItem
                } else {
                    LedgerKind::Requirement
                };
                self.restore_filter();
                if let Some(p) = &mut self.page {
                    p.offset = 0;
                }
                return Action::Fetch;
            }
            KeyCode::Down => {
                self.invalidate_read();
                self.selected = (self.selected + 1).min(self.filtered().len().saturating_sub(1));
                self.detail = None;
                self.source = None;
                self.scroll = 0;
            }
            KeyCode::Up => {
                self.invalidate_read();
                self.selected = self.selected.saturating_sub(1);
                self.detail = None;
                self.source = None;
                self.scroll = 0;
            }
            KeyCode::Right => {
                self.invalidate_read();
                self.tab = (self.tab + 1) % 5;
                self.scroll = 0;
                return Action::Detail;
            }
            KeyCode::Left => {
                self.invalidate_read();
                self.tab = (self.tab + 4) % 5;
                self.scroll = 0;
                return Action::Detail;
            }
            KeyCode::Enter => return Action::Detail,
            KeyCode::Char('s') => return Action::Source,
            KeyCode::Char('e') => self.edit(),
            KeyCode::Char('r') => return Action::Fetch,
            KeyCode::Char('g') => return Action::Runs,
            KeyCode::Char('p') => return Action::Proposal,
            KeyCode::PageDown => {
                if let Some(p) = &mut self.page
                    && p.has_more
                {
                    p.offset = p.offset.saturating_add(100);
                    self.selected = 0;
                    return Action::Fetch;
                }
            }
            KeyCode::PageUp => {
                if let Some(p) = &mut self.page {
                    p.offset = p.offset.saturating_sub(100);
                    self.selected = 0;
                    return Action::Fetch;
                }
            }
            KeyCode::Char('N') => return Action::Create,
            KeyCode::Char('n') => {
                let Some(page) = &self.page else {
                    return Action::Create;
                };
                let spec:EntrySpec=serde_json::from_value(serde_json::json!({"id":format!("{}-{}",if self.kind==LedgerKind::Requirement{"req"}else{"step"},crate::id()),"kind":self.kind,"source":{"source_id":"new","start":0,"end":1},"interpretation":"","criteria":[],"mandatory":true})).unwrap();
                let key = (page.run.id.clone(), spec.id.clone());
                let mut form = Form::new(spec, page.run.spec_revision);
                form.field = 1;
                self.drafts.insert(key.clone(), form);
                self.editing = Some(key);
            }
            _ => {}
        }
        Action::None
    }
    pub fn mouse(&mut self, mouse: MouseEvent) -> Action {
        if self.editing.is_some() && self.preview.is_none() {
            let body = self.form_area();
            let visible = body.height as usize;
            let start = self
                .form_mut()
                .map_or(0, |f| f.field.saturating_sub(visible / 2));
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                if self.area.width >= 60
                    && mouse.column < body.x
                    && mouse.row >= body.y
                    && mouse.row < body.bottom()
                {
                    if let Some(form) = self.form_mut() {
                        form.field = (start + mouse.row.saturating_sub(body.y) as usize)
                            .min(LABELS.len() - 1);
                    }
                    self.edit_scroll = 0;
                } else if mouse.row == self.area.y + 2 {
                    if let Some(form) = self.form_mut() {
                        form.field = (form.field + 1) % LABELS.len();
                    }
                    self.edit_scroll = 0;
                } else if body.contains((mouse.column, mouse.row).into()) {
                    let row = mouse.row.saturating_sub(body.y) as usize + self.edit_scroll;
                    if let Some(form) = self.form_mut() {
                        let input = &mut form.fields[form.field];
                        let lines =
                            super::editor::compute_visual_lines(&input.text, body.width as usize);
                        if let Some(line) = lines.get(row) {
                            let column = mouse.column.saturating_sub(body.x);
                            input.cursor = line
                                .chars
                                .iter()
                                .find(|(_, x, width)| column < x.saturating_add(*width))
                                .map(|(byte, _, _)| *byte)
                                .unwrap_or(line.end_byte);
                            input.preferred_col = None;
                        }
                    }
                }
            } else if matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            ) && let Some(form) = self.form_mut()
            {
                for _ in 0..3 {
                    if mouse.kind == MouseEventKind::ScrollUp {
                        form.fields[form.field].move_up(body.width as usize);
                    } else {
                        form.fields[form.field].move_down(body.width as usize);
                    }
                }
            }
            return Action::None;
        }
        if self.preview.is_some() {
            match mouse.kind {
                MouseEventKind::ScrollDown => self.scroll = self.scroll.saturating_add(3),
                MouseEventKind::ScrollUp => self.scroll = self.scroll.saturating_sub(3),
                _ => {}
            }
            return Action::None;
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            if mouse.row == self.area.y + 1 {
                if self.pending
                    && matches!(
                        self.pending_kind.as_str(),
                        "preview" | "applied" | "created"
                    )
                {
                    return Action::None;
                }
                self.invalidate_read();
                self.preserve_filter();
                self.kind = if mouse.column < self.area.width / 2 {
                    LedgerKind::Requirement
                } else {
                    LedgerKind::PlanItem
                };
                self.restore_filter();
                return Action::Fetch;
            }
            let row = mouse.row.saturating_sub(self.area.y + 3) as usize;
            if row < self.filtered().len() {
                self.invalidate_read();
                self.selected = row;
                self.detail = None;
                self.source = None;
                return Action::Detail;
            }
        }
        let code = match mouse.kind {
            MouseEventKind::ScrollDown => KeyCode::Down,
            MouseEventKind::ScrollUp => KeyCode::Up,
            _ => return Action::None,
        };
        self.key(KeyEvent::new(code, KeyModifiers::NONE), "")
    }
    pub fn reply(&mut self, kind: &str, reply: std::result::Result<Value, String>) {
        self.pending = false;
        match reply {
            Err(error) => self.error = Some(error),
            Ok(value) => {
                self.error = None;
                match kind {
                    "page" => match serde_json::from_value::<LedgerPage>(value) {
                        Ok(page) => {
                            if let Some(form) = self.form_mut() {
                                form.revision = page.run.spec_revision;
                                form.operation_id = crate::id();
                            }
                            self.page = Some(page);
                            self.selected =
                                self.selected.min(self.filtered().len().saturating_sub(1));
                        }
                        Err(error) => self.error = Some(error.to_string()),
                    },
                    "detail" => self.detail = Some(value),
                    "source" => self.source = value["text"].as_str().map(str::to_owned),
                    "preview" => {
                        self.proposed_change = serde_json::from_value(value["change"].clone()).ok();
                        self.preview = Some(value["proposal"].clone());
                        self.scroll = 0;
                    }
                    "applied" => {
                        self.preview = None;
                        if let Some((key, snapshot)) = self.prepared_draft.take() {
                            if self.drafts.get(&key) == Some(&snapshot) {
                                self.drafts.remove(&key);
                                if self.editing.as_ref() == Some(&key) {
                                    self.editing = None;
                                }
                            } else if let Some(form) = self.drafts.get_mut(&key) {
                                form.revision =
                                    value["spec_revision"].as_i64().unwrap_or(form.revision);
                                form.operation_id = crate::id();
                                if let Some(change) = &self.proposed_change
                                    && let Some(saved) = change.entries.first()
                                {
                                    form.base = saved.clone();
                                }
                            }
                        }
                        self.proposed_change = None;
                    }
                    "runs" => {
                        self.preserve_filter();
                        self.runs = serde_json::from_value(value).unwrap_or_default();
                        if !self.runs.is_empty() {
                            let current = self.page.as_ref().map(|p| p.run.id.as_str());
                            let index = self
                                .runs
                                .iter()
                                .position(|r| Some(r.id.as_str()) == current)
                                .map_or(0, |i| (i + 1) % self.runs.len());
                            if let Some(p) = &mut self.page {
                                p.run = self.runs[index].clone();
                                p.offset = 0;
                                p.entries.clear();
                            }
                            self.restore_filter();
                        }
                    }
                    "created" => {
                        self.preserve_filter();
                        self.page = serde_json::from_value(value).ok();
                        self.restore_filter();
                    }
                    _ => {}
                }
            }
        }
    }
    fn form_area(&self) -> Rect {
        let left = if self.area.width >= 60 { 26 } else { 1 };
        Rect::new(
            self.area.x.saturating_add(left),
            self.area.y.saturating_add(4),
            self.area.width.saturating_sub(left + 1).max(1),
            self.area.height.saturating_sub(7).max(1),
        )
    }
    fn draw_form(&mut self, frame: &mut Frame, area: Rect) {
        let body = self.form_area();
        let Some(form) = self.editing.as_ref().and_then(|key| self.drafts.get(key)) else {
            return;
        };
        frame.render_widget(
            Paragraph::new(format!(
                "Edit {} · revision {}",
                form.base.id, form.revision
            )),
            Rect::new(area.x, area.y, area.width, 1),
        );
        frame.render_widget(
            Paragraph::new("Tab/Shift+Tab field · Ctrl+S preview · Ctrl+R reload · Esc keep draft"),
            Rect::new(area.x, area.y + 1, area.width, 1),
        );
        frame.render_widget(
            Paragraph::new(format!("{}: {}", form.field + 1, LABELS[form.field])),
            Rect::new(area.x, area.y + 2, area.width, 1),
        );
        if area.width >= 60 {
            let start = form.field.saturating_sub(body.height as usize / 2);
            let labels = LABELS
                .iter()
                .enumerate()
                .skip(start)
                .take(body.height as usize)
                .map(|(i, label)| {
                    Line::from(format!(
                        "{} {}",
                        if i == form.field { ">" } else { " " },
                        label
                    ))
                })
                .collect::<Vec<_>>();
            frame.render_widget(
                Paragraph::new(labels),
                Rect::new(area.x, body.y, 24, body.height),
            );
        }
        let input = &form.fields[form.field];
        let (column, row) = input.visual_cursor(body.width as usize);
        if row < self.edit_scroll {
            self.edit_scroll = row;
        } else if row >= self.edit_scroll + body.height as usize {
            self.edit_scroll = row.saturating_sub(body.height as usize - 1);
        }
        let lines = super::visual_text_lines(&input.text, body.width as usize)
            .into_iter()
            .skip(self.edit_scroll)
            .take(body.height as usize)
            .map(Line::from)
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), body);
        let cursor_y = body
            .y
            .saturating_add(row.saturating_sub(self.edit_scroll).min(u16::MAX as usize) as u16);
        if cursor_y < body.bottom() {
            frame.set_cursor_position((
                body.x
                    .saturating_add(column)
                    .min(body.right().saturating_sub(1)),
                cursor_y,
            ));
        }
        let status = self.error.as_deref().unwrap_or(if self.pending {
            "Waiting for daemon; draft preserved"
        } else {
            "Original source is immutable. Add a user correction in the New source field."
        });
        frame.render_widget(
            Paragraph::new(sanitize(status)),
            Rect::new(area.x, area.bottom().saturating_sub(2), area.width, 1),
        );
        frame.render_widget(
            Paragraph::new(
                "Criteria: ID | required/optional | integration/check | text. Enter adds a line.",
            ),
            Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1),
        );
    }
    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        self.area = area;
        if area.width < 12 || area.height < 5 {
            frame.render_widget(Paragraph::new("Resize terminal"), area);
            return;
        }
        if self.editing.is_some() && self.preview.is_none() {
            self.draw_form(frame, area);
            return;
        }
        let theme = Theme::from_env();
        let mut lines = Vec::new();
        let name = if self.kind == LedgerKind::Requirement {
            "Requirements"
        } else {
            "Plan items"
        };
        lines.push(Line::from(format!(
            "Requirements  |  Plan items   [{name}]"
        )));
        if let Some(page) = &self.page {
            lines.push(Line::from(format!(
                "Run {} · revision {} · {} records · filter: {} · search: {}",
                page.run.id, page.run.spec_revision, page.total, self.status.text, self.query.text
            )));
        }
        if self.task_filter_edit {
            lines.push(Line::from(format!(
                "Task ID / UUID: {}",
                self.task_filter.text
            )));
        }
        if let Some(task) = &self.task {
            lines.push(Line::from(format!("Task filter: {task}")));
        }
        if let Some(error) = &self.error {
            lines.push(Line::from(format!(
                "Error: {} · draft preserved",
                sanitize(error)
            )));
        }
        if self.pending {
            lines.push(Line::from("Waiting for daemon…"));
        }
        if let Some(preview) = &self.preview {
            lines.push(Line::from("Review exact scope change and affected tasks. Enter applies · Esc returns to draft"));
            if let Some(change) = &self.proposed_change {
                lines.push(Line::from(format!("Reason: {}", change.reason)));
            }
            let text = serde_json::to_string_pretty(&preview["diff"]).unwrap_or_default();
            for line in text.lines() {
                for part in wrap(&sanitize(line), area.width.saturating_sub(2) as usize) {
                    lines.push(Line::from(part));
                }
            }
        } else {
            for (i, e) in self
                .filtered()
                .into_iter()
                .enumerate()
                .skip(self.selected.saturating_sub(area.height as usize / 3))
                .take(area.height as usize / 3)
            {
                let parent = e
                    .spec
                    .parent
                    .as_deref()
                    .map(|p| format!("{p} > "))
                    .unwrap_or_default();
                lines.push(Line::from(format!(
                    "{} {}{} [{}] {}",
                    if i == self.selected { ">" } else { " " },
                    parent,
                    e.spec.id,
                    e.status,
                    e.spec.interpretation
                )));
            }
            lines.push(Line::from(format!(
                "{}   ←/→ tab · Enter details · s source",
                TABS[self.tab]
            )));
            if let Some(source) = &self.source {
                for line in source.lines() {
                    for part in wrap(&sanitize(line), area.width.saturating_sub(2) as usize) {
                        lines.push(Line::from(part));
                    }
                }
            } else if let Some(e) = self.selected_entry() {
                let text = match self.tab {
                    0 => format!(
                        "{} bytes {}..{}\n{}",
                        e.spec.source.source_id, e.spec.source.start, e.spec.source.end, e.original
                    ),
                    1 => e
                        .spec
                        .criteria
                        .iter()
                        .map(|c| {
                            format!(
                                "{} [{}{}] {}",
                                c.id,
                                if c.mandatory { "required" } else { "optional" },
                                if c.integration { ", integration" } else { "" },
                                c.text
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                    2 => {
                        let linked=self.detail.as_ref().and_then(|v|v["tasks"].as_array()).map(|tasks|tasks.iter().map(|t|format!("{}: {} [{}] · plan {} · input v{} · generation {} · {}",t["task"].as_str().unwrap_or_default(),t["title"].as_str().unwrap_or_default(),t["status"].as_str().unwrap_or_default(),t["plan"].as_str().unwrap_or_default(),t["entry_version"],t["generation"],t["id"].as_str().unwrap_or_default())).collect::<Vec<_>>().join("\n")).unwrap_or_else(||"Enter to load linked tasks".into());
                        format!(
                            "Declared task IDs: {}\nImplementation: {}\nVerification: {}\n{linked}",
                            e.spec.task_ids.join(", "),
                            e.spec.implementation_owners.join(", "),
                            e.spec.verification_owners.join(", ")
                        )
                    }
                    3 => self
                        .detail
                        .as_ref()
                        .map(|v| serde_json::to_string_pretty(&v["evidence"]).unwrap_or_default())
                        .unwrap_or_else(|| "Enter to load evidence".into()),
                    _ => self
                        .detail
                        .as_ref()
                        .map(|v| serde_json::to_string_pretty(&v["history"]).unwrap_or_default())
                        .unwrap_or_else(|| "Enter to load history".into()),
                };
                for line in text.lines() {
                    for part in wrap(&sanitize(line), area.width.saturating_sub(2) as usize) {
                        lines.push(Line::from(part));
                    }
                }
            }
            lines.push(Line::from("n new · N new run · e edit · p proposal · g next run · / search · f status · t task · m gaps · Tab ledger · PgUp/PgDn page · r refresh · Esc close"));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .scroll((self.scroll.min(u16::MAX as usize) as u16, 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_set(if theme.ascii {
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
                            ratatui::symbols::border::PLAIN
                        })
                        .title(format!(" {name} ")),
                ),
            area,
        );
    }
}
