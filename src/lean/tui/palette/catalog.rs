pub(super) const SELECTION_HELP: &str =
    "Ctrl+Y           copy selection (drag / double click selects)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    InterviewQuestion,
    ApproveRequirements,
    Requirements,
    Config,
    Accept,
    Reject,
    Plan,
    Plans,
    Resume,
    Cancel,
    Pause,
    Continue,
    Autonomous,
    Supervised,
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
}

pub(super) const COMMANDS: [(Command, &str); 27] = [
    (
        Command::InterviewQuestion,
        "Open pending interview question · /interview",
    ),
    (
        Command::ApproveRequirements,
        "Approve requirements and start planning · /planning",
    ),
    (
        Command::Requirements,
        "Browse and edit requirements · /requirements",
    ),
    (Command::Config, "Edit configuration live · /config"),
    (Command::Accept, "Accept proposed plan · Ctrl+A"),
    (Command::Reject, "Reject proposed plan · /reject"),
    (Command::Plan, "Open plan document viewer · Ctrl+L / /plan"),
    (
        Command::Plans,
        "Browse historical plan graphs and reports · /plans",
    ),
    (Command::Resume, "Resume plan · Ctrl+R"),
    (Command::Cancel, "Cancel running work · Ctrl+X"),
    (
        Command::Pause,
        "Pause new steps (current ones finish) · /pause",
    ),
    (Command::Continue, "Continue paused work · /continue"),
    (
        Command::Autonomous,
        "Autonomous mode: approved work progresses automatically · /mode autonomous",
    ),
    (
        Command::Supervised,
        "Supervised mode: plans wait for approval · /mode supervised",
    ),
    (Command::NewConversation, "New conversation · Ctrl+N"),
    (Command::SwitchConversation, "Switch conversation · Ctrl+O"),
    (
        Command::Projects,
        "Open or switch project · Ctrl+G / /project",
    ),
    (Command::Find, "Find agent · /"),
    (Command::Search, "Search conversation · Ctrl+F"),
    (Command::Details, "Selected agent details · Enter"),
    (
        Command::ConversationDetails,
        "Conversation details (inspector) · Ctrl+I",
    ),
    (Command::AddressMessage, "Address message · @"),
    (Command::Diff, "Selected agent task diff · d"),
    (Command::Comment, "Comment on selected element · c"),
    (Command::Filter, "Next graph relation filter · f"),
    (Command::Help, "Help · ?"),
    (Command::Quit, "Detach (work continues) · q"),
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
}
