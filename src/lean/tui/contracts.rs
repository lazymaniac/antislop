use super::*;

pub enum Update {
    Conversations {
        state_dir: String,
        request_id: u64,
        result: std::result::Result<Value, String>,
    },
    ConversationDeleted {
        project: String,
        id: String,
        result: std::result::Result<Value, String>,
    },
    SessionAction {
        conversation: String,
        action: crate::lean::daemon::SessionActionKind,
        result: crate::lean::daemon::SessionActionResult,
    },
    ResultDocument {
        conversation: String,
        generation: u64,
        key: String,
        selection: u64,
        id: String,
        result: std::result::Result<document::Document, String>,
    },
    NativeInputRejected {
        conversation: String,
        role: String,
        input: crate::lean::terminal::Input,
        error: String,
    },
    WorkbenchSessions {
        conversation: String,
        generation: u64,
        request_id: u64,
        result: std::result::Result<Vec<crate::lean::store::SessionInfo>, String>,
    },
    PlanHistory {
        conversation: String,
        generation: u64,
        offset: usize,
        reply: std::result::Result<crate::lean::store::PlansPage, String>,
    },
    InterviewResult {
        conversation: String,
        question: u64,
        result: std::result::Result<(), String>,
    },
    ApprovalResult {
        conversation: String,
        request: String,
        operation_id: String,
        result: std::result::Result<(), String>,
    },
    AgentMessage {
        conversation: String,
        recipient: String,
        operation: String,
        text: String,
        result: std::result::Result<Value, String>,
    },
    Config {
        project: String,
        save: bool,
        reply: std::result::Result<Value, String>,
    },
    Events {
        conversation: String,
        generation: u64,
        events: Vec<Value>,
        idle: bool,
        observer: bool,
    },
    Conversation(String),
    ConversationCreated {
        project: std::path::PathBuf,
        state_dir: std::path::PathBuf,
        request_id: u64,
        generation: u64,
        result: std::result::Result<String, String>,
    },
    Notice(String),
    RecoveryConsent {
        conversation: String,
        generation: u64,
        plan: String,
        task: String,
        result: std::result::Result<crate::lean::tui::consent::RecoveryConsent, String>,
    },
    /// A project was opened: switch to it, to `conversation` if one was named.
    ProjectReady {
        conn: Box<workspace::ProjectConn>,
        conversation: Option<String>,
    },
    /// The directory must be trusted before it is opened.
    /// The conversations of a connected project, as it last listed them.
    ProjectStatus {
        path: std::path::PathBuf,
        conversations: Vec<Value>,
    },
    /// A task's change, for the diff view of `task`.
    Diff {
        conversation: String,
        plan: Option<String>,
        generation: u64,
        request_id: u64,
        task: String,
        reply: std::result::Result<Value, String>,
    },
    /// A task as the daemon stores it, for the details of `key`. `round`
    /// is the task's round when the fetch was issued; a reply for a round
    /// that already moved on is dropped instead of shown stale.
    Task {
        conversation: String,
        plan: Option<String>,
        generation: u64,
        request_id: u64,
        key: String,
        round: u64,
        task: Value,
        documents: std::collections::HashMap<String, document::Document>,
    },
    TaskFailed {
        conversation: String,
        generation: u64,
        plan: String,
        request_id: u64,
        key: String,
        error: String,
    },
    SendResult {
        conversation: String,
        generation: u64,
        request_id: u64,
        sent_text: String,
        result: std::result::Result<(), String>,
    },
    CommentResult {
        conversation: String,
        plan: Option<String>,
        operation_id: String,
        generation: u64,
        request_id: u64,
        editor: Editor,
        result: std::result::Result<(), String>,
    },
    PlanRevisionResult {
        conversation: String,
        generation: u64,
        revision: crate::lean::store::PlanRevision,
        request_ai_review: bool,
        task_id: String,
        task_title: String,
        reply: std::result::Result<Value, String>,
    },
}

/// A comment being written on one element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor {
    pub target: String,
    pub input: InputBuffer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Chat,
    Graph,
}

/// Frontend screen selected independently of the daemon's projected data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Graph,
    Workbench,
}

