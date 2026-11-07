use super::super::view::wrap;
use super::{Form, InputBuffer, Question, Theme};
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

impl Form {
    pub fn draw(&mut self, frame: &mut Frame, screen: Rect, theme: &Theme) {
        let (area, block) = interview_box(screen, theme);
        let inner_width = usize::from(area.width.saturating_sub(3).max(1));
        let custom = self.selected == self.question.options.len();
        let (rows, selected_rows, input_row, scope_insert) = self.rows(inner_width, custom, theme);
        let has_scope = self.scope_summary.is_some();
        let scope_count = self.scope_summary.as_deref().map_or(0, |summary| {
            wrapped_scope(summary, inner_width, 0, 0, theme).0
        });
        let row_count = rows.len().saturating_add(scope_count.saturating_sub(1));
        let area = content_box(area, row_count);
        let viewport = usize::from(area.height.saturating_sub(2).max(1));
        self.scroll_step = viewport.saturating_sub(1).max(1);
        let (cursor_col, cursor_row) = self.input.visual_cursor(inner_width);
        let scope_offset = if has_scope {
            scope_count.saturating_sub(1)
        } else {
            0
        };
        let (target_start, target_end) = if custom {
            let target = input_row + cursor_row;
            (target + scope_offset, target + scope_offset)
        } else {
            (
                selected_rows.0 + scope_offset,
                selected_rows.1 + scope_offset,
            )
        };
        let insertion = scope_insert.unwrap_or(0);
        let scroll = self.visible_scroll(target_start, target_end, viewport, row_count);
        let scope = self.scope_summary.as_deref();
        let visible = visible_rows(
            rows,
            scope,
            (insertion, scope_count),
            scroll..scroll.saturating_add(viewport),
            inner_width,
            theme,
        );
        render_interview_rows(frame, area, visible, theme, block);
        self.place_cursor(frame, area, custom, target_start, cursor_col);
    }

    fn rows(
        &self,
        width: usize,
        custom: bool,
        theme: &Theme,
    ) -> (Vec<Line<'static>>, (usize, usize), usize, Option<usize>) {
        let scope = self.scope_summary.is_some();
        let mut rows = question_rows(&self.question, scope, width, theme);
        let option_selected = option_rows(
            &mut rows,
            &self.question.options,
            self.selected,
            width,
            theme,
        );
        let selected = if custom {
            (rows.len(), rows.len())
        } else {
            option_selected
        };
        let input_row = append_answer_rows(
            &mut rows,
            &self.input,
            self.error.as_deref(),
            custom,
            width,
            theme,
        );
        append_interview_footer(&mut rows, self.sending, theme);
        let scope_insert = scope.then_some(1);
        (rows, selected, input_row, scope_insert)
    }

    fn visible_scroll(
        &mut self,
        target_start: usize,
        target_end: usize,
        viewport: usize,
        row_count: usize,
    ) -> usize {
        if !self.manual_scroll {
            if target_start < self.scroll {
                self.scroll = target_start;
            }
            if target_end >= self.scroll + viewport {
                self.scroll = target_end + 1 - viewport;
            }
        }
        self.scroll = self.scroll.min(row_count.saturating_sub(viewport));
        self.scroll
    }

    fn place_cursor(&self, frame: &mut Frame, area: Rect, custom: bool, target: usize, col: u16) {
        if !custom || self.sending || self.manual_scroll || area.width < 3 || area.height < 3 {
            return;
        }
        frame.set_cursor_position((
            area.x + 1 + col.min(area.width.saturating_sub(3)),
            area.y
                + 1
                + (target.saturating_sub(self.scroll) as u16).min(area.height.saturating_sub(3)),
        ));
    }
}

fn content_box(area: Rect, row_count: usize) -> Rect {
    let height = row_count.saturating_add(2).min(usize::from(area.height)) as u16;
    Rect::new(
        area.x,
        area.y + area.height.saturating_sub(height) / 2,
        area.width,
        height,
    )
}

fn render_interview_rows<'a>(
    frame: &mut Frame,
    area: Rect,
    visible: Vec<Line<'a>>,
    theme: &Theme,
    block: Block<'static>,
) {
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(visible).style(theme.card()).block(block),
        area,
    );
}

