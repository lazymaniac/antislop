//! Full-screen interactive plan document viewer with line-by-line cursor
//! navigation, inline line comments, section completion underlining,
//! dependency/parallel breakdown, and a fixed bottom action bar.

use super::theme::{Theme, Tone};
use super::view::{Comment, TaskRow, View, clean, polish};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// Actions that `PlanView` can request from the parent `App`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanAction {
    None,
    Close,
    Accept,
    Reject,
    CommentLine { line: usize, text_preview: String },
    CommentWholePlan,
    ToggleResolveComment { id: u64, next_status: String },
}

/// Which button in the fixed bottom bar is currently highlighted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BottomButton {
    Accept,
    Reject,
    CommentWholePlan,
}

impl BottomButton {
    pub fn next(self) -> Self {
        match self {
            Self::Accept => Self::Reject,
            Self::Reject => Self::CommentWholePlan,
            Self::CommentWholePlan => Self::Accept,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Accept => Self::CommentWholePlan,
            Self::Reject => Self::Accept,
            Self::CommentWholePlan => Self::Reject,
        }
    }
}

/// One logical line in the rendered plan document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocLine {
    /// 1-based line number in the plan document.
    pub number: usize,
    pub text: String,
    /// Optional section name or task id associated with this line.
    pub section: Option<String>,
    /// Optional task id directly represented by this line.
    pub task_id: Option<String>,
    /// Whether this line is a Markdown heading.
    pub is_heading: bool,
    /// Whether the section/task for this line is completed (`approved`).
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanView {
    pub plan_id: String,
    pub cursor: usize,
    pub scroll: usize,
    pub button: BottomButton,
    /// Hit-boxes for the three bottom bar buttons `(Accept, Reject, CommentWholePlan)`.
    pub button_rects: [Rect; 3],
    /// Inner document area for mouse line selection.
    pub doc_area: Rect,
}

impl PlanView {
    pub fn new(plan_id: String) -> Self {
        Self {
            plan_id,
            cursor: 0,
            scroll: 0,
            button: BottomButton::Accept,
            button_rects: [Rect::default(); 3],
            doc_area: Rect::default(),
        }
    }

    /// Builds the full document lines from `View`, including section completion
    /// state, task dependency & parallel execution breakdown.
    pub fn build_lines(&self, view: &View) -> Vec<DocLine> {
        let raw_doc = if !view.plan_doc.trim().is_empty() {
            view.plan_doc.clone()
        } else {
            synthesize_document(&view.tasks)
        };

        let mut lines = Vec::new();
        let mut current_section: Option<String> = None;
        let mut current_task_id: Option<String> = None;

        for (idx, raw_line) in raw_doc.lines().enumerate() {
            let line_no = idx + 1;
            let trimmed = raw_line.trim();
            let is_heading = trimmed.starts_with('#');
            if is_heading {
                let heading_text = trimmed.trim_start_matches('#').trim();
                current_section = Some(heading_text.to_owned());
                current_task_id = extract_bracket_task_id(heading_text, &view.tasks);
            } else if let Some(tid) = extract_line_task_id(trimmed, &view.tasks) {
                current_task_id = Some(tid);
            }

            let completed = is_section_or_task_completed(
                current_section.as_deref(),
                current_task_id.as_deref(),
                &view.tasks,
            );

            lines.push(DocLine {
                number: line_no,
                text: clean(raw_line),
                section: current_section.clone(),
                task_id: current_task_id.clone(),
                is_heading,
                completed,
            });
        }

        if lines.is_empty() {
            lines.push(DocLine {
                number: 1,
                text: "# Plan Document (empty)".into(),
                section: None,
                task_id: None,
                is_heading: true,
                completed: false,
            });
        }
        lines
    }

