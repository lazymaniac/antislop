use super::super::{Comment, DiffLine, FileDiff, View, clean};
use crate::lean::tui::theme::Theme;
use ratatui::{
    style::{Modifier, Style},
    text::Line,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The gutter of a diff line: two line numbers and the sign, then `│`.
const GUTTER: usize = 14;
/// Where a hunk header's text starts.
const HUNK_INDENT: usize = 12;

pub(super) fn file_comments<'a>(view: &'a View, target: &str) -> Vec<&'a Comment> {
    view.comments
        .values()
        .filter(|comment| comment.status != "deleted")
        .filter(|comment| {
            comment.target == target || comment.target.starts_with(&format!("{target}:"))
        })
        .collect()
}

#[derive(Clone, Copy)]
pub(super) struct DiffRows<'a> {
    pub(super) file: &'a FileDiff,
    pub(super) comments: &'a [&'a Comment],
    pub(super) task: &'a str,
    pub(super) file_index: usize,
    pub(super) file_count: usize,
    pub(super) compact: bool,
    pub(super) selected_line: usize,
    pub(super) on_files: bool,
    pub(super) width: usize,
    pub(super) theme: &'a Theme,
}

pub(super) fn diff_rows(input: DiffRows<'_>) -> (Vec<Line<'static>>, usize) {
    let target = format!("diff:{}:{}", input.task, input.file.path);
    let mut rows = diff_file_prefix(input, &target);
    let cursor_row = append_diff_lines(&mut rows, input, &target);
    (rows, cursor_row)
}

fn diff_file_prefix(input: DiffRows<'_>, target: &str) -> Vec<Line<'static>> {
    let DiffRows {
        file,
        comments,
        file_index,
        file_count,
        compact,
        width,
        theme,
        ..
    } = input;
    let mut rows = vec![file_title(
        file, file_index, file_count, compact, width, theme,
    )];
    append_comments(
        &mut rows,
        comments
            .iter()
            .copied()
            .filter(|comment| comment.target == target),
        theme,
    );
    if file.binary {
        rows.push(Line::styled("  binary file", theme.dim()));
    }
    rows
}

fn append_diff_lines(rows: &mut Vec<Line<'static>>, input: DiffRows<'_>, target: &str) -> usize {
    let DiffRows {
        file,
        comments,
        selected_line,
        on_files,
        width,
        theme,
        ..
    } = input;
    let mut cursor_row = 0;
    for (index, line) in file.lines.iter().enumerate() {
        if index == selected_line && !on_files {
            cursor_row = rows.len();
        }
        rows.extend(diff_line(
            line,
            index == selected_line && !on_files,
            width,
            theme,
        ));
        if let Some(key) = comment_target(target, line) {
            append_comments(
                rows,
                comments
                    .iter()
                    .copied()
                    .filter(|comment| comment.target == key),
                theme,
            );
        }
    }
    cursor_row
}

fn file_title(
    file: &FileDiff,
    index: usize,
    count: usize,
    compact: bool,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let title = if let Some(old) = &file.old_path {
        format!(" {old} {} {}", theme.glyphs().to, file.path)
    } else if compact && count > 1 {
        format!(" [{}/{}] {} [Tab: files]", index + 1, count, file.path)
    } else {
        format!(" {}", file.path)
    };
    surface_row(
        theme.fit(&title, width),
        width,
        theme.raised().patch(theme.text()),
    )
}

fn surface_row(mut text: String, width: usize, style: Style) -> Line<'static> {
    if style.bg.is_some() {
        text.push_str(&" ".repeat(width.saturating_sub(text.width())));
    }
    Line::styled(text, style)
}

fn append_comments<'a>(
    rows: &mut Vec<Line<'static>>,
    comments: impl Iterator<Item = &'a Comment>,
    theme: &Theme,
) {
    rows.extend(comments.map(|comment| comment_line(comment, theme)));
}

