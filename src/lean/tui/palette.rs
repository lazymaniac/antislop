//! Ctrl+P command palette and active-view help overlays.
mod catalog;
mod help;
mod picker;
mod picker_layout;

use super::graph::Graph;
use super::view::{View, agent_name, dim, truncate};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

pub use catalog::{Action, Command, ConversationItem, Kind, Picker, ProjectItem};
pub use help::Help;

#[cfg(test)]
mod tests;