fn question_rows(
    question: &Question,
    has_scope_summary: bool,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut rows = Vec::new();
    if has_scope_summary {
        append_wrapped(&mut rows, "Scope of changes:", width, theme.title(), theme);
    } else {
        append_wrapped(&mut rows, &question.context, width, theme.dim(), theme);
    }
    rows.push(Line::raw(""));
    append_wrapped(&mut rows, &question.question, width, theme.title(), theme);
    rows.push(Line::raw(""));
    rows
}

/// Sanitizes and wraps the scope incrementally. Only requested rows and the
/// current chunk are retained; an oversized cluster is kept whole while it is
/// sanitized so wrapping never separates its graphemes.
fn wrapped_scope<'a>(
    text: &'a str,
    width: usize,
    start: usize,
    count: usize,
    theme: &Theme,
) -> (usize, Vec<Line<'a>>) {
    let width = width.max(1);
    let mut sanitizer = super::super::sanitize::Sanitizer::default();
    let mut chunk = String::with_capacity(1024);
    let mut output = String::with_capacity(1024);
    let mut wrapped = ScopeWrapper::new(width, start, count, theme);
    let mut graphemes = text.grapheme_indices(true).peekable();
    while let Some((offset, grapheme)) = graphemes.next() {
        if grapheme.len() > 1024 {
            let mut end = offset + grapheme.len();
            while let Some((next_offset, next)) = graphemes.peek().copied() {
                if next_offset != end || next.len() <= 1024 {
                    break;
                }
                end += next.len();
                graphemes.next();
            }
            let oversized = &text[offset..end];
            if !chunk.is_empty() {
                sanitizer.append_chunk(&chunk, &mut output);
                wrapped.consume(&output);
                chunk.clear();
                output.clear();
            }
            if sanitizer.can_borrow_visible(oversized) {
                if !wrapped.row.is_empty() {
                    wrapped.emit_row();
                }
                wrapped.emit_borrowed(if theme.ascii { "?" } else { oversized });
            } else {
                // Sanitize scalar by scalar, but wrap only after the complete
                // cluster so a long run of combining marks stays together.
                for character in oversized.chars() {
                    let mut scalar = [0; 4];
                    let scalar = character.encode_utf8(&mut scalar);
                    sanitizer.append_chunk(scalar, &mut output);
                }
                wrapped.consume(&output);
                output.clear();
            }
            continue;
        }
        if chunk.len() + grapheme.len() > 1024 && !chunk.is_empty() {
            sanitizer.append_chunk(&chunk, &mut output);
            wrapped.consume(&output);
            chunk.clear();
            output.clear();
        }
        chunk.push_str(grapheme);
    }
    sanitizer.append_chunk(&chunk, &mut output);
    wrapped.consume(&output);
    wrapped.finish()
}

struct ScopeWrapper<'a, 'theme> {
    width: usize,
    start: usize,
    count: usize,
    theme: &'theme Theme,
    row: String,
    columns: usize,
    last_space: Option<(usize, usize)>,
    total: usize,
    selected: Vec<Line<'a>>,
}

impl<'a, 'theme> ScopeWrapper<'a, 'theme> {
    fn new(width: usize, start: usize, count: usize, theme: &'theme Theme) -> Self {
        ScopeWrapper {
            width,
            start,
            count,
            theme,
            row: String::new(),
            columns: 0,
            last_space: None,
            total: 0,
            selected: Vec::with_capacity(count),
        }
    }

    fn emit_row(&mut self) {
        if self.total >= self.start && self.total < self.start.saturating_add(self.count) {
            self.selected
                .push(Line::styled(self.theme.clean(&self.row), self.theme.dim()));
        }
        self.total += 1;
        self.row.clear();
        self.columns = 0;
        self.last_space = None;
    }

    fn emit_borrowed(&mut self, text: &'a str) {
        if self.total >= self.start && self.total < self.start.saturating_add(self.count) {
            self.selected.push(Line::styled(text, self.theme.dim()));
        }
        self.total += 1;
    }

