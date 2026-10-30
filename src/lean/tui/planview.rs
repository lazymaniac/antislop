//! Full-screen interactive plan document viewer with line-by-line cursor
//! navigation, inline line comments, section completion underlining,
//! dependency/parallel breakdown, and a fixed bottom action bar.

mod actions;
mod document;
mod render;
#[cfg(test)]
#[path = "planview/tests.rs"]
mod wrap_tests;

use super::theme::{Theme, Tone};
use super::view::{Comment, TaskRow, View, clean, status_label};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
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
pub(super) const GUTTER: usize = 7;

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
}
