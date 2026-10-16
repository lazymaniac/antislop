//! A writer's change full screen: changed files with their counts beside
//! the unified diff with old and new line numbers, the user's comments under
//! the lines they are on.

use super::super::diff::FileDiff;
use super::view::{Comment, View, clean, dim, truncate};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

#[derive(Debug)]
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
                (Some(base), None) => format!("{base}..drzewo robocze"),
                _ => String::new(),
            };
            self.truncated = value["truncated"] == true;
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
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!(" {} diff ", self.task),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "{} · plików: {} · +{added} −{removed}{}",
                        self.range,
                        self.files().len(),
                        if self.truncated {
                            " · obcięty (za duży)"
                        } else {
                            ""
                        }
                    ),
                    dim(),
                ),
            ])),
            header,
        );
        frame.render_widget(
            Paragraph::new(Span::styled(
                "↑/↓ wybór · ←/→ plik · Tab pliki/linie · c komentarz · R recenzja kodu · r odśwież · ? pomoc · Esc powrót",
                dim(),
            )),
            footer,
        );
        let files = match &self.files {
            None => {
                frame.render_widget(Paragraph::new(Span::styled("Pobieram…", dim())), body);
                return;
            }
            Some(Err(error)) => {
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        format!("✗ {error}"),
                        Style::default().fg(Color::Red),
                    )),
                    body,
                );
                return;
            }
            Some(Ok(files)) if files.is_empty() => {
                frame.render_widget(Paragraph::new(Span::styled("Brak zmian.", dim())), body);
                return;
            }
            Some(Ok(_)) => self.files(),
        };
        let lines_area = if body.width >= 90 {
            let width = (body.width / 3).min(44);
            let [list, rest] =
                Layout::horizontal([Constraint::Length(width), Constraint::Min(1)]).areas(body);
            draw_files(frame, list, files, self.file, self.on_files);
            rest
        } else {
            body
        };
        self.draw_lines(frame, lines_area, view);
    }

    fn draw_lines(&mut self, frame: &mut Frame, area: Rect, view: &View) {
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
                        " (rozwiązany)"
                    } else {
                        ""
                    }
                ),
                if comment.status == "open" {
                    Style::default().fg(Color::Cyan)
                } else {
                    dim()
                },
            )
        };
        let width = usize::from(area.width);
        let mut rows: Vec<Line> = vec![];
        let mut title = format!(" {}", file.path);
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
            rows.push(Line::styled("  plik binarny", dim()));
        }
        let mut cursor_row = 0;
        for (index, line) in file.lines.iter().enumerate() {
            let number =
                |n: Option<u32>| n.map_or_else(|| "     ".to_owned(), |n| format!("{n:>5}"));
            let (sign, style) = match line.kind.as_str() {
                "add" => ("+", Style::default().fg(Color::Green)),
                "del" => ("-", Style::default().fg(Color::Red)),
                "hunk" => ("", Style::default().fg(Color::Cyan)),
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

fn draw_files(frame: &mut Frame, area: Rect, files: &[FileDiff], selected: usize, focused: bool) {
    let block = Block::default().borders(Borders::RIGHT);
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
                "added" => ("A", Color::Green),
                "deleted" => ("D", Color::Red),
                "renamed" => ("R", Color::Yellow),
                _ => ("M", Color::Blue),
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