    /// Comments attached to a 1-based line number of this plan.
    pub fn line_comments<'a>(&self, view: &'a View, line_no: usize) -> Vec<&'a Comment> {
        let target_scoped = format!("plan:{}:line:{line_no}", self.plan_id);
        let target_plain = format!("plan:line:{line_no}");
        view.comments
            .values()
            .filter(|c| c.status != "deleted")
            .filter(|c| c.target == target_scoped || c.target == target_plain)
            .collect()
    }

    /// Comments attached to the whole plan rather than a single line.
    pub fn whole_plan_comments<'a>(&self, view: &'a View) -> Vec<&'a Comment> {
        let target_scoped = format!("plan:{}", self.plan_id);
        view.comments
            .values()
            .filter(|c| c.status != "deleted")
            .filter(|c| {
                (c.target == target_scoped || c.target == "plan")
                    && (c.plan_id.is_none() || c.plan_id.as_deref() == Some(self.plan_id.as_str()))
            })
            .collect()
    }

    /// Handle keyboard navigation and actions in the full-screen plan view.
    pub fn key(&mut self, key: KeyEvent, view: &View) -> PlanAction {
        let lines = self.build_lines(view);
        let max_cursor = lines.len().saturating_sub(1);
        self.cursor = self.cursor.min(max_cursor);

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('a') => return PlanAction::Accept,
                KeyCode::Char('x') | KeyCode::Char('r') => return PlanAction::Reject,
                _ => {}
            }
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => PlanAction::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                PlanAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor = (self.cursor + 1).min(max_cursor);
                PlanAction::None
            }
            KeyCode::PageUp => {
                self.cursor = self.cursor.saturating_sub(10);
                PlanAction::None
            }
            KeyCode::PageDown => {
                self.cursor = (self.cursor + 10).min(max_cursor);
                PlanAction::None
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.cursor = 0;
                PlanAction::None
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.cursor = max_cursor;
                PlanAction::None
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                self.button = self.button.next();
                PlanAction::None
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                self.button = self.button.prev();
                PlanAction::None
            }
            KeyCode::Char('a') | KeyCode::Char('1') => PlanAction::Accept,
            KeyCode::Char('x') | KeyCode::Char('2') => PlanAction::Reject,
            KeyCode::Char('C') | KeyCode::Char('3') => PlanAction::CommentWholePlan,
            KeyCode::Char('c') => {
                let line_no = self.cursor + 1;
                let preview = lines
                    .get(self.cursor)
                    .map(|l| l.text.clone())
                    .unwrap_or_default();
                PlanAction::CommentLine {
                    line: line_no,
                    text_preview: preview,
                }
            }
            KeyCode::Char(ch @ ('r' | 'o')) => {
                let line_no = self.cursor + 1;
                if let Some(comment) = self
                    .line_comments(view, line_no)
                    .into_iter()
                    .find(|c| c.status == "open")
                    .or_else(|| self.line_comments(view, line_no).into_iter().next())
                    .or_else(|| {
                        self.whole_plan_comments(view)
                            .into_iter()
                            .find(|c| c.status == "open")
                    })
                    .or_else(|| self.whole_plan_comments(view).into_iter().next())
                {
                    let next_status = if ch == 'o' {
                        "open".to_owned()
                    } else if comment.status == "open" {
                        "resolved".to_owned()
                    } else {
                        "open".to_owned()
                    };
                    PlanAction::ToggleResolveComment {
                        id: comment.id,
                        next_status,
                    }
                } else {
                    PlanAction::None
                }
            }
            KeyCode::Enter => match self.button {
                BottomButton::Accept => PlanAction::Accept,
                BottomButton::Reject => PlanAction::Reject,
                BottomButton::CommentWholePlan => PlanAction::CommentWholePlan,
            },
            _ => PlanAction::None,
        }
    }

    /// Handle mouse click at `(col, row)`.
    pub fn mouse_click(&mut self, col: u16, row: u16, view: &View) -> PlanAction {
        let pos = ratatui::layout::Position::new(col, row);
        if self.button_rects[0].contains(pos) {
            self.button = BottomButton::Accept;
            return PlanAction::Accept;
        }
        if self.button_rects[1].contains(pos) {
            self.button = BottomButton::Reject;
            return PlanAction::Reject;
        }
        if self.button_rects[2].contains(pos) {
            self.button = BottomButton::CommentWholePlan;
            return PlanAction::CommentWholePlan;
        }
        if self.doc_area.contains(pos) {
            let lines = self.build_lines(view);
            let rel_y = (row.saturating_sub(self.doc_area.y)) as usize;
            let idx = (self.scroll + rel_y).min(lines.len().saturating_sub(1));
            self.cursor = idx;
        }
        PlanAction::None
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, view: &View, theme: &Theme) {
        frame.render_widget(Clear, area);
        let title = format!(" Plan Document ({}) ", self.plan_id);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.height < 4 || inner.width < 12 {
            return;
        }

        let open_comments = view.open_plan_comments();
        let activity = view.comment_activity();
        let has_banner = !open_comments.is_empty() || activity.is_some();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(if has_banner {
                vec![
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(3),
                ]
            } else {
                vec![
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(3),
                ]
            })
            .split(inner);

        // Top status / comment resolution banner
        if has_banner {
            let status = activity.unwrap_or_else(|| {
                open_comments.first().map_or_else(String::new, |comment| {
                    format!(
                        "Comment #{}: {}",
                        comment.id,
                        view.comment_state(comment.id)
                    )
                })
            });
            let banner = vec![
                Line::styled(
                    format!(" {}", clean(&status)),
                    theme.accent().add_modifier(Modifier::BOLD),
                ),
                Line::from(vec![
                    Span::styled(
                        format!(
                            " ⚠ {} unresolved comment(s) on this plan ",
                            open_comments.len()
                        ),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        "— r resolve · o retry/reopen · wait for revision before accepting",
                        theme.dim(),
                    ),
                ]),
                Line::styled(
                    " Navigate: ↑/↓/j/k · Comment line: c · Resolve: r · Retry: o · Close preview: Esc",
                    theme.dim(),
                ),
            ];
            frame.render_widget(Paragraph::new(banner), chunks[0]);
        } else {
            let completed_count = view.tasks.iter().filter(|t| t.status == "approved").count();
            let header_line = Line::from(vec![
                Span::styled(
                    format!(
                        " Tasks: {}/{} completed ",
                        completed_count,
                        view.tasks.len()
                    ),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "· Navigate: ↑/↓/j/k · Comment line: c · Comment plan: C · Resolve: r · Close: Esc",
                    theme.dim(),
                ),
            ]);
            frame.render_widget(Paragraph::new(header_line), chunks[0]);
        }

        // Document body
        let doc_rect = chunks[1];
        self.doc_area = doc_rect;
        let doc_lines = self.build_lines(view);
        let max_cursor = doc_lines.len().saturating_sub(1);
        self.cursor = self.cursor.min(max_cursor);

        let visible_h = doc_rect.height as usize;
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if visible_h > 0 && self.cursor >= self.scroll + visible_h {
            self.scroll = self.cursor + 1 - visible_h;
        }

        let mut rendered: Vec<Line<'static>> = Vec::new();
        // Whole-plan comments at top when scrolled to top
        if self.scroll == 0 {
            for c in self.whole_plan_comments(view) {
                let badge = view.comment_state(c.id);
                let style = if c.status == "open" {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    theme.dim()
                };
                let mut text = format!("     ✎ #{} [{badge}]: {}", c.id, clean(&c.text));
                if let Some(error) = view.comment_error(c.id) {
                    text.push_str(&format!("\n       Error: {}", clean(error)));
                }
                for line in super::view::wrap(&text, usize::from(doc_rect.width)) {
                    if rendered.len() >= visible_h {
                        break;
                    }
                    rendered.push(Line::styled(line, style));
                }
            }
        }

        for (idx, dline) in doc_lines.iter().enumerate().skip(self.scroll) {
            if rendered.len() >= visible_h {
                break;
            }
            let selected = idx == self.cursor;
            let cursor_marker = if selected { "▶" } else { " " };
            let line_num = format!("{cursor_marker}{:>3} │ ", dline.number);

            let mut text_style = if dline.is_heading {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            if dline.completed {
                text_style = text_style
                    .fg(Color::Green)
                    .add_modifier(Modifier::UNDERLINED);
            }
            if selected {
                text_style = text_style.add_modifier(Modifier::REVERSED);
            }

            let mut spans = vec![
                Span::styled(
                    line_num,
                    if selected {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        theme.dim()
                    },
                ),
                Span::styled(dline.text.clone(), text_style),
            ];
            if dline.completed && dline.is_heading {
                spans.push(Span::styled(
                    " ✓ COMPLETED",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                ));
            }
            rendered.push(Line::from(spans));

            for c in self.line_comments(view, dline.number) {
                if rendered.len() >= visible_h {
                    break;
                }
                let status_badge = view.comment_state(c.id);
                let c_style = if c.status == "open" {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    theme.dim()
                };
                let mut text = format!("     │   ✎ #{} [{status_badge}]: {}", c.id, clean(&c.text));
                if let Some(error) = view.comment_error(c.id) {
                    text.push_str(&format!("\n       Error: {}", clean(error)));
                }
                for line in super::view::wrap(&text, usize::from(doc_rect.width)) {
                    if rendered.len() >= visible_h {
                        break;
                    }
                    rendered.push(Line::styled(line, c_style));
                }
            }
        }
        frame.render_widget(Paragraph::new(rendered), doc_rect);

        // Fixed bottom bar with 3 buttons: Accept / Reject / Comment whole plan
        let bar_area = chunks[2];
        let btn_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(34),
                Constraint::Percentage(33),
                Constraint::Percentage(33),
            ])
            .split(bar_area);
        self.button_rects = [btn_cols[0], btn_cols[1], btn_cols[2]];

        let can_accept = open_comments.is_empty() && view.plan_proposed();
        let buttons = [
            (
                BottomButton::Accept,
                if can_accept {
                    " [ Accept (Ctrl+A / a) ] "
                } else if !open_comments.is_empty() {
                    " [ Accept (resolve comments first) ] "
                } else {
                    " [ Accept (Ctrl+A / a) ] "
                },
                if can_accept {
                    Tone::Done
                } else {
                    Tone::Waiting
                },
                btn_cols[0],
            ),
            (
                BottomButton::Reject,
                " [ Reject (Ctrl+X / x) ] ",
                Tone::Failed,
                btn_cols[1],
            ),
            (
                BottomButton::CommentWholePlan,
                " [ Comment whole plan (C) ] ",
                Tone::Active,
                btn_cols[2],
            ),
        ];

        for (b_kind, label, tone, rect) in buttons {
            let active = self.button == b_kind;
            let mut style = match tone {
                Tone::Done => theme.status("approved"),
                Tone::Failed => theme.error(),
                Tone::Waiting => theme.warning(),
                Tone::Active => theme.accent(),
                Tone::Queued => theme.dim(),
            };
            if active {
                style = style.add_modifier(Modifier::BOLD | Modifier::REVERSED);
            }
            let b_block = Block::default()
                .borders(Borders::ALL)
                .border_style(if active {
                    Style::default().fg(Color::Cyan)
                } else {
                    theme.dim()
                });
            let b_inner = b_block.inner(rect);
            frame.render_widget(b_block, rect);
            if b_inner.height > 0 && b_inner.width > 0 {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        theme.fit(label, b_inner.width as usize),
                        style,
                    ))),
                    b_inner,
                );
            }
        }
    }
}