/// Stable frontend target identity. Agent keys are the existing graph keys.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum WorkbenchTarget {
    #[default]
    Session,
    Agent(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WorkbenchTab {
    #[default]
    Summary,
    Conversation,
    Results,
    Checks,
    Comments,
    Diff,
}

impl WorkbenchTab {
    /// These tabs use the Overview adapter's layout/follow rules; Conversation
    /// and Diff still have their own records and independent view memory.
    pub(in crate::lean::tui) fn uses_overview(self) -> bool {
        matches!(self, Self::Summary | Self::Conversation | Self::Diff)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavigationFocus {
    #[default]
    Chat,
    Graph,
    WorkbenchList,
    WorkbenchContent,
    WorkbenchTerminal,
    Composer,
}

/// Presentation preferences scoped to a session and stable workbench target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchPresentation {
    pub tab: WorkbenchTab,
    pub round: Option<u64>,
    /// Focus remembered for this target; distinct from the current input owner.
    pub focus: NavigationFocus,
    pub split: bool,
    /// A new target stays on its PTY until real changes or explicit navigation.
    pub terminal_only_default: bool,
    /// Preferred PTY share of the vertical content area, in percent (1..=99).
    pub terminal_percent: u8,
    pub zoomed: bool,
    pub split_before_zoom: bool,
    pub list_visible: bool,
    pub list_requested: bool,
    pub pinned: bool,
    /// Agent CLI shown in the Workbench terminal, independent of content tab.
    pub terminal_owner: Option<String>,
    /// Key events are delivered to the CLI only while this is enabled.
    pub native_input: bool,
}

impl Default for WorkbenchPresentation {
    fn default() -> Self {
        Self {
            tab: WorkbenchTab::Summary,
            round: None,
            focus: NavigationFocus::WorkbenchContent,
            split: true,
            terminal_only_default: false,
            terminal_percent: 50,
            zoomed: false,
            split_before_zoom: true,
            list_visible: true,
            list_requested: false,
            pinned: false,
            terminal_owner: None,
            native_input: false,
        }
    }
}

/// Frontend navigation model; it contains no daemon-owned or persisted data.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NavigationState {
    /// Focus of the graph/chat surface behind Workbench or composer.
    pub background_focus: Focus,
    pub screen: Screen,
    pub target: WorkbenchTarget,
    pub focus: NavigationFocus,
    pub presentations: std::collections::HashMap<WorkbenchTarget, WorkbenchPresentation>,
    pub session_delivery: crate::lean::store::Delivery,
    pub compact_secondary: bool,
    pub work_selection: Option<super::work_units::WorkSelection>,
    pub remembered_stages:
        std::collections::HashMap<super::work_units::WorkUnitId, super::work_units::StageId>,
}

impl NavigationState {
    /// Select the underlying graph/chat surface, preserving its return focus
    /// when a Workbench pane or composer subsequently takes input.
    pub fn set_surface_focus(&mut self, focus: Focus) {
        self.background_focus = focus;
        self.focus = match focus {
            Focus::Chat => NavigationFocus::Chat,
            Focus::Graph => NavigationFocus::Graph,
        };
    }

    pub fn presentation(&self) -> WorkbenchPresentation {
        self.presentations
            .get(&self.target)
            .cloned()
            .unwrap_or_default()
    }

    pub fn update_presentation(&mut self, update: impl FnOnce(&mut WorkbenchPresentation)) {
        update(self.presentations.entry(self.target.clone()).or_default());
    }

    pub fn zoom(&mut self, zoomed: bool) {
        let presentation = self.presentation();
        let current = presentation.zoomed;
        if zoomed && !current {
            self.update_presentation(|presentation| {
                presentation.split_before_zoom = presentation.split;
            });
        }
        self.update_presentation(|presentation| presentation.zoomed = zoomed);
        if !zoomed {
            self.update_presentation(|presentation| {
                presentation.split = presentation.split_before_zoom;
            });
        }
    }
}

/// Full preserved state of a conversation across switches.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConversationState {
    pub input: InputBuffer,
    pub editor: Option<Editor>,
    pub sending: bool,
    pub pending_send: Option<String>,
    pub send_request_id: u64,
    pub failed_send: Option<String>,
    pub sending_comment: bool,
    pub pending_comment: Option<Editor>,
    pub comment_request_id: u64,
    pub failed_comment: Option<Editor>,
    pub comment_drafts: std::collections::HashMap<String, String>,
    pub comment_draft_cursors: std::collections::HashMap<String, (usize, Option<u16>)>,
    pub selected: String,
    pub relations: Relations,
    pub offset: (u16, u16),
    pub details: Option<Details>,
    pub diff: Option<DiffView>,
    pub plan_view: Option<PlanView>,
    pub scroll: usize,
    pub task_request_id: u64,
    pub diff_request_id: u64,
    pub navigation: NavigationState,
    pub graph_scope: Scope,
    pub graph_visibility_exception: Option<String>,
    pub graph_collapsed: std::collections::HashSet<String>,
    pub graph_grouping: graph::GroupingMode,
    pub details_scroll_memory: std::collections::HashMap<(String, usize), usize>,
    pub workbench_memory: std::collections::HashMap<(String, WorkbenchTab), (usize, String)>,
    pub details_query_memory: std::collections::HashMap<(String, usize), String>,
    pub graph_viewport: Option<(u16, u16)>,
    pub flow_editor: Option<flow_editor::FlowEditor>,
    pub recovery_consent: Option<consent::RecoveryConsent>,
    pub graph_link_source: Option<String>,
}

