use super::super::view::wrap;
use super::{Form, InputBuffer, Question, ScopeCache, ScopeCacheKey, ScopeWindow, Theme};
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
        let scope_count = self.scope_cache.row_count(
            self.scope_revision,
            self.scope_summary.as_deref(),
            inner_width,
            theme,
        );
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
            ScopeRows {
                summary: scope,
                insertion,
                count: scope_count,
                revision: self.scope_revision,
                cache: &mut self.scope_cache,
            },
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
#[cfg(test)]
fn wrapped_scope<'a>(
    text: &'a str,
    width: usize,
    start: usize,
    count: usize,
    theme: &Theme,
) -> (usize, Vec<Line<'a>>) {
    let (total, rows, _) = wrapped_scope_from(
        text,
        width,
        theme,
        ScopePass {
            start,
            count,
            checkpoint: None,
            index: None,
        },
    );
    (total, rows)
}

// Sparse checkpoints keep only resumable state, roughly one entry per 4 KiB
// of source. Replays in either direction are bounded by that spacing plus the
// requested viewport, while rendered rows stay viewport-sized.
const SCOPE_CHECKPOINT_BYTES: usize = 4096;
const MAX_CHECKPOINT_ROW_BYTES: usize = 1024;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ScopeCheckpoint {
    offset: usize,
    sanitizer: super::super::sanitize::Sanitizer,
    wrapper: ScopeWrapState,
}

#[derive(Debug, Clone, PartialEq)]
struct ScopeWrapState {
    row: String,
    columns: usize,
    last_space: Option<(usize, usize)>,
    total: usize,
}

struct ScopePass<'a> {
    start: usize,
    count: usize,
    checkpoint: Option<&'a ScopeCheckpoint>,
    index: Option<&'a mut Vec<ScopeCheckpoint>>,
}

