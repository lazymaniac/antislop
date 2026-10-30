use super::contracts::*;
use super::*;

mod update;

/// Layout data for the active conversation, separate from the viewport buffer.
/// Unchanged cards reuse wrapped rows; only visible rows are copied per frame.
#[derive(Default)]
pub(super) struct TranscriptLayout {
    pub(super) context: Option<(String, u16, Theme)>,
    pub(super) entries: Vec<TranscriptLayoutEntry>,
}

pub(super) struct TranscriptLayoutEntry {
    pub(super) source: TranscriptCard,
    pub(super) comment: Option<(String, Option<String>)>,
    pub found: bool,
    pub(super) rows: Vec<Row>,
    pub(super) end: usize,
}

pub struct App {
    pub(super) native_views: std::collections::HashMap<String, native_view::View>,
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
    pub updates: mpsc::Sender<Update>,
    pub focus: Focus,
    /// The selected graph node, by key, so it survives new events.
    pub selected: String,
    pub filter: Filter,
    /// The graph's top-left corner shown in its panel.
    pub offset: (u16, u16),
    pub details: Option<Details>,
    pub diff: Option<DiffView>,
    pub plan_view: Option<PlanView>,
    pub ledger_view: Option<ledger::LedgerView>,
    pub editor: Option<Editor>,
    pub picker: Option<Picker>,
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
    pub failed_comment: Option<Editor>,
    pub conversation_drafts: std::collections::HashMap<String, String>,
    pub comment_drafts: std::collections::HashMap<String, String>,
    pub comment_draft_cursors: std::collections::HashMap<String, (usize, Option<u16>)>,
    pub conversation_states: std::collections::HashMap<String, ConversationState>,
    pub graph_filter: Option<String>,
    pub graph_collapsed: std::collections::HashSet<String>,
    pub graph_grouping: graph::GroupingMode,
    pub confirmation: Option<ConfirmationCard>,
    pub details_scroll_memory: std::collections::HashMap<(String, usize), usize>,
    pub details_query_memory: std::collections::HashMap<(String, usize), String>,
    /// The last rendered graph viewport in terminal coordinates, for hit testing.
    pub graph_area: Option<Rect>,
    pub phase_action_area: Option<Rect>,
    pub phase_action_command: Option<&'static str>,
    pub chat_area: Rect,
    pub graph_viewport: Option<(u16, u16)>,
    pub trust_prompt: Option<String>,
    /// The way out for a stopped step, while the user picks one.
    pub(super) unblock: Option<unblock::Unblock>,
    /// The projects the screen is connected to; the active one's client is
    /// `client`.
    pub projects: Vec<workspace::ProjectConn>,
    pub active_project: usize,
    /// A directory the user is asked about before it is opened.
    pub project_trust: Option<std::path::PathBuf>,
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
            focus: Focus::Chat,
            selected: ORCHESTRATOR.into(),
            filter: Filter::All,
            offset: (0, 0),
            details: None,
            native_views: std::collections::HashMap::new(),
            diff: None,
            plan_view: None,
            ledger_view: None,
            editor: None,
            picker: None,
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
            failed_comment: None,
            conversation_drafts: std::collections::HashMap::new(),
            comment_drafts: std::collections::HashMap::new(),
            comment_draft_cursors: std::collections::HashMap::new(),
            conversation_states: std::collections::HashMap::new(),
            graph_filter: None,
            graph_collapsed: std::collections::HashSet::new(),
            graph_grouping: graph::GroupingMode::default(),
            confirmation: None,
            details_scroll_memory: std::collections::HashMap::new(),
            details_query_memory: std::collections::HashMap::new(),
            graph_viewport: None,
            graph_area: None,
            phase_action_area: None,
            phase_action_command: None,
            chat_area: Rect::default(),
            trust_prompt: None,
            unblock: None,
            projects: Vec::new(),
            active_project: 0,
            project_trust: None,
        }
    };
}

impl App {
    pub(super) fn bootstrap(client: Client, updates: mpsc::Sender<Update>) -> App {
        initial_app!(client, updates)
    }

    pub fn test_app() -> App {
        let (updates, _rx) = mpsc::channel();
        Self::test_app_with_client_and_updates(
            Client::at(std::path::Path::new("/dev/null")),
            updates,
        )
    }

    pub fn test_app_with_client(client: Client) -> (App, mpsc::Receiver<Update>) {
        let (updates, rx) = mpsc::channel();
        (Self::test_app_with_client_and_updates(client, updates), rx)
    }

    pub fn test_app_with_client_and_updates(client: Client, updates: mpsc::Sender<Update>) -> App {
        Self::bootstrap(client, updates)
    }
}
