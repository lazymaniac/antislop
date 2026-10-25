use super::super::diff::{FileDiff, ReviewerProfile};
use super::view::{Comment, View, clean, dim, truncate};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, PartialEq)]
pub struct DiffView {
    /// The task's id in its plan (`t2`).
    pub task: String,
    files: Option<Result<Vec<FileDiff>, String>>,
    range: String,
    truncated: bool,
    file: usize,
    /// The selected line of the selected file.
    line: usize,
    /// Whether arrows move through files rather than lines.
    on_files: bool,
    scroll: usize,
    pub reviewer: Option<ReviewerProfile>,
}

impl DiffView {
    pub fn new(task: String) -> Self {
        DiffView {
            task,
            files: None,
            range: String::new(),
            truncated: false,
            file: 0,
            line: 0,
            on_files: false,
            scroll: 0,
            reviewer: None,
        }
    }

    pub fn load(&mut self, reply: Result<Value, String>) {
        self.files = Some(reply.and_then(|value| {
            let short = |key: &str| {
                value[key]
                    .as_str()
                    .map(|rev| rev[..rev.len().min(10)].to_owned())
            };
            self.range = match (short("base"), short("head")) {
                (Some(base), Some(head)) => format!("{base}..{head}"),
                (Some(base), None) => format!("{base}..working tree"),
                _ => String::new(),
            };
            self.truncated = value["truncated"] == true;
            if let Ok(rev) = serde_json::from_value::<ReviewerProfile>(value["reviewer"].clone()) {
                self.reviewer = Some(rev);
            }
            serde_json::from_value(value["files"].clone()).map_err(|error| error.to_string())
        }));
        let count = self.files().len();
        self.file = self.file.min(count.saturating_sub(1));
        self.line = self.line.min(self.lines_in_file().saturating_sub(1));
    }

    fn files(&self) -> &[FileDiff] {
        match &self.files {
            Some(Ok(files)) => files,
            _ => &[],
        }
    }

    fn lines_in_file(&self) -> usize {
        self.files()
            .get(self.file)
            .map_or(0, |file| file.lines.len())
    }

    /// What `c` comments on: the selected line, or the file.
    pub fn target(&self) -> Option<String> {
        let file = self.files().get(self.file)?;
        let target = format!("diff:{}:{}", self.task, file.path);
        if self.on_files {
            return Some(target);
        }
        let line = file.lines.get(self.line)?;
        Some(match (line.kind.as_str(), line.old, line.new) {
            ("del", Some(old), _) => format!("{target}:old{old}"),
            (_, _, Some(new)) => format!("{target}:{new}"),
            _ => target,
        })
    }

    /// Vim-style Ctrl+D / Ctrl+U: half of a page down or up.
    pub fn half_page(&mut self, down: bool) {
        let count = if self.on_files {
            self.files().len()
        } else {
            self.lines_in_file()
        };
        let last = count.saturating_sub(1);
        let position = if self.on_files {
            &mut self.file
        } else {
            &mut self.line
        };
        *position = if down {
            (*position + 10).min(last)
        } else {
            position.saturating_sub(10)
        };
        if self.on_files {
            self.line = 0;
        }
    }