fn wrapped_scope_from<'a>(
    text: &'a str,
    width: usize,
    theme: &Theme,
    pass: ScopePass<'_>,
) -> (usize, Vec<Line<'a>>, usize) {
    let ScopePass {
        start,
        count,
        checkpoint,
        index,
    } = pass;
    let mut index = index;
    let resume_offset = checkpoint.map_or(0, |checkpoint| checkpoint.offset);
    let width = width.max(1);
    let mut sanitizer = checkpoint
        .map(|checkpoint| checkpoint.sanitizer.clone())
        .unwrap_or_default();
    let mut chunk = String::with_capacity(1024);
    let mut output = String::with_capacity(1024);
    let mut wrapped = ScopeWrapper::from_state(width, start, count, theme, checkpoint);
    let mut graphemes = text[resume_offset..].grapheme_indices(true).peekable();
    let mut indexed_offset = resume_offset;
    let mut processed = 0;
    while let Some((offset, grapheme)) = graphemes.next() {
        if checkpoint.is_some() && count > 0 && wrapped.selected.len() >= count {
            break;
        }
        processed += 1;
        let source_offset = resume_offset + offset;
        if grapheme.len() > 1024 {
            let mut end = source_offset + grapheme.len();
            while let Some((next_offset, next)) = graphemes.peek().copied() {
                let next_source_offset = resume_offset + next_offset;
                if next_source_offset != end || next.len() <= 1024 {
                    break;
                }
                processed += 1;
                end += next.len();
                graphemes.next();
            }
            let oversized = &text[source_offset..end];
            if !chunk.is_empty() {
                sanitizer.append_chunk(&chunk, &mut output);
                wrapped.consume(&output);
                chunk.clear();
                output.clear();
                record_checkpoint(&mut index, end, &sanitizer, &wrapped, &mut indexed_offset);
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
            record_checkpoint(&mut index, end, &sanitizer, &wrapped, &mut indexed_offset);
            continue;
        }
        if chunk.len() + grapheme.len() > 1024 && !chunk.is_empty() {
            sanitizer.append_chunk(&chunk, &mut output);
            wrapped.consume(&output);
            chunk.clear();
            output.clear();
            record_checkpoint(
                &mut index,
                source_offset,
                &sanitizer,
                &wrapped,
                &mut indexed_offset,
            );
        }
        chunk.push_str(grapheme);
    }
    sanitizer.append_chunk(&chunk, &mut output);
    wrapped.consume(&output);
    record_checkpoint(
        &mut index,
        text.len(),
        &sanitizer,
        &wrapped,
        &mut indexed_offset,
    );
    let (total, rows) = wrapped.finish();
    (total, rows, processed)
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

fn record_checkpoint(
    index: &mut Option<&mut Vec<ScopeCheckpoint>>,
    offset: usize,
    sanitizer: &super::super::sanitize::Sanitizer,
    wrapper: &ScopeWrapper<'_, '_>,
    indexed_offset: &mut usize,
) {
    let Some(index) = index.as_deref_mut() else {
        return;
    };
    // Keep continuation rows small so a pathological grapheme cannot be
    // duplicated by every index entry. Sanitizer state resumes escape parsing.
    if !sanitizer.at_text_boundary()
        || wrapper.row.len() > MAX_CHECKPOINT_ROW_BYTES
        || offset.saturating_sub(*indexed_offset) < SCOPE_CHECKPOINT_BYTES
    {
        return;
    }
    index.push(ScopeCheckpoint {
        offset,
        sanitizer: sanitizer.clone(),
        wrapper: ScopeWrapState {
            row: wrapper.row.clone(),
            columns: wrapper.columns,
            last_space: wrapper.last_space,
            total: wrapper.total,
        },
    });
    *indexed_offset = offset;
}

impl ScopeCache {
    fn prepare(&mut self, revision: u64, width: usize, ascii: bool) {
        let key = ScopeCacheKey {
            revision,
            width,
            ascii,
        };
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            self.row_count = None;
            self.window = None;
            self.checkpoints.clear();
        }
    }

    fn row_count(
        &mut self,
        revision: u64,
        scope: Option<&str>,
        width: usize,
        theme: &Theme,
    ) -> usize {
        let Some(scope) = scope else {
            return 0;
        };
        self.prepare(revision, width, theme.ascii);
        if let Some(row_count) = self.row_count {
            return row_count;
        }
        #[cfg(test)]
        {
            self.scans += 1;
        }
        self.checkpoints.clear();
        self.checkpoints.push(ScopeCheckpoint {
            offset: 0,
            sanitizer: super::super::sanitize::Sanitizer::default(),
            wrapper: ScopeWrapState {
                row: String::new(),
                columns: 0,
                last_space: None,
                total: 0,
            },
        });
        let row_count = wrapped_scope_from(
            scope,
            width,
            theme,
            ScopePass {
                start: 0,
                count: 0,
                checkpoint: None,
                index: Some(&mut self.checkpoints),
            },
        )
        .0;
        self.row_count = Some(row_count);
        row_count
    }

    fn window(
        &mut self,
        revision: u64,
        scope: &str,
        start: usize,
        count: usize,
        width: usize,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        self.prepare(revision, width, theme.ascii);
        if let Some(window) = &self.window
            && window.start == start
            && window.count == count
        {
            return window
                .rows
                .iter()
                .map(|row| Line::styled(row.clone(), theme.dim()))
                .collect();
        }
        #[cfg(test)]
        {
            self.scans += 1;
        }
        if self.checkpoints.is_empty() {
            let _ = self.row_count(revision, Some(scope), width, theme);
        }
        let after = self
            .checkpoints
            .partition_point(|checkpoint| checkpoint.wrapper.total <= start);
        let checkpoint = &self.checkpoints[after.saturating_sub(1)];
        let (_partial_row_count, rows, _processed) = wrapped_scope_from(
            scope,
            width,
            theme,
            ScopePass {
                start,
                count,
                checkpoint: Some(checkpoint),
                index: None,
            },
        );
        #[cfg(test)]
        {
            self.processed_graphemes += _processed;
        }
        let rows = rows.iter().map(ToString::to_string).collect::<Vec<_>>();
        self.window = Some(ScopeWindow {
            start,
            count,
            rows: rows.clone(),
        });
        rows.into_iter()
            .map(|row| Line::styled(row, theme.dim()))
            .collect()
    }
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

    fn from_state(
        width: usize,
        start: usize,
        count: usize,
        theme: &'theme Theme,
        checkpoint: Option<&ScopeCheckpoint>,
    ) -> Self {
        let mut wrapper = Self::new(width, start, count, theme);
        if let Some(state) = checkpoint.map(|checkpoint| &checkpoint.wrapper) {
            wrapper.row.clone_from(&state.row);
            wrapper.columns = state.columns;
            wrapper.last_space = state.last_space;
            wrapper.total = state.total;
        }
        wrapper
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

struct ScopeRows<'a> {
    summary: Option<&'a str>,
    insertion: usize,
    count: usize,
    revision: u64,
    cache: &'a mut ScopeCache,
}

fn visible_rows<'a>(
    rows: Vec<Line<'static>>,
    scope_rows: ScopeRows<'a>,
    viewport: std::ops::Range<usize>,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'a>> {
    let ScopeRows {
        summary: scope,
        insertion,
        count: scope_count,
        revision: scope_revision,
        cache: scope_cache,
    } = scope_rows;
    let scroll = viewport.start;
    let viewport_len = viewport.end.saturating_sub(viewport.start);
    let mut visible = Vec::with_capacity(viewport_len);
    let end = viewport.end;
    let scope_start = scroll.max(insertion);
    let scope_end = end.min(insertion.saturating_add(scope_count));
    let scope_lines = if let Some(scope) = scope.filter(|_| scope_start < scope_end) {
        scope_cache.window(
            scope_revision,
            scope,
            scope_start - insertion,
            scope_end - scope_start,
            width,
            theme,
        )
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
    fn sparse_scope_checkpoints_bound_forward_and_backward_page_scans() {
        let theme = Theme::new(false, true);
        let scope = "word 日本語 🙂 ".repeat(30_000);
        let mut cache = ScopeCache::default();
        let total = cache.row_count(4, Some(&scope), 24, &theme);
        let source_fragments = scope.graphemes(true).count();
        let checkpoint_count = cache.checkpoints.len();
        assert!(total > 10_000);
        assert!(checkpoint_count > 1);
        assert!(cache.checkpoints.last().unwrap().wrapper.total > 10);
        assert!(checkpoint_count * 100 < source_fragments);
        assert!(cache.checkpoints.windows(2).all(|pair| {
            pair[0].offset <= pair[1].offset && pair[0].wrapper.total <= pair[1].wrapper.total
        }));

        let initial_work = cache.processed_graphemes;
        let mut requests = (0..80).map(|page| page * 10).collect::<Vec<_>>();
        requests.extend((0..80).rev().map(|page| page * 10));
        for (request, start) in requests.into_iter().enumerate() {
            let rows = cache.window(4, &scope, start, 10, 24, &theme);
            if request == 0 || request == 79 || request == 80 || request == 159 {
                let (direct_total, direct_rows) = wrapped_scope(&scope, 24, start, 10, &theme);
                assert_eq!(direct_total, total);
                assert_eq!(
                    rows.iter().map(ToString::to_string).collect::<Vec<_>>(),
                    direct_rows
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                );
            }
        }
        let indexed_work = cache.processed_graphemes - initial_work;
        assert_eq!(cache.row_count, Some(total));
        eprintln!(
            "scope fragments: source={source_fragments}, forward+backward indexed={indexed_work}, full-scan baseline={}, checkpoints={checkpoint_count}",
            source_fragments * 80 * 2
        );
        assert!(
            indexed_work < 80 * 2 * source_fragments / 10,
            "indexed page scans processed {indexed_work} of {source_fragments} fragments per full scan"
        );
        assert!(
            indexed_work < 80 * 2 * (SCOPE_CHECKPOINT_BYTES / 2 + 100),
            "checkpoint replay unexpectedly exceeded spacing bounds: {indexed_work}"
        );
    }

    #[test]
    fn sparse_scope_checkpoints_invalidate_with_content_width_and_ascii() {
        let theme = Theme::new(false, true);
        let mut cache = ScopeCache::default();
        let first = "alpha beta gamma ".repeat(2_000);
        let second = "changed scope ".repeat(2_000);
        cache.row_count(1, Some(&first), 20, &theme);
        let first_checkpoints = cache.checkpoints.len();
        assert!(first_checkpoints > 1);
        cache.window(1, &first, 400, 8, 20, &theme);
        cache.row_count(2, Some(&second), 20, &theme);
        assert!(
            cache.window.is_none(),
            "content revision invalidates the cached viewport"
        );
        assert!(cache.checkpoints.len() > 1);
        assert_ne!(cache.checkpoints.len(), 0);
        let content_count = cache.row_count.unwrap();
        let width_count = cache.row_count(2, Some(&second), 12, &theme);
        assert_ne!(content_count, width_count);
        assert!(
            cache.window.is_none(),
            "width changes invalidate the cached viewport"
        );
        let ascii_theme = Theme::new(false, false);
        cache.window(2, &second, 20, 8, 12, &theme);
        cache.row_count(2, Some(&second), 12, &ascii_theme);
        assert!(
            cache.window.is_none(),
            "ASCII mode invalidates the viewport cache"
        );
    }

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

    #[test]
    fn scope_render_reuses_same_window_and_invalidates_for_content_and_width() {
        let theme = Theme::new(false, true);
        let mut form = Form::new(
            1,
            serde_json::from_value(serde_json::json!({
                "id":"scope-cache",
                "context":"Context",
                "question":"Choose?",
                "options":[{"label":"Continue","description":"Proceed"}]
            }))
            .unwrap(),
        );
        form.set_scope_summary(Some(&"initial scope row ".repeat(80)));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 12)).unwrap();
        let draw = |terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>,
                    form: &mut Form| {
            terminal
                .draw(|frame| form.draw(frame, frame.area(), &theme))
                .unwrap();
        };

        draw(&mut terminal, &mut form);
        let first_scans = form.scope_cache.scans;
        assert!(first_scans >= 2, "initial draw counts and wraps scope");
        let cached = form.scope_cache.window.as_ref().unwrap();
        assert!(cached.rows.len() <= 10, "cached rows stay within viewport");

        draw(&mut terminal, &mut form);
        assert_eq!(form.scope_cache.scans, first_scans);

        form.set_scope_summary(Some(&"changed scope row ".repeat(80)));
        draw(&mut terminal, &mut form);
        let content_scans = form.scope_cache.scans;
        assert!(content_scans > first_scans);

        terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(32, 12)).unwrap();
        draw(&mut terminal, &mut form);
        assert!(form.scope_cache.scans > content_scans);
    }
}
