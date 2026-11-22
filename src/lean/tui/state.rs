use super::contracts::*;
use super::render::transcript_cache::TranscriptLayout;
use super::*;

mod update;

pub struct App {
    pub(super) native_views: std::collections::HashMap<String, native_view::View>,
    pub(super) workbench_sessions: Vec<crate::lean::store::SessionInfo>,
    /// Most recently issued CLI owner fetch; older concurrent replies are stale.
    pub(super) workbench_sessions_request_id: u64,
    pub(super) workbench_sessions_inflight: Option<(String, u64, u64)>,
    pub(super) workbench_sessions_pending: bool,
    pub(super) workbench_sessions_after: u64,
    pub(super) workbench_sessions_fetch_after: u64,
    pub(super) conversations_request_id: u64,
    pub(super) conversation_deletions: std::collections::HashSet<(String, String)>,
    pub(super) plan_history: Option<planhistory::Browser>,
    pub(super) graph_layout_cache: graph::LayoutCache,
    pub(super) transcript_layout: TranscriptLayout,
    pub text_selection: selection::TextSelection,
    pub(super) text_area: Option<Rect>,
    pub(super) text_origin: usize,
    pub(super) pending_clipboard: Option<String>,
    pub config_editor: Option<config::ConfigEditor>,
    pub(super) config_drafts: std::collections::HashMap<String, config::ConfigEditor>,
    pub(super) agent_drafts: details::Drafts,
    pub client: Client,
    pub project: String,
    pub conversation: String,
    pub generation: u64,
    pub stop_poller: Arc<AtomicBool>,
    /// Frames drawn so far: advances spinners; tests set it directly.
    pub(super) tick: u64,
    pub view: View,
    pub interview_forms: std::collections::HashMap<String, interview::Form>,
    pub input: InputBuffer,
    pub input_shown: usize,
    pub input_width: usize,
    pub editor_width: usize,
    /// Lines scrolled up from the bottom of the transcript.
    pub scroll: usize,
    /// The transcript entry a search led to: shown at the top, highlighted
    /// until the next key.
    pub found: Option<usize>,
    pub notice: Option<String>,
    pub updates: UpdateSender,
    pub(super) result_fetch: app::requests::ResultFetchCoordinator,
    /// Frontend screen, workbench target, focus and per-target presentation.
    pub navigation: NavigationState,
    pub navigation_revision: u64,
    pub send_navigation: std::collections::HashMap<(String, u64), (u64, u64)>,
    /// The selected graph node, by key, so it survives new events.
    pub selected: String,
    /// The relations the graph draws.
    pub relations: graph::Relations,
    /// The graph's top-left corner shown in its panel.
    pub offset: (u16, u16),
    pub details: Option<Details>,
    pub diff: Option<DiffView>,
    pub plan_view: Option<PlanView>,
    pub editor: Option<Editor>,
    pub picker: Option<Picker>,
    pub(super) rigor_recommendations_shown: std::collections::HashSet<String>,
    pub help: Option<Help>,
    pub help_scroll: usize,
    pub terminal_area: Rect,
    pub history: Vec<String>,
    pub(super) history_path: Option<std::path::PathBuf>,
    pub history_index: Option<usize>,
    pub history_draft: String,
    pub history_draft_cursor: usize,
    pub history_draft_preferred_col: Option<u16>,
    pub history_item_cursors: std::collections::HashMap<usize, usize>,
    pub sending: bool,
    pub sending_comment: bool,
    pub pending_send: Option<String>,
    pub pending_comment: Option<Editor>,
    pub send_request_id: u64,
    pub comment_request_id: u64,
    pub diff_request_id: u64,
    pub task_request_id: u64,
    pub failed_send: Option<String>,
    pub(super) image_drafts: std::collections::HashMap<String, Vec<std::path::PathBuf>>,
    pub(super) send_images: std::collections::HashMap<(String, u64), Vec<std::path::PathBuf>>,
    pub(super) send_operation_keys: std::collections::HashMap<(String, u64), String>,
    pub failed_comment: Option<Editor>,
    pub conversation_drafts: std::collections::HashMap<String, String>,
    pub comment_drafts: std::collections::HashMap<String, String>,
    /// Stable idempotency ids for comment submissions, keyed by conversation,
    /// target and text so a failed retry keeps the same durable identity.
    pub comment_operations: std::collections::HashMap<(String, String, String), String>,
    /// Stable idempotency ids for chat messages, keyed by conversation and
    /// text, so resending a failed message cannot post it twice.
    pub send_operations: std::collections::HashMap<(String, String), String>,
    /// Another frontend steers this conversation; this one watches until
    /// the user takes control.
    pub observer: bool,
    pub comment_draft_cursors: std::collections::HashMap<String, (usize, Option<u16>)>,
    pub conversation_states: std::collections::HashMap<String, ConversationState>,
    /// Exact owning project for frontend-preserved conversation state.
    pub conversation_projects: std::collections::HashMap<String, std::path::PathBuf>,
    /// Which cards the graph shows.
    pub graph_scope: graph::Scope,
    /// One card exposed by clicking its active row in the Now panel.
    pub graph_visibility_exception: Option<String>,
    /// The card last scrolled into view; the graph follows the selection
    /// only when it changes, so the user can scroll away from it.
    pub graph_revealed: Option<String>,
    pub graph_collapsed: std::collections::HashSet<String>,
    pub graph_grouping: graph::GroupingMode,
    /// The only card expanded in the graph, if any.
    pub graph_expanded: Option<String>,
    /// A card growing to (opening) or shrinking from (closing) full size.
    pub animate_graph: bool,
    pub graph_anim: Option<(String, bool, std::time::Instant)>,
    /// Where the animated card's centre was on screen when it began to grow,
    /// so it grows out from there rather than from its top-left corner.
    pub graph_anim_anchor: Option<(i32, i32)>,
    pub graph_tab: usize,
    /// Whether the graph inspector is shown when the terminal meets its minimum size.
    pub graph_inspector_visible: bool,
    pub chrome_keys: Vec<(Rect, KeyCode, KeyModifiers)>,
    pub(super) workbench_tabs: Vec<(Rect, WorkbenchTab)>,
    pub direction_actions: Option<(Rect, Rect)>,
    pub direction_area: Rect,
    pub direction_input_area: Rect,
    pub(in crate::lean::tui) direction_choices: Vec<(Rect, super::render::direction::Action)>,
    pub inspector_hits: Vec<(Rect, super::work_units::InspectorAction)>,
    /// Lines the expanded card's Results tab is scrolled down.
    pub graph_card_scroll: usize,
    /// The expanded card's content area as last drawn, for hit testing.
    pub graph_card_area: Option<Rect>,
    pub detail_states: std::collections::HashMap<(String, String), Details>,
    pub diff_states: std::collections::HashMap<(String, String), DiffView>,
    pub details_scroll_memory: std::collections::HashMap<(String, usize), usize>,
    pub workbench_memory: std::collections::HashMap<(String, WorkbenchTab), (usize, String)>,
    pub details_query_memory: std::collections::HashMap<(String, usize), String>,
    /// The last rendered graph viewport in terminal coordinates, for hit testing.
    pub graph_area: Option<Rect>,
    /// The last rendered Now panel, for row-to-agent click mapping.
    pub(super) now_area: Option<Rect>,
    /// Agent ids and visibility in the last rendered Now rows, in display order.
    pub(super) now_click_targets: Vec<Option<(String, bool)>>,
    pub phase_action_area: Option<Rect>,
    pub phase_action_command: Option<Command>,
    pub chat_area: Rect,
    pub(super) workbench_areas: super::workbench::WorkbenchAreas,
    pub(super) workbench_dragging: bool,
    pub graph_viewport: Option<(u16, u16)>,
    /// The way out for a stopped step, while the user picks one.
    pub(super) unblock: Option<unblock::Unblock>,
    pub(super) recovery_consent: Option<consent::RecoveryConsent>,
    /// The visible agent CLI approval card.
    pub(super) approval: Option<super::approval::ApprovalCard>,
    /// Approvals the user put off; they stay open and the palette lists them.
    pub(super) approval_hidden: std::collections::HashSet<String>,
    pub(super) decision_inbox: Option<super::decision_inbox::Inbox>,
    pub(super) approval_submissions:
        std::collections::HashMap<(String, String), super::decision_inbox::ApprovalStatus>,
    /// The graph filter menu, while it is open.
    pub(super) graph_filters: Option<super::graph_filters::FilterMenu>,
    /// Direct DAG flow editor for in-place task contract and dependency edits.
    pub flow_editor: Option<super::flow_editor::FlowEditor>,
    /// Source task id when interactive visual edge linking (`l`) is active on the graph.
    pub graph_link_source: Option<String>,
    /// The projects the screen is connected to; the active one's client is
    /// `client`.
    pub projects: Vec<workspace::ProjectConn>,
    pub active_project: usize,
    /// Monotonic frontend identity for requests that create a session.
    pub new_conversation_request_id: u64,
    /// Filter a successful delete until a project poll confirms it is absent.
    pub deleted_conversations: std::collections::HashSet<(std::path::PathBuf, String)>,
}