impl ConversationState {
    pub(super) fn capture(app: &App) -> Self {
        let mut state = Self::default();
        state.capture_composer(app);
        state.capture_navigation(app);
        state.capture_overlays(app);
        state.capture_requests(app);
        state
    }

    fn capture_composer(&mut self, app: &App) {
        self.input = app.input.clone();
        self.editor = app.editor.clone();
        self.sending = app.sending;
        self.pending_send = app.pending_send.clone();
        self.send_request_id = app.send_request_id;
        self.failed_send = app.failed_send.clone();
        self.sending_comment = app.sending_comment;
        self.pending_comment = app.pending_comment.clone();
        self.comment_request_id = app.comment_request_id;
        self.failed_comment = app.failed_comment.clone();
        self.comment_drafts = app.comment_drafts.clone();
        self.comment_draft_cursors = app.comment_draft_cursors.clone();
    }

    fn capture_navigation(&mut self, app: &App) {
        self.selected = app.selected.clone();
        self.relations = app.relations;
        self.offset = app.offset;
        self.scroll = app.scroll;
        self.navigation = app.navigation.clone();
        self.graph_scope = app.graph_scope.clone();
        self.graph_visibility_exception = app.graph_visibility_exception.clone();
        self.graph_collapsed = app.graph_collapsed.clone();
        self.graph_grouping = app.graph_grouping;
        self.graph_viewport = app.graph_viewport;
    }

    fn capture_overlays(&mut self, app: &App) {
        self.details = app.details.clone();
        self.diff = app.diff.clone();
        self.plan_view = app.plan_view.clone();
        self.flow_editor = app.flow_editor.clone();
        self.graph_link_source = app.graph_link_source.clone();
        self.recovery_consent = app.recovery_consent.clone();
    }

    fn capture_requests(&mut self, app: &App) {
        self.task_request_id = app.task_request_id;
        self.diff_request_id = app.diff_request_id;
        self.details_scroll_memory = app.details_scroll_memory.clone();
        self.details_query_memory = app.details_query_memory.clone();
        self.workbench_memory = app.workbench_memory.clone();
    }

    pub(super) fn restore(&self, app: &mut App) {
        self.restore_composer(app);
        self.restore_navigation(app);
        self.restore_overlays(app);
        self.restore_requests(app);
    }

    fn restore_composer(&self, app: &mut App) {
        app.input = self.input.clone();
        app.editor = self.editor.clone();
        app.sending = self.sending;
        app.pending_send = self.pending_send.clone();
        app.send_request_id = self.send_request_id;
        app.failed_send = self.failed_send.clone();
        app.sending_comment = self.sending_comment;
        app.pending_comment = self.pending_comment.clone();
        app.comment_request_id = self.comment_request_id;
        app.failed_comment = self.failed_comment.clone();
        app.comment_drafts = self.comment_drafts.clone();
        app.comment_draft_cursors = self.comment_draft_cursors.clone();
    }

    fn restore_navigation(&self, app: &mut App) {
        app.selected = self.selected.clone();
        app.relations = self.relations;
        app.offset = self.offset;
        app.scroll = self.scroll;
        app.navigation = self.navigation.clone();
        app.graph_scope = self.graph_scope.clone();
        app.graph_visibility_exception = self.graph_visibility_exception.clone();
        app.graph_collapsed = self.graph_collapsed.clone();
        app.graph_grouping = self.graph_grouping;
        app.graph_viewport = self.graph_viewport;
    }

    fn restore_overlays(&self, app: &mut App) {
        app.details = self.details.clone();
        app.diff = self.diff.clone();
        app.plan_view = self.plan_view.clone();
        app.flow_editor = self.flow_editor.clone();
        app.graph_link_source = self.graph_link_source.clone();
        app.recovery_consent = self.recovery_consent.clone();
    }

