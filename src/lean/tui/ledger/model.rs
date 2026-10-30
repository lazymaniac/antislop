use crate::lean::ledger::*;
use crate::lean::tui::InputBuffer;
use ratatui::layout::Rect;
use serde_json::Value;
use std::collections::HashMap;

pub(super) const LABELS: [&str; 14] = [
    "ID",
    "Interpretation",
    "Qualifiers (one per line)",
    "Criteria (ID | required/optional | integration/check | text)",
    "Parent ID",
    "Dependency IDs (comma separated)",
    "Implementation owners",
    "Verification owners",
    "Task IDs",
    "Disposition (active/superseded_by_user)",
    "Reason for change",
    "New source / user correction (immutable)",
    "Mandatory (yes/no)",
    "Position (0-based order)",
];
pub(super) const TABS: [&str; 5] = ["Overview", "Criteria", "Tasks", "Evidence", "History"];

use super::form::Form;

#[derive(Debug, Clone, PartialEq)]
pub struct LedgerView {
    pub visible: bool,
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
    pub review_run: Option<String>,
    pub approval_event: u64,
    pub stale: bool,
    pub scope_epoch: u64,
    pub pending_scope_epoch: u64,
    pub pending_run: Option<String>,
    pub pending_read: bool,
    pub rejecting: bool,
    pub rejection: InputBuffer,
    pub rejection_drafts: HashMap<String, InputBuffer>,
    pub prepared_draft: Option<((String, String), Form)>,
    pub run_filters: HashMap<String, RunFilter>,
}

macro_rules! ledger_view_default {
    () => {
        LedgerView {
            visible: true,
            page: None,
            selected: 0,
            query: InputBuffer::default(),
            searching: false,
            task: None,
            missing_only: false,
            status: InputBuffer::default(),
            status_edit: false,
            task_filter: InputBuffer::default(),
            task_filter_edit: false,
            tab: 0,
            scroll: 0,
            drafts: HashMap::default(),
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
            runs: Vec::default(),
            edit_scroll: 0,
            pending_kind: String::default(),
            review_run: None,
            approval_event: 0,
            stale: false,
            scope_epoch: 0,
            pending_scope_epoch: 0,
            pending_run: None,
            pending_read: false,
            rejecting: false,
            rejection: InputBuffer::default(),
            rejection_drafts: HashMap::default(),
            prepared_draft: None,
            run_filters: HashMap::default(),
        }
    };
}

impl Default for LedgerView {
    fn default() -> Self {
        ledger_view_default!()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunFilter {
    pub(super) query: InputBuffer,
    pub(super) status: InputBuffer,
    pub(super) missing: bool,
    pub(super) task: Option<String>,
    pub(super) selected: usize,
    pub(super) tab: usize,
    pub(super) scroll: usize,
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
    Reject {
        proposal: String,
        hash: String,
        run: String,
        comment: String,
    },
    Create,
    Runs,
    Proposal,
    Review(String),
}