fn comment_line(comment: &Comment, theme: &Theme) -> Line<'static> {
    let resolved = if comment.status == "resolved" {
        " (resolved)"
    } else {
        ""
    };
    let style = if comment.status == "open" {
        theme.accent()
    } else {
        theme.dim()
    };
    Line::styled(
        format!(
            "              {} #{} {}{resolved}",
            theme.chrome("✎"),
            comment.id,
            clean(&comment.text)
        ),
        style,
    )
}

fn comment_target(target: &str, line: &DiffLine) -> Option<String> {
    match (line.kind.as_str(), line.old, line.new) {
        ("del", Some(old), _) => Some(format!("{target}:old{old}")),
        (_, _, Some(new)) if line.kind != "hunk" => Some(format!("{target}:{new}")),
        _ => None,
    }
}

/// One diff line wrapped to `width`: the first row carries the gutter, the
/// rows after it continue the text under the code column.
fn diff_line(line: &DiffLine, selected: bool, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let (sign, style) = diff_style(&line.kind, theme);
    let style = if selected {
        style.add_modifier(if theme.color {
            Modifier::UNDERLINED
        } else {
            Modifier::REVERSED
        })
    } else {
        style
    };
    let (first, text, indent) = diff_text(line, sign, theme);
    let room = width.saturating_sub(indent);
    if room < 8 {
        // Too narrow to wrap under the gutter: cut the line as before.
        return vec![surface_row(
            theme.fit(&format!("{first}{text}"), width),
            width,
            style,
        )];
    }
    let continuation = if indent == GUTTER {
        format!("{}{}", " ".repeat(GUTTER - 1), theme.glyphs().vertical)
    } else {
        " ".repeat(indent)
    };
    wrap_graphemes(&text, room)
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            let prefix = if index == 0 { &first } else { &continuation };
            surface_row(format!("{prefix}{chunk}"), width, style)
        })
        .collect()
}

/// `text` cut into rows at most `width` columns wide, on grapheme cluster
/// boundaries; an empty text is one empty row.
fn wrap_graphemes(text: &str, width: usize) -> Vec<String> {
    let mut rows = vec![String::new()];
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let grapheme_width = grapheme.width();
        if used + grapheme_width > width && used > 0 {
            rows.push(String::new());
            used = 0;
        }
        if let Some(row) = rows.last_mut() {
            row.push_str(grapheme);
        }
        used += grapheme_width;
    }
    rows
}

fn diff_style(kind: &str, theme: &Theme) -> (&'static str, Style) {
    match kind {
        "add" => (
            "+",
            theme.style(
                Style::default()
                    .fg(crate::lean::tui::theme::ayu::DIFF_ADD)
                    .bg(crate::lean::tui::theme::ayu::DIFF_ADD_BG),
            ),
        ),
        "del" => (
            "-",
            theme.style(
                Style::default()
                    .fg(crate::lean::tui::theme::ayu::DIFF_REMOVE)
                    .bg(crate::lean::tui::theme::ayu::DIFF_REMOVE_BG),
            ),
        ),
        "hunk" => (
            "",
            theme.style(Style::default().fg(crate::lean::tui::theme::ayu::BLUE)),
        ),
        _ => (" ", Style::default()),
    }
}

/// A diff line's first-row prefix, its text and the column the text starts at.
fn diff_text(line: &DiffLine, sign: &str, theme: &Theme) -> (String, String, usize) {
    if line.kind == "hunk" {
        (" ".repeat(HUNK_INDENT), line.text.clone(), HUNK_INDENT)
    } else {
        (
            format!(
                "{} {} {sign}{}",
                diff_number(line.old),
                diff_number(line.new),
                theme.glyphs().vertical
            ),
            line.text.replace('\t', "    "),
            GUTTER,
        )
    }
}

fn diff_number(value: Option<u32>) -> String {
    value.map_or_else(|| "     ".to_owned(), |value| format!("{value:>5}"))
}

pub(super) fn scroll_to_cursor(
    mut scroll: usize,
    cursor: usize,
    row_count: usize,
    height: usize,
) -> usize {
    if cursor < scroll {
        scroll = cursor.saturating_sub(1);
    } else if cursor >= scroll + height {
        scroll = cursor + 1 - height;
    }
    scroll.min(row_count.saturating_sub(height))
}