    fn consume(&mut self, clean: &str) {
        for grapheme in clean.graphemes(true) {
            if grapheme == "\n" {
                self.emit_row();
                continue;
            }
            let cells = UnicodeWidthStr::width(grapheme);
            if self.columns > 0 && self.columns + cells > self.width {
                if grapheme.chars().all(char::is_whitespace) {
                    self.emit_row();
                    continue;
                }
                if let Some((space_start, space_end)) = self.last_space.take() {
                    let carry = self.row.split_off(space_end);
                    self.row.truncate(space_start);
                    self.emit_row();
                    self.row = carry;
                    self.columns = UnicodeWidthStr::width(self.row.as_str());
                    self.last_space = self
                        .row
                        .grapheme_indices(true)
                        .rfind(|(_, item)| item.chars().all(char::is_whitespace))
                        .map(|(start, item)| (start, start + item.len()));
                } else {
                    self.emit_row();
                }
            }
            let start = self.row.len();
            self.row.push_str(grapheme);
            self.columns += cells;
            if grapheme.chars().all(char::is_whitespace) && self.columns > cells {
                self.last_space = Some((start, self.row.len()));
            }
        }
    }

    fn finish(mut self) -> (usize, Vec<Line<'a>>) {
        if !self.row.is_empty() || self.total == 0 {
            self.emit_row();
        }
        (self.total, self.selected)
    }
}

fn visible_rows<'a>(
    rows: Vec<Line<'static>>,
    scope: Option<&'a str>,
    scope_rows: (usize, usize),
    viewport: std::ops::Range<usize>,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'a>> {
    let (insertion, scope_count) = scope_rows;
    let scroll = viewport.start;
    let viewport_len = viewport.end.saturating_sub(viewport.start);
    let mut visible = Vec::with_capacity(viewport_len);
    let end = viewport.end;
    let scope_start = scroll.max(insertion);
    let scope_end = end.min(insertion.saturating_add(scope_count));
    let scope_lines = if let Some(scope) = scope.filter(|_| scope_start < scope_end) {
        wrapped_scope(
            scope,
            width,
            scope_start - insertion,
            scope_end - scope_start,
            theme,
        )
        .1
    } else {
        Vec::new()
    };
    let mut scope_lines = scope_lines.into_iter();
    for index in scroll..end {
        if scope.is_some() && index >= insertion && index < insertion.saturating_add(scope_count) {
            if let Some(line) = scope_lines.next() {
                visible.push(line);
            }
        } else {
            let row_index = if scope.is_some() && index >= insertion + scope_count {
                index - scope_count + 1
            } else {
                index
            };
            if let Some(row) = rows.get(row_index) {
                visible.push(row.clone());
            }
        }
    }
    visible
}

fn append_answer_rows(
    rows: &mut Vec<Line<'static>>,
    input: &InputBuffer,
    error: Option<&str>,
    custom: bool,
    width: usize,
    theme: &Theme,
) -> usize {
    rows.push(Line::styled(
        "Other / own answer:",
        if custom { theme.accent() } else { theme.dim() },
    ));
    let input_row = rows.len();
    append_input_rows(rows, input, width, theme);
    if let Some(error) = error {
        append_wrapped(rows, error, width, theme.error(), theme);
    }
    input_row
}

fn append_interview_footer(rows: &mut Vec<Line<'static>>, sending: bool, theme: &Theme) {
    let footer = if sending {
        "Sending… draft preserved"
    } else {
        "Up/Down/Tab choose · Enter send · PgUp/PgDn scroll · Esc chat"
    };
    rows.push(Line::styled(footer, theme.dim()));
}

