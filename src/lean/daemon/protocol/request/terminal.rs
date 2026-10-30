#[derive(Debug, Clone, PartialEq)]
pub enum TerminalRequest {
    TerminalOpen {
        conversation: String,
        role: String,
        columns: u16,
        rows: u16,
        retry: bool,
    },
    TerminalRead {
        conversation: String,
        role: String,
        handle: String,
        after: u64,
    },
    TerminalInput {
        conversation: String,
        role: String,
        handle: String,
        sequence: u64,
        input: crate::lean::terminal::Input,
    },
    TerminalResize {
        conversation: String,
        role: String,
        handle: String,
        columns: u16,
        rows: u16,
    },
    TerminalDetach {
        conversation: String,
        role: String,
        handle: String,
    },
}