macro_rules! initial_app {
    ($client:expr, $updates:expr) => {
        App {
            plan_history: None,
            graph_layout_cache: graph::LayoutCache::default(),
            transcript_layout: TranscriptLayout::default(),
            text_selection: Default::default(),
            text_area: None,
            text_origin: 0,
            pending_clipboard: None,
            config_editor: None,
            config_drafts: Default::default(),
            agent_drafts: Default::default(),
            client: $client,
            project: "test".into(),
            conversation: "test-conv".into(),
            generation: 1,
            stop_poller: Arc::new(AtomicBool::new(false)),
            tick: 0,
            view: View::default(),
            interview_forms: Default::default(),
            input: InputBuffer::new(),
            input_shown: 0,
            input_width: 80,
            editor_width: 80,
            scroll: 0,
            found: None,
            notice: None,
            updates: $updates,
            result_fetch: app::requests::ResultFetchCoordinator::new(),
            navigation: NavigationState {
                focus: NavigationFocus::Graph,
                background_focus: Focus::Graph,
                ..Default::default()
            },
            navigation_revision: 0,
            send_navigation: Default::default(),
            selected: ORCHESTRATOR.into(),
            relations: graph::Relations::default(),
            offset: (0, 0),
            details: None,
            native_views: std::collections::HashMap::new(),
            workbench_sessions: Vec::new(),
            workbench_sessions_request_id: 0,
            workbench_sessions_inflight: None,
            workbench_sessions_pending: false,
            workbench_sessions_after: 0,
            workbench_sessions_fetch_after: 0,
            conversations_request_id: 0,
            conversation_deletions: Default::default(),
            diff: None,
            plan_view: None,
            editor: None,
            picker: None,
            rigor_recommendations_shown: Default::default(),
            help: None,
            help_scroll: 0,
            terminal_area: Rect::new(0, 0, 80, 24),
            history: vec![],
            history_path: None,
            history_index: None,
            history_draft: String::new(),
            history_draft_cursor: 0,
            history_draft_preferred_col: None,
            history_item_cursors: std::collections::HashMap::new(),
            sending: false,
            sending_comment: false,
            pending_send: None,
            pending_comment: None,
            send_request_id: 0,
            comment_request_id: 0,
            diff_request_id: 0,
            task_request_id: 0,
            failed_send: None,
            image_drafts: Default::default(),
            send_images: Default::default(),
            send_operation_keys: Default::default(),
            failed_comment: None,
            conversation_drafts: std::collections::HashMap::new(),
            comment_drafts: std::collections::HashMap::new(),
            comment_operations: std::collections::HashMap::new(),
            send_operations: std::collections::HashMap::new(),
            observer: false,
            comment_draft_cursors: std::collections::HashMap::new(),
            conversation_states: std::collections::HashMap::new(),
            conversation_projects: std::collections::HashMap::new(),
            graph_scope: graph::Scope::default(),
            graph_visibility_exception: None,
            graph_revealed: None,
            graph_collapsed: std::collections::HashSet::new(),
            graph_grouping: graph::GroupingMode::default(),
            graph_expanded: None,
            animate_graph: false,
            graph_anim: None,
            graph_anim_anchor: None,
            graph_tab: 0,
            graph_inspector_visible: true,
            chrome_keys: Vec::new(),
            workbench_tabs: Vec::new(),
            direction_actions: None,
            direction_area: Rect::default(),
            direction_input_area: Rect::default(),
            direction_choices: Vec::new(),
            inspector_hits: Vec::new(),
            graph_card_scroll: 0,
            graph_card_area: None,
            detail_states: Default::default(),
            diff_states: Default::default(),
            details_scroll_memory: std::collections::HashMap::new(),
            workbench_memory: Default::default(),
            details_query_memory: std::collections::HashMap::new(),
            graph_viewport: None,
            graph_area: None,
            now_area: None,
            now_click_targets: Vec::new(),
            phase_action_area: None,
            phase_action_command: None,
            chat_area: Rect::default(),
            workbench_areas: super::workbench::WorkbenchAreas::default(),
            workbench_dragging: false,
            unblock: None,
            recovery_consent: None,
            approval: None,
            approval_hidden: std::collections::HashSet::new(),
            decision_inbox: None,
            approval_submissions: Default::default(),
            graph_filters: None,
            flow_editor: None,
            graph_link_source: None,
            projects: Vec::new(),
            active_project: 0,
            new_conversation_request_id: 0,
            deleted_conversations: Default::default(),
        }
    };
}

impl App {
    /// Select graph/chat focus through the navigation model.
    pub fn set_focus(&mut self, focus: Focus) {
        self.navigation.set_surface_focus(focus);
    }

    pub(super) fn bootstrap(client: Client, updates: UpdateSender) -> App {
        initial_app!(client, updates)
    }

    pub fn test_app() -> App {
        let (updates, _rx) = super::update_channel(256);
        Self::test_app_with_client_and_updates(
            Client::at(std::path::Path::new("/dev/null")),
            updates,
        )
    }

    pub fn test_app_with_client(client: Client) -> (App, super::UpdateReceiver) {
        let (updates, rx) = super::update_channel(256);
        (Self::test_app_with_client_and_updates(client, updates), rx)
    }

    pub fn test_app_with_client_and_updates(client: Client, updates: UpdateSender) -> App {
        Self::bootstrap(client, updates)
    }
}
