mod control;
mod conversation;
mod history;
mod ledger;
mod session;
mod terminal;
pub use control::ControlRequest;
pub use conversation::ConversationRequest;
pub use history::HistoryRequest;
pub use ledger::LedgerRequest;
pub use session::SessionRequest;
pub use terminal::TerminalRequest;

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Terminal(TerminalRequest),
    Session(SessionRequest),
    History(HistoryRequest),
    Ledger(LedgerRequest),
    Conversation(ConversationRequest),
    Control(ControlRequest),
}
