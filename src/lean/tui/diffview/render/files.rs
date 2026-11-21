use super::super::{FileDiff, truncate};
use crate::lean::tui::theme::Theme;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use unicode_width::UnicodeWidthStr;

pub(super) fn draw_files(
    frame: &mut Frame,
    area: Rect,
    files: &[FileDiff],
    selected: usize,
    focused: bool,
    compact: bool,
    theme: &Theme,
) {
    let block = if compact {
        Block::default()
            .borders(Borders::ALL)
            .border_style(if focused {
                theme.separator(true)
            } else {
                theme.dim()
            })
            .title(" Changed files [Tab: switch to diff] ")
    } else {
        Block::default().borders(Borders::RIGHT)
    };
    let block = block
        .border_set(theme.borders())
        .border_style(theme.separator(focused));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = usize::from(inner.width);
    let height = usize::from(inner.height);
    let start = selected.saturating_sub(height.saturating_sub(1));
    let rows = file_rows(files, selected, start, height, width, focused, theme);
    frame.render_widget(Paragraph::new(rows), inner);
}

fn file_rows(
    files: &[FileDiff],
    selected: usize,
    start: usize,
    height: usize,
    width: usize,
    focused: bool,
    theme: &Theme,
) -> Vec<Line<'static>> {
    files
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(index, file)| file_row(file, index == selected, focused, width, theme))
        .collect()
}

fn file_row(
    file: &FileDiff,
    selected: bool,
    focused: bool,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let (mark, color) = file_marker(&file.status);
    let counts = theme.chrome(&format!(" +{} −{}", file.added, file.removed));
    let path_style = if !selected {
        Style::default()
    } else if focused {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };
    Line::from(vec![
        Span::styled(format!("{mark} "), theme.style(Style::default().fg(color))),
        Span::styled(
            truncate(&file.path, width.saturating_sub(counts.width() + 2)),
            path_style,
        ),
        Span::styled(counts, theme.dim()),
    ])
}

fn file_marker(status: &str) -> (&'static str, ratatui::style::Color) {
    match status {
        "added" => ("A", super::super::super::theme::ayu::GREEN),
        "deleted" => ("D", super::super::super::theme::ayu::RED),
        "renamed" => ("R", super::super::super::theme::ayu::YELLOW),
        _ => ("M", super::super::super::theme::ayu::BLUE),
    }
}
