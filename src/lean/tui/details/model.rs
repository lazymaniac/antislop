use super::super::{dashboard, document, native_view};
use super::*;
use crate::lean::store::SessionInfo;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq)]
pub struct DetailRecord {
    pub lines: Vec<(Style, String)>,
    pub search_text: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(in crate::lean::tui) struct LayoutCache {
    records: Vec<Rc<DetailRecord>>,
    query: String,
    width: usize,
    lines: Vec<Line<'static>>,
}

impl LayoutCache {
    pub(in crate::lean::tui) fn update(
        &mut self,
        records: Vec<Rc<DetailRecord>>,
        query: &str,
        width: usize,
    ) -> &[Line<'static>] {
        let unchanged = self.width == width
            && self.query == query
            && self.records.len() == records.len()
            && self
                .records
                .iter()
                .zip(&records)
                .all(|(a, b)| Rc::ptr_eq(a, b) || a == b);
        if !unchanged {
            self.lines = Details::render_records(&records, query)
                .into_iter()
                .flat_map(|(style, text)| {
                    wrap(&text, width)
                        .into_iter()
                        .map(move |line| Line::styled(line, style))
                })
                .collect();
            self.records = records;
            self.query = query.to_owned();
            self.width = width;
        }
        &self.lines
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Composer {
    pub input: super::super::InputBuffer,
    pub sending: bool,
    pub operation: Option<String>,
    pub error: Option<String>,
    pub sent_text: Option<String>,
    pub mail: Option<u64>,
    pub delivery: Option<String>,
    pub delivery_receipts: Vec<u64>,
    pub delivery_failure: Option<(Vec<u64>, String)>,
    pub address: Option<String>,
}

pub type Drafts = std::rc::Rc<std::cell::RefCell<HashMap<(String, String), Composer>>>;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Details {
    pub native: HashMap<String, native_view::Snapshot>,
    pub native_focus: bool,
    /// An earlier round was chosen and its session is being resumed to look at.
    pub resume_round: bool,
    pub native_areas: [Rect; 2],
    /// Last rendered document viewport, excluding the scrollbar.
    pub text_area: Rect,
    /// None follows the current round; an explicit older round freezes selection.
    pub selected_round: Option<u64>,
    pub available_rounds: Vec<u64>,
    pub latest_observed: u64,
    pub active_panel: usize,
    /// The last draw put the task info beside (or above) the terminal.
    pub split: bool,
    /// Lines the dashboard is scrolled down from its top.
    pub dash_scroll: usize,
    pub panel_scroll: [usize; 2],
    pub panel_expanded: [bool; 2],
    /// Disk-backed history documents of the fetched task: result, messages, review.
    pub documents: HashMap<String, document::Document>,
    pub(super) panel_layout: [LayoutCache; 2],
    pub fetched_events: u64,
    pub fetch_in_flight: bool,
    pub task_uuid: Option<String>,
    pub conversation: String,
    pub drafts: Drafts,
    pub editing: bool,
    pub input_width: [usize; 2],
    pub key: String,
    pub tab: usize,
    /// Lines scrolled down from the top.
    pub scroll: usize,
    /// The task as the daemon stores it, once fetched.
    pub task: Option<Value>,
    /// The task round the last task fetch was issued for, so a reply for a
    /// round that already moved on can be dropped and refetched, not shown.
    pub fetched_round: Option<u64>,
    /// Request identity belongs to the saved details, including across A→B→A.
    pub fetched_generation: Option<u64>,
    pub fetched_plan: Option<String>,
    /// The selected comment on the Comments tab.
    pub comment: usize,
    /// The selected check on the Checks tab.
    pub check: usize,
    /// The selection moved: the next draw scrolls it into view.
    pub reveal_check: bool,
    /// How many check events the last task fetch had seen.
    pub fetched_checks: u64,
    pub fetch_failures: u32,
    pub fetch_retry_after: Option<std::time::Instant>,
    pub fetch_error: Option<String>,
    /// Scroll memory per (agent_key, tab_index)
    pub scroll_memory: HashMap<(String, usize), usize>,
    /// Query memory per (agent_key, tab_index)
    pub query_memory: HashMap<(String, usize), String>,
    /// Active search/filter query on current tab
    pub query: String,
    /// Whether user is typing a query in search mode
    pub searching: bool,
    /// Retained turn metadata returned by the daemon
    pub sessions: Vec<SessionInfo>,
    /// Explicit flag indicating missing/pruned historical data
    pub history_unavailable: bool,
    /// Follow new entries by default; scrolling up pauses, G/End resumes.
    pub follow: bool,
    /// Full task metadata, explicitly opened instead of the live transcript.
    pub show_info: bool,
    /// Dashboard cards: how much of each is shown, which one has focus, and
    /// the ids of the last draw (the focus walks them in screen order).
    pub dash_fold: HashMap<&'static str, dashboard::Fold>,
    pub dash_ids: Vec<&'static str>,
    pub dash_focus: usize,
    pub dash_reveal: bool,
    pub(in crate::lean::tui) pending_g: bool,
    pub(in crate::lean::tui) layout_cache: LayoutCache,
}
