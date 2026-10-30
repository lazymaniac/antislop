//! Clipboard text normalization for editor insertion.

use super::view::clean;

/// Normalizes line endings and sanitizes terminal controls without clipping text.
pub fn sanitize_paste(raw: &str) -> String {
    clean(&raw.replace("\r\n", "\n").replace('\r', "\n"))
}

/// Flattens multiline paste for single-line search and picker inputs.
pub fn sanitize_single_line_paste(raw: &str) -> String {
    let text = sanitize_paste(raw);
    text.replace('\n', " ").trim().to_string()
}
