use super::super::diff::{DiffLine, FileDiff, ReviewerProfile};
mod header;
mod render;
use super::view::{Comment, View, clean, dim, truncate};
pub use header::{diff_footer_hint, diff_header, diff_header_with_reviewer};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use serde_json::Value;

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
        match code {
            KeyCode::Tab | KeyCode::BackTab => {
                self.on_files = !self.on_files;
            }
            // Next or previous file from the lines too.
            KeyCode::Right | KeyCode::Left if !self.on_files => {
                self.move_file(code == KeyCode::Right);
            }
            _ => self.move_selection(code),
        }
    }

    fn move_file(&mut self, next: bool) {
        let last = self.files().len().saturating_sub(1);
        self.file = if next {
            (self.file + 1).min(last)
        } else {
            self.file.saturating_sub(1)
        };
        self.line = 0;
    }

    fn move_selection(&mut self, code: crossterm::event::KeyCode) {
        use crossterm::event::KeyCode;
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
        *position = match code {
            KeyCode::Down => (*position + 1).min(last),
            KeyCode::Up => position.saturating_sub(1),
            KeyCode::PageDown => (*position + 20).min(last),
            KeyCode::PageUp => position.saturating_sub(20),
            KeyCode::Home => 0,
            KeyCode::End => last,
            _ => return,
        };
        if self.on_files {
            self.line = 0;
        }
    }
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