fn interview_box(screen: Rect, theme: &Theme) -> (Rect, Block<'static>) {
    let width = screen.width.min(94);
    let area = Rect::new(
        screen.x + screen.width.saturating_sub(width) / 2,
        screen.y,
        width,
        screen.height,
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Question · choose an answer ")
        .border_style(theme.accent());
    (area, block)
}

fn append_wrapped(
    rows: &mut Vec<Line<'static>>,
    text: &str,
    width: usize,
    style: ratatui::style::Style,
    theme: &Theme,
) {
    rows.extend(
        wrap(&theme.clean(text), width)
            .into_iter()
            .map(|line| Line::styled(line, style)),
    );
}

fn option_rows(
    rows: &mut Vec<Line<'static>>,
    options: &[crate::lean::interview::Choice],
    selected: usize,
    width: usize,
    theme: &Theme,
) -> (usize, usize) {
    let mut selected_rows = (rows.len(), rows.len());
    for (index, option) in options.iter().enumerate() {
        let picked = selected == index;
        let label_rows = append_option(rows, option, index, picked, width, theme);
        if picked {
            selected_rows = label_rows;
        }
    }
    selected_rows
}

fn append_option(
    rows: &mut Vec<Line<'static>>,
    option: &crate::lean::interview::Choice,
    index: usize,
    picked: bool,
    width: usize,
    theme: &Theme,
) -> (usize, usize) {
    let label_row = append_option_label(rows, option, index, picked, width, theme);
    append_wrapped(
        rows,
        &format!("   {}", option.description),
        width,
        theme.dim(),
        theme,
    );
    label_row
}

fn append_option_label(
    rows: &mut Vec<Line<'static>>,
    option: &crate::lean::interview::Choice,
    index: usize,
    picked: bool,
    width: usize,
    theme: &Theme,
) -> (usize, usize) {
    let style = if picked {
        theme.accent().add_modifier(Modifier::REVERSED)
    } else {
        theme.text()
    };
    let first_row = rows.len();
    append_wrapped(
        rows,
        &format!(
            "{} {}. {}",
            if picked { ">" } else { " " },
            index + 1,
            option.label
        ),
        width,
        style,
        theme,
    );
    (first_row, rows.len().saturating_sub(1))
}

fn append_input_rows(
    rows: &mut Vec<Line<'static>>,
    input: &InputBuffer,
    width: usize,
    theme: &Theme,
) {
    if input.is_empty() {
        rows.push(Line::styled("Type a different answer here…", theme.dim()));
    } else {
        rows.extend(
            super::super::visual_text_lines(&input.text, width)
                .iter()
                .map(|line| Line::styled(theme.clean(line), theme.text())),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_scope_wrap_keeps_only_requested_rows() {
        let theme = Theme::new(false, true);
        let scope = "scope-word ".repeat(100_000);
        let (total, visible) = wrapped_scope(&scope, 12, 40_000, 5, &theme);

        assert!(total >= 100_000);
        assert_eq!(visible.len(), 5);
        assert!(visible.iter().all(|line| line.width() <= 12));
    }

    #[test]
    fn wrapped_scope_splits_after_multibyte_whitespace() {
        let theme = Theme::new(false, true);
        let scope = "123456789\u{3000}abc";
        let (total, visible) = wrapped_scope(scope, 12, 0, 4, &theme);

        assert_eq!(total, 2);
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].to_string(), "123456789");
        assert_eq!(visible[1].to_string(), "abc");
    }

    #[test]
    fn oversized_grapheme_is_borrowed_as_a_whole() {
        let theme = Theme::new(false, false);
        let scope = format!("x{}", "\u{301}".repeat(10_000));
        let (total, visible) = wrapped_scope(&scope, 12, 0, 1, &theme);

        assert_eq!(total, 1);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].spans[0].content.as_ref(), scope);
        assert!(matches!(
            visible[0].spans[0].content,
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn oversized_grapheme_has_bounded_ascii_fallback() {
        let theme = Theme::new(false, true);
        let scope = format!("x{}", "\u{301}".repeat(10_000));
        let (total, visible) = wrapped_scope(&scope, 12, 0, 2, &theme);

        assert_eq!(total, 1);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].to_string(), "?");
    }

    #[test]
    fn sanitized_oversized_grapheme_is_not_split_across_rows() {
        let theme = Theme::new(false, false);
        let marks = "\u{301}".repeat(2_000);
        let scope = format!("prefix \u{1b}x{marks}");
        let (total, visible) = wrapped_scope(&scope, 12, 0, 4, &theme);

        assert_eq!(total, 1);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].to_string(), format!("prefix {marks}"));
        assert_eq!(
            visible[0].spans[0].content.as_ref().graphemes(true).count(),
            7
        );
    }
}
