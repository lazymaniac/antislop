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
    PauseOrContinue,
    ToggleMode,
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
    TaskActions,
}

pub(super) const COMMANDS: [(Command, &str); 30] = [
    (
        Command::InterviewQuestion,
        "Open pending interview question · F3",
    ),
    (
        Command::ApproveRequirements,
        "Approve requirements and start planning · Ctrl+A",
    ),
    (Command::Requirements, "Browse and edit requirements · F2"),
    (Command::Config, "Edit configuration live · F8"),
    (Command::Accept, "Accept proposed plan · Ctrl+A"),
    (Command::Reject, "Reject proposed plan · F7"),
    (Command::Plan, "Open plan document viewer · Ctrl+L"),
    (
        Command::Plans,
        "Browse historical plan graphs and reports · F4",
    ),
    (Command::Resume, "Resume plan · Ctrl+R"),
    (Command::Cancel, "Cancel running work · Ctrl+X"),
    (Command::Pause, "Pause new steps (current ones finish) · F6"),
    (Command::Continue, "Continue paused work · F6"),
    (Command::PauseOrContinue, "Pause or continue work · F6"),
    (
        Command::ToggleMode,
        "Switch autonomous / supervised mode · Ctrl+T",
    ),
    (
        Command::Autonomous,
        "Autonomous mode: approved work progresses automatically · Ctrl+T",
    ),
    (
        Command::Supervised,
        "Supervised mode: plans wait for approval · Ctrl+T",
    ),
    (Command::NewConversation, "New conversation · Ctrl+N"),
    (Command::SwitchConversation, "Switch conversation · Ctrl+O"),
    (Command::Projects, "Open or switch project · Ctrl+G"),
    (Command::TaskActions, "Actions for a stopped task · F5"),
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
    (
        Command::Filter,
        "Graph filters: relations, states, providers · f",
    ),
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
