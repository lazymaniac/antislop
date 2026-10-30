//! Public editor facade for TUI input and grapheme-safe text editing.

#[cfg(test)]
mod tests;

pub use crate::lean::tui::buffer::InputBuffer;
pub use crate::lean::tui::layout::lines::{
    VisualLineInfo, compute_visual_lines, visual_text_lines,
};
pub use crate::lean::tui::paste::{sanitize_paste, sanitize_single_line_paste};