    pub fn key(&mut self, code: crossterm::event::KeyCode) {
        use crossterm::event::KeyCode;
        let files = self.files().len();
        let count = if self.on_files {
            files
        } else {
            self.lines_in_file()
        };
        let last = count.saturating_sub(1);
        let mut position = if self.on_files { self.file } else { self.line };
        match code {
            KeyCode::Tab | KeyCode::BackTab => {
                self.on_files = !self.on_files;
                return;
            }
            // Next or previous file from the lines too.
            KeyCode::Right | KeyCode::Left if !self.on_files => {
                self.file = if code == KeyCode::Right {
                    (self.file + 1).min(files.saturating_sub(1))
                } else {
                    self.file.saturating_sub(1)
                };
                self.line = 0;
                return;
            }
            KeyCode::Down => position = (position + 1).min(last),
            KeyCode::Up => position = position.saturating_sub(1),
            KeyCode::PageDown => position = (position + 20).min(last),
            KeyCode::PageUp => position = position.saturating_sub(20),
            KeyCode::Home => position = 0,
            KeyCode::End => position = last,
            _ => return,
        }
        if self.on_files {
            self.file = position;
            self.line = 0;
        } else {
            self.line = position;
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, view: &View) {
        let area = frame.area();
        if area.width == 0 || area.height == 0 {
            return;
        }
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(area);
        let (added, removed) = self
            .files()
            .iter()
            .fold((0, 0), |(a, r), file| (a + file.added, r + file.removed));
        frame.render_widget(
            Paragraph::new(diff_header_with_reviewer(
                &self.task,
                &self.range,
                self.files().len(),
                added,
                removed,
                self.truncated,
                self.reviewer.as_ref(),
                usize::from(header.width),
            )),
            header,
        );
        frame.render_widget(
            Paragraph::new(Span::styled(
                diff_footer_hint(usize::from(footer.width)),
                dim(),
            )),
            footer,
        );
        let files = match &self.files {
            None => {
                frame.render_widget(
                    Paragraph::new(Span::styled("Loading task diff from daemon…", dim())),
                    body,
                );
                return;
            }
            Some(Err(error)) => {
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        format!("✗ Error loading changes: {error}"),
                        Style::default().fg(super::theme::ayu::RED),
                    )),
                    body,
                );
                return;
            }
            Some(Ok(files)) if files.is_empty() => {
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        "No changes in this task. The task did not modify any files.",
                        dim(),
                    )),
                    body,
                );
                return;
            }
            Some(Ok(_)) => self.files(),
        };
        if body.width >= 90 {
            let width = (body.width / 3).min(44);
            let [list, rest] =
                Layout::horizontal([Constraint::Length(width), Constraint::Min(1)]).areas(body);
            draw_files(frame, list, files, self.file, self.on_files, false);
            self.draw_lines(frame, rest, view, false, files.len());
        } else if self.on_files {
            draw_files(frame, body, files, self.file, true, true);
        } else {
            self.draw_lines(frame, body, view, true, files.len());
        }
    }

    fn draw_lines(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &View,
        compact: bool,
        file_count: usize,
    ) {
        let Some(file) = self.files().get(self.file) else {
            return;
        };
        let target = format!("diff:{}:{}", self.task, file.path);
        let comments: Vec<&Comment> = view
            .comments
            .values()
            .filter(|comment| comment.status != "deleted")
            .filter(|comment| {
                comment.target == target || comment.target.starts_with(&format!("{target}:"))
            })
            .collect();
        let comment_line = |comment: &Comment| {
            Line::styled(
                format!(
                    "              ✎ #{} {}{}",
                    comment.id,
                    clean(&comment.text),
                    if comment.status == "resolved" {
                        " (resolved)"
                    } else {
                        ""
                    }
                ),
                if comment.status == "open" {
                    Style::default().fg(super::theme::ayu::BLUE)
                } else {
                    dim()
                },
            )
        };
        let width = usize::from(area.width);
        let mut rows: Vec<Line> = vec![];
        let mut title = if compact && file_count > 1 {
            format!(
                " [{}/{}] {} [Tab: files]",
                self.file + 1,
                file_count,
                file.path
            )
        } else {
            format!(" {}", file.path)
        };
        if let Some(old) = &file.old_path {
            title = format!(" {old} → {}", file.path);
        }
        rows.push(Line::styled(
            truncate(&title, width),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        rows.extend(
            comments
                .iter()
                .filter(|comment| comment.target == target)
                .map(|comment| comment_line(comment)),
        );
        if file.binary {
            rows.push(Line::styled("  binary file", dim()));
        }
        let mut cursor_row = 0;
        for (index, line) in file.lines.iter().enumerate() {
            let number =
                |n: Option<u32>| n.map_or_else(|| "     ".to_owned(), |n| format!("{n:>5}"));
            let (sign, style) = match line.kind.as_str() {
                "add" => ("+", Style::default().fg(super::theme::ayu::GREEN)),
                "del" => ("-", Style::default().fg(super::theme::ayu::RED)),
                "hunk" => ("", Style::default().fg(super::theme::ayu::BLUE)),
                _ => (" ", Style::default()),
            };
            let text = if line.kind == "hunk" {
                format!("            {}", line.text)
            } else {
                format!(
                    "{} {} {sign}│{}",
                    number(line.old),
                    number(line.new),
                    line.text.replace('\t', "    ")
                )
            };
            let selected = index == self.line && !self.on_files;
            if selected {
                cursor_row = rows.len();
            }
            rows.push(Line::styled(
                truncate(&text, width),
                if selected {
                    style.add_modifier(Modifier::REVERSED)
                } else {
                    style
                },
            ));
            let key = match (line.kind.as_str(), line.old, line.new) {
                ("del", Some(old), _) => Some(format!("{target}:old{old}")),
                (_, _, Some(new)) if line.kind != "hunk" => Some(format!("{target}:{new}")),
                _ => None,
            };
            if let Some(key) = key {
                rows.extend(
                    comments
                        .iter()
                        .filter(|comment| comment.target == key)
                        .map(|comment| comment_line(comment)),
                );
            }
        }
        let height = usize::from(area.height);
        if cursor_row < self.scroll {
            self.scroll = cursor_row.saturating_sub(1);
        } else if cursor_row >= self.scroll + height {
            self.scroll = cursor_row + 1 - height;
        }
        self.scroll = self.scroll.min(rows.len().saturating_sub(height));
        let end = (self.scroll + height).min(rows.len());
        frame.render_widget(Paragraph::new(rows[self.scroll..end].to_vec()), area);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn diff_header_with_reviewer(
    task: &str,
    range: &str,
    file_count: usize,
    added: usize,
    removed: usize,
    truncated: bool,
    reviewer: Option<&ReviewerProfile>,
    width: usize,
) -> Line<'static> {
    let trunc_str = if truncated { " · truncated" } else { "" };
    let task_badge = format!(" {task} diff ");
    let reviewer_str = match reviewer {
        Some(r) => match &r.model {
            Some(m) => format!(" · reviewer: {} ({m})", r.provider),
            None => format!(" · reviewer: {}", r.provider),
        },
        None => String::new(),
    };
    if width >= 85 && !range.is_empty() {
        Line::from(vec![
            Span::styled(task_badge, Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                format!(
                    "{range} · files: {file_count} · +{added} −{removed}{trunc_str}{reviewer_str}"
                ),
                dim(),
            ),
        ])
    } else if width >= 50 && !range.is_empty() {
        Line::from(vec![
            Span::styled(task_badge, Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("{range} · +{added} −{removed}{trunc_str}{reviewer_str}"),
                dim(),
            ),
        ])
    } else if width >= 50 {
        Line::from(vec![
            Span::styled(task_badge, Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("+{added} −{removed} · {file_count} file(s){trunc_str}"),
                dim(),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(task_badge, Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(format!("+{added} −{removed}"), dim()),
        ])
    }
}

pub fn diff_header(
    task: &str,
    range: &str,
    file_count: usize,
    added: usize,
    removed: usize,
    truncated: bool,
    width: usize,
) -> Line<'static> {
    diff_header_with_reviewer(
        task, range, file_count, added, removed, truncated, None, width,
    )
}

pub fn diff_footer_hint(width: usize) -> &'static str {
    if width >= 95 {
        "↑/↓ select · ←/→ file · Tab files/lines · c comment · R code review · r refresh · ? help · Esc back"
    } else if width >= 65 {
        "↑/↓ select · ←/→ file · Tab switch · c com. · R review · r refresh · Esc back"
    } else if width >= 45 {
        "↑/↓ · ←/→ · Tab switch · c com · Esc back"
    } else {
        "Tab · c · Esc"
    }
}

fn draw_files(
    frame: &mut Frame,
    area: Rect,
    files: &[FileDiff],
    selected: usize,
    focused: bool,
    compact: bool,
) {
    let block = if compact {
        Block::default()
            .borders(Borders::ALL)
            .border_style(if focused {
                Style::default().fg(super::theme::ayu::BLUE)
            } else {
                dim()
            })
            .title(" Changed files [Tab: switch to diff] ")
    } else {
        Block::default().borders(Borders::RIGHT)
    };
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = usize::from(inner.width);
    let height = usize::from(inner.height);
    let start = selected.saturating_sub(height.saturating_sub(1));
    let rows: Vec<Line> = files
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(index, file)| {
            let (mark, color) = match file.status.as_str() {
                "added" => ("A", super::theme::ayu::GREEN),
                "deleted" => ("D", super::theme::ayu::RED),
                "renamed" => ("R", super::theme::ayu::YELLOW),
                _ => ("M", super::theme::ayu::BLUE),
            };
            let counts = format!(" +{} −{}", file.added, file.removed);
            let room = width.saturating_sub(counts.width() + 2);
            let mut style = Style::default();
            if index == selected {
                style = if focused {
                    style.add_modifier(Modifier::REVERSED)
                } else {
                    style.add_modifier(Modifier::BOLD)
                };
            }
            Line::from(vec![
                Span::styled(format!("{mark} "), Style::default().fg(color)),
                Span::styled(truncate(&file.path, room), style),
                Span::styled(counts, dim()),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(rows), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn diff_header_and_footer_hints_prioritized_by_width() {
        // Wide: 100 columns
        let header = diff_header("t1", "base..head", 3, 10, 5, false, 100);
        let header_text: String = header.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(header_text.contains("files: 3"));
        assert!(header_text.contains("+10 −5"));

        // Narrow: 40 columns
        let header_narrow = diff_header("t1", "base..head", 3, 10, 5, false, 40);
        let narrow_text: String = header_narrow
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(narrow_text.contains("+10 −5"));

        // Footer hints
        assert!(diff_footer_hint(100).contains("code review"));
        assert!(!diff_footer_hint(40).contains("code review"));
        assert!(diff_footer_hint(40).contains("Esc"));
    }

    #[test]
    fn diff_responsive_split_and_compact_rendering() {
        let view = View::default();
        let mut diff = DiffView::new("t2".into());
        diff.files = Some(Ok(vec![FileDiff {
            path: "src/main.rs".into(),
            old_path: None,
            status: "modified".into(),
            binary: false,
            added: 5,
            removed: 2,
            lines: vec![],
        }]));

        // Wide terminal (120x30): side-by-side files & diff
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                diff.draw(frame, &view);
            })
            .unwrap();

        // Narrow terminal (80x24): compact mode
        diff.on_files = true;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                diff.draw(frame, &view);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rendered: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(rendered.contains("Changed files"));

        // Tiny terminal (10x4)
        let backend = TestBackend::new(10, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                diff.draw(frame, &view);
            })
            .unwrap();
    }
}
