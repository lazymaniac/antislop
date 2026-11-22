//! Lightweight message markup; wrapping always keeps extended graphemes intact.
use crate::lean::tui::theme::Theme;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub(super) fn body(text: &str, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let mut rows = Vec::new();
    let mut fence = None;
    for line in theme.clean(text).replace('\t', "    ").lines() {
        let graphemes: Vec<_> = line.graphemes(true).collect();
        if let [marker @ ("`" | "~"), second, third, rest @ ..] = graphemes.as_slice()
            && marker == second
            && marker == third
            && fence.is_none_or(|open| open == *marker)
        {
            if fence.is_some() {
                fence = None;
            } else {
                fence = Some(*marker);
                let language = rest.concat();
                if !language.is_empty() {
                    rows.extend(wrap(vec![Span::styled(language, theme.dim())], width));
                }
            }
            continue;
        }
        if fence.is_some() {
            rows.extend(wrap(
                vec![Span::styled(line.to_owned(), theme.accent())],
                width,
            ));
            continue;
        }
        let level = graphemes
            .iter()
            .take_while(|grapheme| **grapheme == "#")
            .count();
        let (text, style) = if (1..=6).contains(&level) && graphemes.get(level) == Some(&" ") {
            (
                graphemes[level + 1..].concat(),
                theme.accent().add_modifier(Modifier::BOLD),
            )
        } else if graphemes.first() == Some(&">") && graphemes.get(1) == Some(&" ") {
            (
                format!("{} {}", theme.glyphs().vertical, graphemes[2..].concat()),
                theme.dim(),
            )
        } else {
            (line.to_owned(), theme.text())
        };
        rows.extend(wrap(inline(&text, style, theme), width));
    }
    rows
}

fn inline(text: &str, base: Style, theme: &Theme) -> Vec<Span<'static>> {
    let graphemes: Vec<_> = text.graphemes(true).collect();
    let mut spans = Vec::new();
    let mut plain = String::new();
    let mut index = 0;
    while index < graphemes.len() {
        let (marker, length, style) = match graphemes[index] {
            "`" => ("`", 1, theme.accent()),
            "*" if graphemes.get(index + 1) == Some(&"*") => {
                ("*", 2, base.add_modifier(Modifier::BOLD))
            }
            "*" => ("*", 1, base.add_modifier(Modifier::ITALIC)),
            _ => {
                plain.push_str(graphemes[index]);
                index += 1;
                continue;
            }
        };
        let close = (index + length..graphemes.len()).find(|&at| {
            graphemes[at] == marker && (length == 1 || graphemes.get(at + 1) == Some(&marker))
        });
        if let Some(close) = close {
            if !plain.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut plain), base));
            }
            spans.push(Span::styled(
                graphemes[index + length..close].concat(),
                style,
            ));
            index = close + length;
        } else {
            plain.push_str(graphemes[index]);
            index += 1;
        }
    }
    if !plain.is_empty() {
        spans.push(Span::styled(plain, base));
    }
    spans
}

fn wrap(spans: Vec<Span<'static>>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut row: Vec<(String, Style)> = Vec::new();
    let mut columns = 0;
    for span in spans {
        for grapheme in span.content.graphemes(true) {
            let cells = grapheme.width();
            if columns > 0 && columns + cells > width {
                let space = row
                    .iter()
                    .rposition(|(grapheme, _)| grapheme == " ")
                    .filter(|&at| row[..at].iter().any(|(grapheme, _)| grapheme != " "));
                let remainder = if grapheme == " " {
                    Vec::new()
                } else if let Some(space) = space {
                    let remainder = row.split_off(space + 1);
                    row.truncate(space);
                    remainder
                } else {
                    Vec::new()
                };
                rows.push(styled_row(std::mem::take(&mut row)));
                row = remainder;
                columns = row.iter().map(|(grapheme, _)| grapheme.width()).sum();
                if grapheme == " " {
                    continue;
                }
                if columns > 0 && columns + cells > width {
                    rows.push(styled_row(std::mem::take(&mut row)));
                    columns = 0;
                }
            }
            row.push((grapheme.to_owned(), span.style));
            columns += cells;
        }
    }
    rows.push(styled_row(row));
    rows
}

fn styled_row(graphemes: Vec<(String, Style)>) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (grapheme, style) in graphemes {
        if let Some(last) = spans.last_mut()
            && last.style == style
        {
            last.content.to_mut().push_str(&grapheme);
        } else {
            spans.push(Span::styled(grapheme, style));
        }
    }
    Line::from(spans)
}