    fn restore_requests(&self, app: &mut App) {
        app.task_request_id = self.task_request_id;
        app.diff_request_id = self.diff_request_id;
        app.details_scroll_memory = self.details_scroll_memory.clone();
        app.details_query_memory = self.details_query_memory.clone();
        app.workbench_memory = self.workbench_memory.clone();
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;

    #[test]
    fn conversation_snapshot_round_trips_target_presentation_and_existing_draft() {
        let mut app = App::test_app();
        app.input = InputBuffer::with_text("draft 👩‍💻 e\u{301}");
        app.navigation.screen = Screen::Workbench;
        app.navigation.target = WorkbenchTarget::Agent("worker:t7".into());
        app.navigation.focus = NavigationFocus::WorkbenchContent;
        app.navigation.update_presentation(|presentation| {
            presentation.tab = WorkbenchTab::Diff;
            presentation.round = Some(3);
            presentation.split = true;
            presentation.pinned = true;
        });
        app.navigation.zoom(true);

        let snapshot = ConversationState::capture(&app);
        app.input = InputBuffer::new();
        app.navigation = NavigationState::default();
        snapshot.restore(&mut app);

        assert_eq!(app.input.text, "draft 👩‍💻 e\u{301}");
        assert_eq!(app.navigation.screen, Screen::Workbench);
        assert_eq!(app.navigation.focus, NavigationFocus::WorkbenchContent);
        let presentation = app.navigation.presentation();
        assert_eq!(presentation.tab, WorkbenchTab::Diff);
        assert_eq!(presentation.round, Some(3));
        assert!(presentation.split && presentation.zoomed && presentation.pinned);
        app.navigation.zoom(false);
        assert!(app.navigation.presentation().split);
    }

    #[test]
    fn snapshot_preserves_current_remembered_and_background_focus_independently() {
        let mut app = App::test_app();
        app.set_focus(Focus::Graph);
        app.navigation.screen = Screen::Workbench;
        app.navigation.target = WorkbenchTarget::Agent("reviewer:t1".into());
        app.navigation.update_presentation(|p| {
            p.focus = NavigationFocus::WorkbenchTerminal;
            p.tab = WorkbenchTab::Checks;
        });
        app.navigation.focus = NavigationFocus::Composer;
        app.workbench_memory.insert(
            ("reviewer:t1".into(), WorkbenchTab::Checks),
            (7, "e\u{301} 👩‍💻".into()),
        );
        let saved = ConversationState::capture(&app);
        app.navigation = NavigationState::default();
        app.workbench_memory.clear();
        saved.restore(&mut app);
        assert_eq!(app.navigation.focus, NavigationFocus::Composer);
        assert_eq!(
            app.navigation.presentation().focus,
            NavigationFocus::WorkbenchTerminal
        );
        assert_eq!(app.navigation.background_focus, Focus::Graph);
        assert_eq!(
            app.workbench_memory[&("reviewer:t1".into(), WorkbenchTab::Checks)],
            (7, "e\u{301} 👩‍💻".into())
        );
    }

    #[test]
    fn presentation_preferences_are_scoped_to_stable_target() {
        let mut nav = NavigationState {
            target: WorkbenchTarget::Agent("worker:t1".into()),
            ..NavigationState::default()
        };
        nav.update_presentation(|p| p.tab = WorkbenchTab::Checks);
        nav.target = WorkbenchTarget::Agent("reviewer:t1".into());
        assert_eq!(nav.presentation().tab, WorkbenchTab::Summary);
        nav.target = WorkbenchTarget::Agent("worker:t1".into());
        assert_eq!(nav.presentation().tab, WorkbenchTab::Checks);
    }

    #[test]
    fn conversation_snapshot_captures_focus_changed_by_pointer_transition() {
        let mut app = App::test_app();
        app.navigation.screen = Screen::Workbench;
        app.navigation.focus = NavigationFocus::WorkbenchContent;
        app.set_focus(Focus::Graph);

        let snapshot = ConversationState::capture(&app);
        app.set_focus(Focus::Chat);
        snapshot.restore(&mut app);

        assert_eq!(app.navigation.background_focus, Focus::Graph);
        assert_eq!(app.navigation.focus, NavigationFocus::Graph);
    }

    #[test]
    fn conversation_snapshot_preserves_workbench_content_focus_opened_from_graph() {
        let mut app = App::test_app();
        app.set_focus(Focus::Graph);
        assert_eq!(app.navigation.background_focus, Focus::Graph);
        app.navigation.screen = Screen::Workbench;
        app.navigation.focus = NavigationFocus::WorkbenchContent;

        let snapshot = ConversationState::capture(&app);
        app.navigation.screen = Screen::Graph;
        app.navigation.focus = NavigationFocus::Graph;
        snapshot.restore(&mut app);

        assert_eq!(app.navigation.screen, Screen::Workbench);
        assert_eq!(app.navigation.focus, NavigationFocus::WorkbenchContent);
        assert_eq!(app.navigation.background_focus, Focus::Graph);
    }
}
