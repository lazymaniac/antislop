//! Full-screen obligation browser and field editor. Drafts survive closing the
//! editor, record switches, conversation switches, and failed asynchronous saves.
use super::{InputBuffer, Theme, sanitize, view::wrap};
use crate::lean::{daemon::Request, ledger::*};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
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
const TABS: [&str; 5] = ["Overview", "Criteria", "Tasks", "Evidence", "History"];

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
    pub list_area: Rect,
    pub detail_area: Rect,
    pub list_start: usize,
    pub confirm_new_run: bool,
    pub notice: Option<String>,
    pub workflow_notice: Option<(bool, String)>,
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
            list_area: Rect::default(),
            detail_area: Rect::default(),
            list_start: 0,
            confirm_new_run: false,
            notice: None,
            workflow_notice: None,
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
        if self.confirm_new_run {
            match key.code {
                KeyCode::Esc => self.confirm_new_run = false,
                KeyCode::Enter if !self.pending => {
                    self.confirm_new_run = false;
                    return Action::Create;
                }
                _ => {}
            }
            return Action::None;
        }
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
                self.source = None;
                self.scroll = 0;
                return Action::Detail;
            }
            KeyCode::Left => {
                self.invalidate_read();
                self.tab = (self.tab + 4) % 5;
                self.source = None;
                self.scroll = 0;
                return Action::Detail;
            }
            KeyCode::Enter => return Action::Detail,
            KeyCode::Char('s') => return Action::Source,
            KeyCode::Char('e') => self.edit(),
            KeyCode::Char('r') => return Action::Fetch,
            KeyCode::Char('g') => return Action::Runs,
            KeyCode::Char('p' | 'P') => return Action::Proposal,
            KeyCode::Char('j') => self.scroll = self.scroll.saturating_add(1),
            KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
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
            KeyCode::Char('N') => self.confirm_new_run = true,
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
                let column = mouse.column.saturating_sub(self.area.x);
                self.kind = if column < 16 {
                    LedgerKind::Requirement
                } else if column < 30 {
                    LedgerKind::PlanItem
                } else {
                    return Action::None;
                };
                self.restore_filter();
                return Action::Fetch;
            }
            if mouse.row == self.area.bottom().saturating_sub(2)
                && mouse.column.saturating_sub(self.area.x) < 26
            {
                return Action::Proposal;
            }
            if self.detail_area.width > 0
                && mouse.row == self.detail_area.y.saturating_sub(1)
                && mouse.column >= self.detail_area.x
                && mouse.column < self.detail_area.right()
            {
                let column = mouse.column.saturating_sub(self.detail_area.x) as usize;
                let mut end = 0;
                for (index, label) in TABS.iter().enumerate() {
                    end += unicode_width::UnicodeWidthStr::width(*label) + 2;
                    if column < end {
                        self.invalidate_read();
                        self.tab = index;
                        self.scroll = 0;
                        self.source = None;
                        return Action::Detail;
                    }
                }
                return Action::None;
            }
            if self.list_area.contains((mouse.column, mouse.row).into()) {
                let row = self.list_start + mouse.row.saturating_sub(self.list_area.y) as usize / 3;
                if row < self.filtered().len() {
                    self.invalidate_read();
                    self.selected = row;
                    self.scroll = 0;
                    self.detail = None;
                    self.source = None;
                    return Action::Detail;
                }
            }
        }
        if self.detail_area.contains((mouse.column, mouse.row).into()) {
            match mouse.kind {
                MouseEventKind::ScrollDown => self.scroll = self.scroll.saturating_add(3),
                MouseEventKind::ScrollUp => self.scroll = self.scroll.saturating_sub(3),
                _ => {}
            }
            return Action::None;
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
                        self.scroll = 0;
                        self.detail = None;
                        self.source = None;
                        self.notice = Some("Changes saved. Approved analyst proposals resume verification and planning automatically.".into());
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
                        if let Ok(run) = serde_json::from_value::<Run>(value) {
                            self.page = Some(LedgerPage {
                                run,
                                entries: vec![],
                                total: 0,
                                offset: 0,
                                has_more: false,
                            });
                            self.notice = Some("Draft set created. Current planning keeps its existing requirements.".into());
                        }
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
    fn detail_lines(&self, theme: &Theme, width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let Some(entry) = self.selected_entry() else {
            return vec![Line::from(
                "Select a record to inspect its requirements and coverage.",
            )];
        };
        let mut section = |title: &str, text: &str| {
            lines.push(Line::from(Span::styled(title.to_owned(), theme.title())));
            for line in text.lines() {
                lines.extend(wrap(&sanitize(line), width).into_iter().map(Line::from));
            }
            lines.push(Line::from(""));
        };
        if let Some(source) = &self.source {
            section("Original source", source);
            return lines;
        }
        match self.tab {
            0 => {
                section("Intent", &entry.spec.interpretation);
                section(
                    "Constraints & qualifiers",
                    &if entry.spec.qualifiers.is_empty() {
                        "No additional qualifiers.".into()
                    } else {
                        entry
                            .spec
                            .qualifiers
                            .iter()
                            .map(|q| format!("- {q}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    },
                );
                if !entry.spec.requirement_ids.is_empty() {
                    section(
                        "Linked requirements",
                        &entry.spec.requirement_ids.join(", "),
                    );
                }
                if !entry.spec.depends_on.is_empty() {
                    section("Dependencies", &entry.spec.depends_on.join(", "));
                }
                if let Some(goal) = &entry.spec.technical_goal {
                    section("Technical goal", goal);
                }
                if let Some(check) = &entry.spec.check {
                    section("Execution check", check);
                }
                section("Original wording", &entry.original);
                section(
                    "Source reference",
                    &format!(
                        "{} · bytes {}..{} · entry v{}",
                        entry.spec.source.source_id,
                        entry.spec.source.start,
                        entry.spec.source.end,
                        entry.version
                    ),
                );
            }
            1 => {
                if entry.spec.criteria.is_empty() {
                    section("Acceptance criteria", "No criteria have been recorded yet.");
                }
                for criterion in &entry.spec.criteria {
                    section(
                        &format!(
                            "{} · {}{}",
                            criterion.id,
                            if criterion.mandatory {
                                "Required"
                            } else {
                                "Optional"
                            },
                            if criterion.integration {
                                " · Integration"
                            } else {
                                ""
                            }
                        ),
                        &criterion.text,
                    );
                }
            }
            2 => {
                section("Assigned task IDs", &entry.spec.task_ids.join(", "));
                section(
                    "Implementation owners",
                    &entry.spec.implementation_owners.join(", "),
                );
                section(
                    "Verification owners",
                    &entry.spec.verification_owners.join(", "),
                );
                if let Some(tasks) = self.detail.as_ref().and_then(|v| v["tasks"].as_array()) {
                    if tasks.is_empty() {
                        section(
                            "Linked tasks",
                            "No task mapping yet. Tasks are connected when a verified plan is prepared.",
                        );
                    }
                    for task in tasks {
                        section(
                            &format!(
                                "{} · {}",
                                task["title"].as_str().unwrap_or("Task"),
                                task["status"].as_str().unwrap_or("pending")
                            ),
                            &format!(
                                "{} · input v{} · generation {}",
                                task["id"].as_str().unwrap_or(""),
                                task["entry_version"],
                                task["generation"]
                            ),
                        );
                    }
                } else {
                    section("Linked tasks", "Press Enter to load task coverage.");
                }
            }
            3 => {
                if let Some(evidence) = self.detail.as_ref().and_then(|v| v["evidence"].as_array())
                {
                    if evidence.is_empty() {
                        section(
                            "Verification evidence",
                            "No independent evidence yet. Approval of requirements does not verify implementation.",
                        );
                    }
                    for item in evidence {
                        section(
                            if item["approved"].as_bool() == Some(true) {
                                "Verified evidence"
                            } else {
                                "Evidence needs review"
                            },
                            &format!(
                                "{}\n{}\n{}",
                                item["proposal"]["criterion_id"].as_str().unwrap_or(""),
                                item["proposal"]["result"].as_str().unwrap_or(""),
                                item["assessment"].as_str().unwrap_or("")
                            ),
                        );
                    }
                } else {
                    section(
                        "Verification evidence",
                        "Press Enter to load independent checks.",
                    );
                }
            }
            _ => {
                if let Some(history) = self.detail.as_ref().and_then(|v| v["history"].as_array()) {
                    for version in history {
                        section(
                            &format!(
                                "Version {} · revision {}",
                                version["version"], version["spec_revision"]
                            ),
                            version["spec"]["interpretation"].as_str().unwrap_or(""),
                        );
                    }
                    if history.is_empty() {
                        section("History", "No earlier versions.");
                    }
                } else {
                    section("History", "Press Enter to load immutable versions.");
                }
            }
        }
        lines
    }

    fn preview_lines(&self, theme: &Theme, width: usize) -> Vec<Line<'static>> {
        let mut lines = vec![
            Line::from(Span::styled("Review requirements changes", theme.title())),
            Line::from(""),
        ];
        let Some(change) = &self.proposed_change else {
            return lines;
        };
        for part in wrap(&format!("Reason: {}", sanitize(&change.reason)), width) {
            lines.push(Line::from(part));
        }
        lines.push(Line::from(""));
        let before = self
            .preview
            .as_ref()
            .and_then(|p| p["diff"]["before"].as_array());
        for entry in &change.entries {
            lines.push(Line::from(Span::styled(sanitize(&entry.id), theme.title())));
            let old = before
                .and_then(|entries| {
                    entries
                        .iter()
                        .find(|e| e["spec"]["id"].as_str() == Some(&entry.id))
                })
                .map(|e| &e["spec"]);
            let value = serde_json::to_value(entry).unwrap_or_default();
            if let Some(fields) = value.as_object() {
                for (field, value) in fields {
                    if field == "id" || old.and_then(|o| o.get(field)) == Some(value) {
                        continue;
                    }
                    lines.push(Line::from(Span::styled(
                        field.replace('_', " "),
                        theme.accent(),
                    )));
                    if let Some(previous) = old.and_then(|o| o.get(field)) {
                        for part in wrap(&format!("Before: {}", readable(previous)), width) {
                            lines.push(Line::from(Span::styled(part, theme.dim())));
                        }
                    }
                    for part in wrap(&format!("After: {}", readable(value)), width) {
                        lines.push(Line::from(part));
                    }
                    lines.push(Line::from(""));
                }
            }
        }
        let tasks = self
            .preview
            .as_ref()
            .map(|p| readable(&p["diff"]["affected_tasks"]))
            .unwrap_or_default();
        lines.push(Line::from(Span::styled("Affected tasks", theme.title())));
        lines.extend(wrap(&tasks, width).into_iter().map(Line::from));
        lines
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
        if self.confirm_new_run {
            let text = "Create a new requirements draft?\n\nThis creates a separate empty set. It does not replace the requirements used by current planning.\n\nEnter creates the draft. Esc returns to your existing requirements.";
            let lines = wrap(text, area.width.saturating_sub(4) as usize)
                .into_iter()
                .map(Line::from)
                .collect::<Vec<_>>();
            frame.render_widget(
                Paragraph::new(lines)
                    .style(theme.text())
                    .block(panel(&theme, " New draft set ")),
                area,
            );
            return;
        }
        frame.render_widget(Block::default().style(theme.panel()), area);
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
        let selected = |label: &str, active: bool| {
            Span::styled(
                format!(" {label} "),
                if active {
                    theme.selected()
                } else {
                    theme.dim()
                },
            )
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                selected("Requirements", self.kind == LedgerKind::Requirement),
                Span::raw("  "),
                selected("Plan items", self.kind == LedgerKind::PlanItem),
                Span::styled("   Tab switches view", theme.dim()),
            ])),
            rows[1],
        );
        let heading = self
            .page
            .as_ref()
            .map(|p| {
                format!(
                    " {}   /   revision {}",
                    sanitize(&p.run.goal),
                    p.run.spec_revision
                )
            })
            .unwrap_or_else(|| " Requirements workspace".into());
        frame.render_widget(Paragraph::new(heading).style(theme.title()), rows[0]);
        let filter = if self.searching {
            format!("Search: {}", self.query.text)
        } else if self.status_edit {
            format!("Status: {}", self.status.text)
        } else if self.task_filter_edit {
            format!("Task: {}", self.task_filter.text)
        } else {
            format!(
                " {} records  |  {}  |  {}",
                self.page.as_ref().map_or(0, |p| p.total),
                if self.query.text.is_empty() {
                    "/ Search".into()
                } else {
                    format!("Search: {}", self.query.text)
                },
                if self.missing_only {
                    "Missing coverage"
                } else {
                    "All coverage"
                }
            )
        };
        frame.render_widget(
            Paragraph::new(sanitize(&filter)).style(theme.dim()),
            rows[2],
        );
        if let Some((failed, message)) = &self.workflow_notice {
            frame.render_widget(
                Paragraph::new(sanitize(message)).style(if *failed {
                    theme.error()
                } else {
                    theme.accent()
                }),
                Rect::new(
                    rows[2].x,
                    rows[2].y.saturating_add(1),
                    rows[2].width,
                    rows[2].height.saturating_sub(1),
                ),
            );
        }
        let body = rows[3];
        self.list_area = Rect::default();
        self.detail_area = Rect::default();
        if self.preview.is_some() {
            let lines = self.preview_lines(&theme, body.width.saturating_sub(4) as usize);
            self.scroll = self.scroll.min(
                lines
                    .len()
                    .saturating_sub(body.height.saturating_sub(2) as usize),
            );
            frame.render_widget(
                Paragraph::new(lines)
                    .scroll((self.scroll.min(u16::MAX as usize) as u16, 0))
                    .block(panel(&theme, " Exact changes ")),
                body,
            );
            frame.render_widget(
                Paragraph::new(
                    " Enter  Approve changes & continue planning    Esc  Return without applying",
                )
                .style(theme.selected()),
                rows[4],
            );
            let status = self.error.as_deref().unwrap_or(if self.pending {
                "Applying changes... Your review is preserved."
            } else {
                "Requirements approval does not start implementation."
            });
            frame.render_widget(
                Paragraph::new(sanitize(status)).style(if self.error.is_some() {
                    theme.error()
                } else {
                    theme.dim()
                }),
                rows[5],
            );
            return;
        }
        let wide = body.width >= 96;
        let panes = Layout::default()
            .direction(if wide {
                Direction::Horizontal
            } else {
                Direction::Vertical
            })
            .constraints(if wide {
                vec![Constraint::Percentage(38), Constraint::Percentage(62)]
            } else {
                vec![
                    Constraint::Length((body.height / 3).max(3).min(body.height)),
                    Constraint::Min(0),
                ]
            })
            .split(body);
        let list_block = panel(&theme, " Records ");
        self.list_area = list_block.inner(panes[0]);
        frame.render_widget(list_block, panes[0]);
        let visible = (self.list_area.height as usize / 3).max(1);
        self.list_start = self.selected.saturating_sub(visible - 1);
        let entries = self.filtered();
        let mut records = Vec::new();
        for (index, entry) in entries
            .iter()
            .enumerate()
            .skip(self.list_start)
            .take(visible)
        {
            let style = if index == self.selected {
                theme.selected()
            } else {
                theme.text()
            };
            records.push(Line::from(Span::styled(
                theme.fit(
                    &format!(
                        " {} {}",
                        if index == self.selected {
                            theme.glyphs().pointer
                        } else {
                            " "
                        },
                        entry.spec.interpretation
                    ),
                    self.list_area.width as usize,
                ),
                style,
            )));
            records.push(Line::from(vec![
                Span::styled(
                    theme.fit(
                        &format!("   {}  ", entry.spec.id),
                        self.list_area.width as usize / 2,
                    ),
                    theme.dim(),
                ),
                Span::styled(
                    format!("{} {}", theme.status_glyph(&entry.status), entry.status),
                    theme.status(&entry.status),
                ),
            ]));
            records.push(Line::from(""));
        }
        if entries.is_empty() {
            let message = if self.pending {
                "Loading records..."
            } else if !self.query.text.is_empty() {
                "No matches. Change the search or filters."
            } else if self.kind == LedgerKind::PlanItem {
                "Plan items appear when a plan is mapped to verified requirements."
            } else {
                "No requirements yet. Continue the interview or add a requirement with n."
            };
            records.extend(
                wrap(message, self.list_area.width as usize)
                    .into_iter()
                    .map(Line::from),
            );
        }
        frame.render_widget(Paragraph::new(records), self.list_area);
        let detail_block = panel(&theme, " Details ");
        let detail = detail_block.inner(panes[1]);
        frame.render_widget(detail_block, panes[1]);
        let detail_rows =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(detail);
        let tabs = TABS
            .iter()
            .enumerate()
            .map(|(i, label)| selected(label, self.tab == i))
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(Line::from(tabs)), detail_rows[0]);
        self.detail_area = detail_rows[1];
        let lines = self.detail_lines(&theme, self.detail_area.width as usize);
        self.scroll = self
            .scroll
            .min(lines.len().saturating_sub(self.detail_area.height as usize));
        frame.render_widget(
            Paragraph::new(lines).scroll((self.scroll.min(u16::MAX as usize) as u16, 0)),
            self.detail_area,
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" p  Review pending changes ", theme.accent()),
                Span::styled(
                    " Up/Down Select  |  Left/Right Details  |  e Edit  |  Esc Close",
                    theme.dim(),
                ),
            ])),
            rows[4],
        );
        let status=self.error.as_ref().map(|e|format!("Error: {}. Draft preserved.",sanitize(e))).unwrap_or_else(||if self.pending {"Loading... Your drafts are preserved.".into()} else {"n Add  |  N New draft set  |  g Browse sets  |  / Search  |  f Status  |  t Task  |  m Gaps  |  j/k Scroll  |  r Refresh".into()});
        let status = if self.error.is_none() && !self.pending {
            self.notice.clone().unwrap_or(status)
        } else {
            status
        };
        frame.render_widget(
            Paragraph::new(status).style(if self.error.is_some() {
                theme.error()
            } else {
                theme.dim()
            }),
            rows[5],
        );
    }
}

fn panel<'a>(theme: &Theme, title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(theme.separator(false))
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
            ratatui::symbols::border::ROUNDED
        })
        .title(Span::styled(title, theme.title()))
}

fn readable(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::String(text) => sanitize(text),
        Value::Bool(value) => if *value { "Yes" } else { "No" }.into(),
        Value::Array(values) => {
            if values.is_empty() {
                "None".into()
            } else {
                values.iter().map(readable).collect::<Vec<_>>().join("; ")
            }
        }
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| format!("{}: {}", key.replace('_', " "), readable(value)))
            .collect::<Vec<_>>()
            .join("; "),
        Value::Number(value) => value.to_string(),
    }
}
