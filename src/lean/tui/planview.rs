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
    CommentLine {
        line: usize,
        text_preview: String,
    },
    CommentWholePlan,
    /// The user finished commenting: the agent may work on the drafts.
    SubmitReview,
    ToggleResolveComment {
        id: u64,
        next_status: String,
    },
}

/// Which button in the fixed bottom bar is currently highlighted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BottomButton {
    Accept,
    Reject,
    CommentWholePlan,
    Review,
}

impl BottomButton {
    pub fn next(self) -> Self {
        match self {
            Self::Accept => Self::Reject,
            Self::Reject => Self::CommentWholePlan,
            Self::CommentWholePlan => Self::Review,
            Self::Review => Self::Accept,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Accept => Self::Review,
            Self::Reject => Self::Accept,
            Self::CommentWholePlan => Self::Reject,
            Self::Review => Self::CommentWholePlan,
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
    /// How the tasks of this line's section are doing.
    pub state: SectionState,
}

/// The combined state of the tasks assigned to a plan section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionState {
    /// No task is assigned, or none has started.
    Open,
    Working,
    Problem,
    /// Every assigned task is approved.
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanView {
    pub plan_id: String,
    pub cursor: usize,
    pub scroll: usize,
    pub button: BottomButton,
    /// Hit-boxes for the three bottom bar buttons `(Accept, Reject, CommentWholePlan)`.
    pub button_rects: [Rect; 4],
    /// Inner document area for mouse line selection.
    pub doc_area: Rect,
    /// The document line each screen row belongs to, from the last draw.
    pub row_docs: Vec<Option<usize>>,
    /// Accept was activated once from its button; a second activation accepts.
    pub confirm_accept: bool,
    /// Advances the in-progress spinner; the app sets it before drawing.
    pub tick: u64,
}

/// Columns taken by the line number and its separator.
const GUTTER: usize = 7;

struct VisualRow {
    /// The document line the row shows or comments on.
    doc: Option<usize>,
    line: Line<'static>,
}

impl PlanView {
    pub fn new(plan_id: String) -> Self {
        Self {
            plan_id,
            cursor: 0,
            scroll: 0,
            button: BottomButton::CommentWholePlan,
            button_rects: [Rect::default(); 4],
            doc_area: Rect::default(),
            row_docs: Vec::new(),
            confirm_accept: false,
            tick: 0,
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
        let mut current_level = 0;
        let mut current_tasks: Vec<String> = Vec::new();

        for (idx, raw_line) in raw_doc.lines().enumerate() {
            let line_no = idx + 1;
            let trimmed = raw_line.trim();
            let is_heading = trimmed.starts_with('#');
            if is_heading {
                let level = trimmed.chars().take_while(|ch| *ch == '#').count();
                let heading_text = trimmed.trim_start_matches('#').trim();
                let matched = section_tasks(heading_text, &view.tasks);
                // A subheading without tasks of its own stays in its section.
                if !matched.is_empty() || level <= current_level || current_section.is_none() {
                    current_tasks = matched;
                    current_section = Some(heading_text.to_owned());
                    current_level = level;
                }
            }

            lines.push(DocLine {
                number: line_no,
                text: clean(raw_line),
                section: current_section.clone(),
                task_id: current_tasks.first().cloned(),
                is_heading,
                state: section_state(&current_tasks, &view.tasks),
            });
        }

        if lines.is_empty() {
            lines.push(DocLine {
                number: 1,
                text: "# Plan Document (empty)".into(),
                section: None,
                task_id: None,
                is_heading: true,
                state: SectionState::Open,
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

        // Only a proposed plan can be accepted or rejected; otherwise the
        // view is a document to read and comment on.
        let decidable = view.plan_proposed();
        let drafts = view.draft_plan_comments().len();
        let confirming = std::mem::take(&mut self.confirm_accept);
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('a') if decidable => return PlanAction::Accept,
                KeyCode::Char('x') | KeyCode::Char('r') if decidable => {
                    return PlanAction::Reject;
                }
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
            KeyCode::Char('a') | KeyCode::Char('1') if decidable => PlanAction::Accept,
            KeyCode::Char('x') | KeyCode::Char('2') if decidable => PlanAction::Reject,
            KeyCode::Char('C') | KeyCode::Char('3') => PlanAction::CommentWholePlan,
            KeyCode::Char('R') | KeyCode::Char('4') => {
                self.activate(BottomButton::Review, decidable, false, drafts)
            }
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
            // Takes back a comment of this line, a draft usually.
            KeyCode::Delete | KeyCode::Char('D') => {
                match self
                    .line_comments(view, self.cursor + 1)
                    .into_iter()
                    .rev()
                    .find(|c| c.status == "draft")
                    .or_else(|| {
                        self.line_comments(view, self.cursor + 1)
                            .into_iter()
                            .next_back()
                    }) {
                    Some(comment) => PlanAction::ToggleResolveComment {
                        id: comment.id,
                        next_status: "deleted".into(),
                    },
                    None => PlanAction::None,
                }
            }
            KeyCode::Char(ch @ ('r' | 'o')) => {
                let line_no = self.cursor + 1;
                if let Some(comment) = self
                    .line_comments(view, line_no)
                    .into_iter()
                    .find(|c| c.status == "open")
                    .or_else(|| self.line_comments(view, line_no).into_iter().next())
                    // Whole-plan comments are listed at the top of the document.
                    .or_else(|| {
                        (self.cursor == 0)
                            .then(|| {
                                self.whole_plan_comments(view)
                                    .into_iter()
                                    .find(|c| c.status == "open")
                                    .or_else(|| self.whole_plan_comments(view).into_iter().next())
                            })
                            .flatten()
                    })
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
            KeyCode::Enter => self.activate(self.button, decidable, confirming, drafts),
            _ => PlanAction::None,
        }
    }

    /// A button was pressed: Accept asks for a second press first.
    fn activate(
        &mut self,
        button: BottomButton,
        decidable: bool,
        confirming: bool,
        drafts: usize,
    ) -> PlanAction {
        match button {
            BottomButton::Review if drafts > 0 => PlanAction::SubmitReview,
            BottomButton::Review => PlanAction::None,
            BottomButton::Accept if !decidable => PlanAction::None,
            BottomButton::Accept if confirming => PlanAction::Accept,
            BottomButton::Accept => {
                self.confirm_accept = true;
                PlanAction::None
            }
            BottomButton::Reject if decidable => PlanAction::Reject,
            BottomButton::Reject => PlanAction::None,
            BottomButton::CommentWholePlan => PlanAction::CommentWholePlan,
        }
    }

    /// Handle mouse click at `(col, row)`.
    pub fn mouse_click(&mut self, col: u16, row: u16, view: &View) -> PlanAction {
        let pos = ratatui::layout::Position::new(col, row);
        let decidable = view.plan_proposed();
        let drafts = view.draft_plan_comments().len();
        let confirming = std::mem::take(&mut self.confirm_accept);
        for (rect, button) in self.button_rects.into_iter().zip([
            BottomButton::Accept,
            BottomButton::Reject,
            BottomButton::CommentWholePlan,
            BottomButton::Review,
        ]) {
            if rect.contains(pos) {
                self.button = button;
                return self.activate(button, decidable, confirming, drafts);
            }
        }
        if self.doc_area.contains(pos) {
            let rel_y = usize::from(row.saturating_sub(self.doc_area.y));
            if let Some(Some(idx)) = self.row_docs.get(self.scroll + rel_y) {
                self.cursor = (*idx).min(self.build_lines(view).len().saturating_sub(1));
            }
        }
        PlanAction::None
    }

    /// The document as screen rows: long lines are word-wrapped, and the
    /// comments follow the line they are on. Returns the rows and the
    /// first and last row of the cursor's line.
    fn layout_rows(
        &self,
        view: &View,
        doc_lines: &[DocLine],
        width: u16,
        theme: &Theme,
    ) -> (Vec<VisualRow>, (usize, usize)) {
        let width = usize::from(width);
        let text_width = width.saturating_sub(GUTTER).max(1);
        let comment_style = |c: &Comment| {
            if c.status == "open" {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                theme.dim()
            }
        };
        let comment_rows = |c: &Comment, indent: &str| {
            let badge = view.comment_state(c.id);
            let mut text = format!("{indent}✎ #{} [{badge}]: {}", c.id, clean(&c.text));
            if let Some(error) = view.comment_error(c.id) {
                text.push_str(&format!("\n{indent}  Error: {}", clean(error)));
            }
            let style = comment_style(c);
            super::view::wrap(&text, width.max(1))
                .into_iter()
                .map(move |line| (line, style))
                .collect::<Vec<_>>()
        };

        let mut rows = Vec::new();
        for c in self.whole_plan_comments(view) {
            for (line, style) in comment_rows(c, "     ") {
                rows.push(VisualRow {
                    doc: None,
                    line: Line::styled(line, style),
                });
            }
        }
        let mut cursor_rows = (0, 0);
        for (idx, dline) in doc_lines.iter().enumerate() {
            let selected = idx == self.cursor;
            let start = rows.len();
            let mut text_style = if dline.is_heading {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            match dline.state {
                SectionState::Done => {
                    text_style = text_style
                        .fg(Color::Green)
                        .add_modifier(Modifier::UNDERLINED);
                }
                SectionState::Problem => text_style = text_style.fg(Color::Red),
                SectionState::Working => text_style = text_style.fg(Color::Cyan),
                SectionState::Open => {}
            }
            if selected {
                text_style = text_style.add_modifier(Modifier::REVERSED);
            }
            let gutter_style = if selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                theme.dim()
            };
            let pieces = super::view::wrap(&dline.text, text_width);
            let count = pieces.len();
            for (n, piece) in pieces.into_iter().enumerate() {
                let gutter = if n == 0 {
                    let marker = if selected { "▶" } else { " " };
                    format!("{marker}{:>3} │ ", dline.number)
                } else {
                    "     │ ".to_owned()
                };
                let mut spans = vec![
                    Span::styled(gutter, gutter_style),
                    Span::styled(piece, text_style),
                ];
                if n + 1 == count && dline.is_heading {
                    let badge = match dline.state {
                        SectionState::Done => {
                            Some((format!(" {} COMPLETED", theme.glyphs().done), Color::Green))
                        }
                        SectionState::Working => Some((
                            format!(" {} IN PROGRESS", theme.spinner(self.tick)),
                            Color::Cyan,
                        )),
                        SectionState::Problem => Some((
                            format!(" {} NEEDS ATTENTION", theme.glyphs().problem),
                            Color::Red,
                        )),
                        SectionState::Open => None,
                    };
                    if let Some((text, color)) = badge {
                        spans.push(Span::styled(
                            text,
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        ));
                    }
                }
                rows.push(VisualRow {
                    doc: Some(idx),
                    line: Line::from(spans),
                });
            }
            if selected {
                cursor_rows = (start, rows.len() - 1);
            }
            for c in self.line_comments(view, dline.number) {
                for (line, style) in comment_rows(c, "     │   ") {
                    rows.push(VisualRow {
                        doc: Some(idx),
                        line: Line::styled(line, style),
                    });
                }
            }
        }
        (rows, cursor_rows)
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
            let drafts = view.draft_plan_comments().len();
            let sent = open_comments.len() - drafts;
            let mut notes = Vec::new();
            if drafts > 0 {
                notes.push(Span::styled(
                    format!(" ✎ {drafts} draft comment(s) "),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
                notes.push(Span::styled("— R sends them for review · ", theme.dim()));
            }
            if sent > 0 {
                notes.push(Span::styled(
                    format!(" ⚠ {sent} unresolved comment(s) on this plan "),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
                notes.push(Span::styled("— r resolve · o retry/reopen", theme.dim()));
            }
            let banner = vec![
                Line::styled(
                    format!(" {}", clean(&status)),
                    theme.accent().add_modifier(Modifier::BOLD),
                ),
                Line::from(notes),
                Line::styled(
                    " ↑/↓ move · c comment line · R send drafts for review · D delete draft · Esc close",
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

        let (rows, cursor_rows) = self.layout_rows(view, &doc_lines, doc_rect.width, theme);
        let visible_h = usize::from(doc_rect.height);
        let (first, last) = cursor_rows;
        if first < self.scroll {
            self.scroll = first;
        } else if visible_h > 0 && last >= self.scroll + visible_h {
            // Show the whole cursor line, or at least its start.
            self.scroll = (last + 1 - visible_h).min(first);
        }
        self.scroll = self.scroll.min(rows.len().saturating_sub(visible_h.max(1)));
        self.row_docs = rows.iter().map(|row| row.doc).collect();
        let rendered: Vec<Line<'static>> = rows
            .into_iter()
            .skip(self.scroll)
            .take(visible_h)
            .map(|row| row.line)
            .collect();
        frame.render_widget(Paragraph::new(rendered), doc_rect);

        // Fixed bottom bar: Accept / Reject / Comment whole plan / Review
        let bar_area = chunks[2];
        let btn_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(25); 4])
            .split(bar_area);
        self.button_rects = [btn_cols[0], btn_cols[1], btn_cols[2], btn_cols[3]];
        let drafts = view.draft_plan_comments().len();

        let decidable = view.plan_proposed();
        let can_accept = open_comments.is_empty() && decidable;
        let buttons = [
            (
                BottomButton::Accept,
                if !decidable {
                    " [ Accept: decided ] "
                } else if drafts > 0 {
                    " [ Accept: review first ] "
                } else if !open_comments.is_empty() {
                    " [ Accept: resolve first ] "
                } else if self.confirm_accept {
                    " [ Press again: accept ] "
                } else {
                    " [ Accept (a) ] "
                },
                if can_accept {
                    Tone::Done
                } else if decidable {
                    Tone::Waiting
                } else {
                    Tone::Queued
                },
                btn_cols[0],
            ),
            (
                BottomButton::Reject,
                if decidable {
                    " [ Reject (x) ] "
                } else {
                    " [ Reject: decided ] "
                },
                if decidable {
                    Tone::Failed
                } else {
                    Tone::Queued
                },
                btn_cols[1],
            ),
            (
                BottomButton::CommentWholePlan,
                " [ Comment plan (C) ] ",
                Tone::Active,
                btn_cols[2],
            ),
            (
                BottomButton::Review,
                if drafts > 0 {
                    " [ Review: send (R) ] "
                } else {
                    " [ Review: no drafts ] "
                },
                if drafts > 0 { Tone::Done } else { Tone::Queued },
                btn_cols[3],
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

/// The tasks a heading stands for: the one it names as `[id]`, otherwise
/// every task assigned to a section of exactly that name, otherwise the task
/// titled like the heading.
fn section_tasks(heading: &str, tasks: &[TaskRow]) -> Vec<String> {
    if let Some(task) = tasks
        .iter()
        .find(|task| heading.contains(&format!("[{}]", task.id)))
    {
        return vec![task.id.clone()];
    }
    let named = |name: &str| heading.eq_ignore_ascii_case(name.trim());
    let by_section: Vec<String> = tasks
        .iter()
        .filter(|task| {
            task.section
                .as_deref()
                .is_some_and(|section| !section.trim().is_empty() && named(section))
        })
        .map(|task| task.id.clone())
        .collect();
    if !by_section.is_empty() {
        return by_section;
    }
    tasks
        .iter()
        .filter(|task| !task.title.trim().is_empty() && named(&task.title))
        .map(|task| task.id.clone())
        .collect()
}

fn section_state(ids: &[String], tasks: &[TaskRow]) -> SectionState {
    let statuses: Vec<&str> = ids
        .iter()
        .filter_map(|id| tasks.iter().find(|task| &task.id == id))
        .map(|task| task.status.as_str())
        .collect();
    if statuses.is_empty() {
        SectionState::Open
    } else if statuses
        .iter()
        .any(|s| matches!(*s, "failed" | "needs_user"))
    {
        SectionState::Problem
    } else if statuses.iter().all(|s| *s == "approved") {
        SectionState::Done
    } else if statuses
        .iter()
        .any(|s| matches!(*s, "working" | "verifying"))
    {
        SectionState::Working
    } else {
        SectionState::Open
    }
}

#[cfg(test)]
mod wrap_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn screen(term: &Terminal<TestBackend>) -> Vec<String> {
        let buffer = term.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect()
            })
            .collect()
    }

    fn view_of(doc: &str) -> View {
        let mut view = View::default();
        view.plan_doc = doc.into();
        view
    }

    #[test]
    fn long_lines_wrap_and_every_word_stays_readable() {
        let words: Vec<String> = (0..30).map(|n| format!("word{n}")).collect();
        let view = view_of(&format!("# Title\n{}\nlast", words.join(" ")));
        let mut plan = PlanView::new("p".into());
        let mut term = Terminal::new(TestBackend::new(40, 16)).unwrap();
        term.draw(|f| plan.draw(f, f.area(), &view, &Theme::new(true, false)))
            .unwrap();
        let text = screen(&term).join("\n");
        for word in &words {
            assert!(text.contains(word.as_str()), "{word} missing in\n{text}");
        }
        // Continuation rows carry a bare separator instead of a number.
        assert!(text.contains("     │ "));
    }

    #[test]
    fn wrapped_graphemes_are_never_split() {
        let view = view_of(&"👩‍💻e\u{301}".repeat(40));
        let mut plan = PlanView::new("p".into());
        let mut term = Terminal::new(TestBackend::new(30, 14)).unwrap();
        term.draw(|f| plan.draw(f, f.area(), &view, &Theme::new(true, false)))
            .unwrap();
        let text = screen(&term).join("");
        assert!(!text.contains('\u{fffd}'));
        assert!(text.contains("👩‍💻"));
    }

    #[test]
    fn the_cursor_line_stays_in_view_and_clicks_pick_the_wrapped_line() {
        let long = "alpha beta gamma delta epsilon ".repeat(8);
        let doc: Vec<String> = (0..12)
            .map(|n| {
                if n == 5 {
                    long.clone()
                } else {
                    format!("line {n}")
                }
            })
            .collect();
        let view = view_of(&doc.join("\n"));
        let mut plan = PlanView::new("p".into());
        let theme = Theme::new(true, false);
        let mut term = Terminal::new(TestBackend::new(40, 14)).unwrap();
        plan.cursor = 11;
        term.draw(|f| plan.draw(f, f.area(), &view, &theme))
            .unwrap();
        assert!(screen(&term).join("\n").contains("line 11"));

        plan.cursor = 0;
        term.draw(|f| plan.draw(f, f.area(), &view, &theme))
            .unwrap();
        // Find a continuation row of the long line and click it.
        let continuation = plan
            .row_docs
            .iter()
            .enumerate()
            .skip(plan.scroll)
            .filter(|(_, doc)| **doc == Some(5))
            .map(|(row, _)| row)
            .nth(1)
            .expect("the long line wraps");
        let row = plan.doc_area.y + (continuation - plan.scroll) as u16;
        plan.mouse_click(plan.doc_area.x + 3, row, &view);
        assert_eq!(plan.cursor, 5);
    }

    fn proposed(doc: &str) -> View {
        let mut view = view_of(doc);
        view.tasks.push(TaskRow {
            id: "t1".into(),
            status: "proposed".into(),
            ..TaskRow::default()
        });
        view
    }

    fn press(plan: &mut PlanView, view: &View, code: KeyCode) -> PlanAction {
        plan.key(KeyEvent::new(code, KeyModifiers::NONE), view)
    }

    #[test]
    fn enter_needs_a_second_press_to_accept_and_reads_only_once_decided() {
        let view = proposed("# Plan\nbody");
        let mut plan = PlanView::new("p".into());
        // Comment is the default button, so Enter never accepts by accident.
        assert_eq!(
            press(&mut plan, &view, KeyCode::Enter),
            PlanAction::CommentWholePlan
        );
        plan.button = BottomButton::Accept;
        assert_eq!(press(&mut plan, &view, KeyCode::Enter), PlanAction::None);
        assert!(plan.confirm_accept);
        assert_eq!(press(&mut plan, &view, KeyCode::Enter), PlanAction::Accept);
        // Any other key withdraws the first press.
        assert_eq!(press(&mut plan, &view, KeyCode::Enter), PlanAction::None);
        press(&mut plan, &view, KeyCode::Down);
        assert_eq!(press(&mut plan, &view, KeyCode::Enter), PlanAction::None);
        // The explicit shortcut accepts at once.
        assert_eq!(
            press(&mut plan, &view, KeyCode::Char('a')),
            PlanAction::Accept
        );

        let mut decided = view_of("# Plan\nbody");
        decided.tasks.push(TaskRow {
            id: "t1".into(),
            status: "working".into(),
            ..TaskRow::default()
        });
        for code in [KeyCode::Char('a'), KeyCode::Char('x'), KeyCode::Char('1')] {
            assert_eq!(press(&mut plan, &decided, code), PlanAction::None);
        }
        assert_eq!(press(&mut plan, &decided, KeyCode::Esc), PlanAction::Close);
    }

    fn task_in(id: &str, section: &str, status: &str) -> TaskRow {
        TaskRow {
            id: id.into(),
            title: format!("Title {id}"),
            section: Some(section.into()),
            status: status.into(),
            ..TaskRow::default()
        }
    }

    fn states(view: &View) -> Vec<(String, SectionState)> {
        PlanView::new("p".into())
            .build_lines(view)
            .into_iter()
            .map(|line| (line.text, line.state))
            .collect()
    }

    #[test]
    fn a_section_is_done_only_when_all_its_tasks_are_and_subheadings_inherit() {
        let mut view = view_of("## Storage\ntext `t2` mentioned\n### Details\n## Other\nbody");
        view.tasks = vec![
            task_in("t1", "Storage", "approved"),
            task_in("t2", "Storage", "working"),
            task_in("t3", "Other", "failed"),
        ];
        let got = states(&view);
        // One of two tasks is still working: the section is not completed,
        // and a mention of a task id in prose does not change the section.
        assert_eq!(got[0].1, SectionState::Working);
        assert_eq!(got[1].1, SectionState::Working);
        assert_eq!(got[2].1, SectionState::Working);
        assert_eq!(got[3].1, SectionState::Problem);
        assert_eq!(got[4].1, SectionState::Problem);

        view.tasks[1].status = "approved".into();
        assert_eq!(states(&view)[0].1, SectionState::Done);
        assert_eq!(states(&view)[2].1, SectionState::Done);
    }

    #[test]
    fn headings_match_sections_exactly_not_by_substring() {
        let mut view = view_of("## Storage layer rewrite\n## Storage");
        view.tasks = vec![task_in("t1", "Storage", "approved")];
        let got = states(&view);
        assert_eq!(got[0].1, SectionState::Open);
        assert_eq!(got[1].1, SectionState::Done);
    }

    fn comment(id: u64, status: &str, line: usize) -> Comment {
        Comment {
            id,
            plan_id: Some("p".into()),
            target: format!("plan:p:line:{line}"),
            text: format!("remark {id}"),
            status: status.into(),
        }
    }

    #[test]
    fn review_sends_the_drafts_only_when_there_are_some() {
        let mut view = proposed("# Plan\nbody\nmore");
        view.plan = Some("p".into());
        let mut plan = PlanView::new("p".into());
        assert_eq!(
            press(&mut plan, &view, KeyCode::Char('R')),
            PlanAction::None
        );
        plan.button = BottomButton::Review;
        assert_eq!(press(&mut plan, &view, KeyCode::Enter), PlanAction::None);

        view.comments.insert(1, comment(1, "draft", 2));
        view.comments.insert(2, comment(2, "open", 3));
        assert_eq!(view.draft_plan_comments().len(), 1);
        assert_eq!(
            press(&mut plan, &view, KeyCode::Char('R')),
            PlanAction::SubmitReview
        );
        assert_eq!(
            press(&mut plan, &view, KeyCode::Enter),
            PlanAction::SubmitReview
        );

        // The bar offers it, the banner explains it, and a click sends it.
        let theme = Theme::new(true, false);
        let mut term = Terminal::new(TestBackend::new(100, 20)).unwrap();
        term.draw(|f| plan.draw(f, f.area(), &view, &theme))
            .unwrap();
        let text = screen(&term).join("\n");
        assert!(text.contains("Review: send (R)"), "{text}");
        assert!(text.contains("1 draft comment(s)"), "{text}");
        assert!(text.contains("1 unresolved comment(s)"), "{text}");
        assert!(
            text.contains("[draft]"),
            "the comment row says it is a draft"
        );
        let rect = plan.button_rects[3];
        assert_eq!(
            plan.mouse_click(rect.x + 2, rect.y + 1, &view),
            PlanAction::SubmitReview
        );
        // D takes back the draft on the cursor's line.
        plan.cursor = 1;
        assert_eq!(
            press(&mut plan, &view, KeyCode::Char('D')),
            PlanAction::ToggleResolveComment {
                id: 1,
                next_status: "deleted".into()
            }
        );
    }
}
