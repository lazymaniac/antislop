mod control;
mod conversation;
mod history;
mod session;
mod terminal;
pub use control::ControlRequest;
pub use conversation::ConversationRequest;
pub use history::HistoryRequest;
pub use session::SessionRequest;
pub use terminal::TerminalRequest;

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Terminal(TerminalRequest),
    Session(SessionRequest),
    History(HistoryRequest),
    Conversation(ConversationRequest),
    Control(ControlRequest),
}
