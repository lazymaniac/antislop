//! Field-based configuration draft. Only a successful daemon reply marks it saved.
mod draft;
mod edit;
mod model;
mod render;

use super::{InputBuffer, Theme, sanitize_single_line_paste};
use crate::config::{Config, ProviderKind, WorkKind};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Clear, Paragraph},
};
use serde_json::Value;
use std::collections::BTreeMap;

pub use model::{ConfigEditor, Field};

#[cfg(test)]
mod tests;
