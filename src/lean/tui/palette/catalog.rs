pub(super) const SELECTION_HELP: &str =
    "Ctrl+Y           copy selection (drag / double click selects)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    InterviewQuestion,
    Requirements,
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
}

pub(super) const COMMANDS: [(Command, &str); 4] = [
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
        "Conversation details (inspector) · Ctrl+I",
    ),
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
    SwitchConversation(String),
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
}

#[derive(Debug)]
pub struct Picker {
    pub kind: Kind,
    pub query: String,
    pub selected: usize,
    pub conversations: Vec<ConversationItem>,
    pub projects: Vec<ProjectItem>,
    pub confirm_delete: bool,
}
