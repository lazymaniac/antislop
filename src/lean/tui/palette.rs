//! Ctrl+P command palette and active-view help overlays.
mod app;
mod catalog;
mod help;
mod picker;
mod picker_layout;

use super::view::{View, agent_name, truncate};
#[cfg(test)]
use ratatui::style::Modifier;
use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Block, Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

pub use catalog::{Action, Command, ConversationItem, Kind, Picker, ProjectItem};
pub use help::Help;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod app_tests;
