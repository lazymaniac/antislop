pub(super) const SELECTION_HELP: &str =
    "Ctrl+Y           copy selection (drag / double click selects)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    InterviewQuestion,
    Config,
    Reject,
    Plan,
    Plans,
    Resume,
    Cancel,
    Pause,
    Continue,
    PauseOrContinue,
    NewConversation,
    SwitchConversation,
    Find,
    Search,
    Details,
    Diff,
    Comment,
    Filter,
    Help,
    Quit,
    AddressMessage,
    ConversationDetails,
    Projects,
    TaskActions,
    ApprovalsSupervised,
    ApprovalsAutonomous,
    PendingApprovals,
    TakeControl,
    SelectTeam(crate::lean::tasks::Team),
    OpenRigorPicker,
    EditTask,
    AddDependentTask,
    SplitTask,
    HoldTask,
    RetireTask,
    SteerAgent,
    AcceptPlan,
    DirectExecute,
    LinkTask,
}

pub(super) const COMMANDS: [(Command, &str); 8] = [
    (
        Command::PendingApprovals,
        "Answer agents' pending approval requests",
    ),
    (
        Command::ApprovalsSupervised,
        "Approvals: you decide what agents may do",
    ),
    (
        Command::ApprovalsAutonomous,
        "Approvals: the orchestrator decides, you can overrule",
    ),
    (
        Command::ConversationDetails,
        "Conversation details (inspector) · Ctrl+T",
    ),
    (
        Command::TakeControl,
        "Take control: steer this conversation from this window",
    ),
    (Command::Projects, "Open projects · Ctrl+G"),
    (Command::SwitchConversation, "Open sessions · Ctrl+O"),
    (Command::NewConversation, "New session · Ctrl+N"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectItem {
    pub path: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationItem {
    pub id: String,
    pub project: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Run(Command),
    /// Select this graph node.
    Select(String),
    /// Show this entry of the conversation.
    Show(usize),
    /// Switch to another conversation.
    SwitchConversation {
        id: String,
        project: String,
    },
    /// Address a message to a recipient.
    Address(String),
    /// Open this project (a directory) or switch to it.
    OpenProject(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Commands,
    Agents,
    History,
    Conversations,
    Recipients,
    Projects,
    SlashCommands,
    Rigor,
}

#[derive(Debug)]
pub struct Picker {
    pub status: Option<String>,
    pub kind: Kind,
    pub query: String,
    pub selected: usize,
    pub conversations: Vec<ConversationItem>,
    pub projects: Vec<ProjectItem>,
    pub confirm_delete: bool,
    pub delete_target: Option<ConversationItem>,
    pub recommendation: Option<(crate::lean::tasks::Team, String)>,
}
