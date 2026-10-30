//! How each supported CLI is started as an interactive process in a terminal,
//! wired to the daemon's `finish_turn` gateway. Everything is passed per
//! launch (flags, `-c` overrides, environment): no CLI's own configuration
//! files are written. The human can then use the CLI exactly as usual.
use portable_pty::CommandBuilder;

mod providers;
pub(crate) use providers::command;

pub(crate) struct Launch {
    pub command: CommandBuilder,
    /// The CLI's own session identifier when it is known before the process
    /// starts (Claude is told it; any CLI is told it when resuming).
    pub session: Option<String>,
    pub settings: Option<tempfile::NamedTempFile>,
}

#[cfg(test)]
#[path = "launch/tests.rs"]
mod tests;