fn synthesize_document(tasks: &[TaskRow]) -> String {
    if tasks.is_empty() {
        return "# Execution Plan\n\nNo tasks proposed yet.".into();
    }
    let mut out = String::from("# Execution Plan\n\n## Plan Sections & Task Breakdown\n\n");
    for (idx, task) in tasks.iter().enumerate() {
        let section = task
            .section
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&task.title);
        out.push_str(&format!(
            "### {}. {} [{}] ({})\n",
            idx + 1,
            section,
            task.id,
            polish(&task.status)
        ));
        out.push_str(&format!("- **Task**: `{}` — {}\n", task.id, task.title));
        out.push_str(&format!(
            "- **Kind**: `{}` ({})\n",
            task.kind,
            if task.writes { "writes" } else { "read-only" }
        ));
        out.push_str(&format!("- **Goal**: {}\n", task.goal));
        let deps = if task.depends_on.is_empty() {
            "none (can start immediately)".to_owned()
        } else {
            task.depends_on.join(", ")
        };
        out.push_str(&format!("- **Depends on**: {deps}\n"));
        let par = if task.parallel_with.is_empty() {
            "sequential".to_owned()
        } else {
            task.parallel_with.join(", ")
        };
        out.push_str(&format!("- **Parallel with**: {par}\n\n"));
    }
    out.trim_end().to_owned()
}

fn extract_bracket_task_id(heading: &str, tasks: &[TaskRow]) -> Option<String> {
    for task in tasks {
        if heading.contains(&format!("[{}]", task.id)) {
            return Some(task.id.clone());
        }
    }
    tasks
        .iter()
        .find(|t| {
            t.section
                .as_deref()
                .is_some_and(|s| !s.is_empty() && heading.eq_ignore_ascii_case(s.trim()))
                || heading.eq_ignore_ascii_case(t.title.trim())
        })
        .map(|t| t.id.clone())
}

fn extract_line_task_id(line: &str, tasks: &[TaskRow]) -> Option<String> {
    for task in tasks {
        if line.contains(&format!("`{}`", task.id)) || line.contains(&format!("[{}]", task.id)) {
            return Some(task.id.clone());
        }
    }
    None
}

fn is_section_or_task_completed(
    section: Option<&str>,
    task_id: Option<&str>,
    tasks: &[TaskRow],
) -> bool {
    if let Some(tid) = task_id
        && let Some(t) = tasks.iter().find(|t| t.id == tid)
    {
        return t.status == "approved";
    }
    if let Some(sec) = section {
        let matching: Vec<&TaskRow> = tasks
            .iter()
            .filter(|t| {
                t.section
                    .as_deref()
                    .is_some_and(|s| !s.is_empty() && sec.contains(s.trim()))
                    || sec.contains(&t.title)
            })
            .collect();
        if !matching.is_empty() {
            return matching.iter().all(|t| t.status == "approved");
        }
    }
    false
}
