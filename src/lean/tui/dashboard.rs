//! The task dashboard of a worker or verifier: the Overview's second face
//! (`m`), beside the live terminals. A hero strip says where the task stands
//! at a glance; below it, cards (Now, Goal, Checks, Findings, …) hold the
//! detail. Up/Down move between cards, Enter folds or expands one, so a long
//! check list or report never hides the rest. Everything is read from the
//! same sources as the graph cards ([`taskinfo`]), never re-derived.

mod hero;
mod sections;

use super::details::{Details, check_rows};
use super::taskinfo;
use super::theme::Theme;
use super::view::{TaskRow, View, agent_name, clean, duration, status_label, wrap};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

/// Below this the two columns would be narrower than a readable line.
pub(super) const TWO_COLUMNS: u16 = 118;
pub(super) const GAP: u16 = 3;

/// How much of a card is shown. Enter cycles Preview → Full → Folded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fold {
    #[default]
    Preview,
    Full,
    Folded,
}

pub(super) struct Section {
    pub(super) id: &'static str,
    pub(super) title: &'static str,
    pub(super) meta: Vec<Span<'static>>,
    pub(super) body: Vec<Line<'static>>,
    /// Body lines shown before "+N more".
    pub(super) preview: usize,
    pub(super) right: bool,
}

/// The hero's gauge, `3/6` of it filled.
fn gauge(
    theme: &Theme,
    done: usize,
    total: usize,
    width: usize,
    fill: Style,
) -> Vec<Span<'static>> {
    let (full, empty) = if theme.ascii {
        ("#", "-")
    } else {
        ("█", "░")
    };
    let on = if total == 0 {
        0
    } else {
        (done * width).div_ceil(total).min(width)
    };
    vec![
        Span::styled(full.repeat(on), fill),
        Span::styled(empty.repeat(width - on), theme.faint()),
    ]
}

/// 1234 → `1.2k`, so a token count fits a chip.
fn compact(n: u64) -> String {
    match n {
        0..1000 => n.to_string(),
        1000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// A paragraph wrapped to `width`, blank source lines kept.
fn para(theme: &Theme, text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    let text = theme.clean(text);
    text.lines()
        .flat_map(|line| {
            if line.trim().is_empty() {
                vec![String::new()]
            } else {
                wrap(line, width)
            }
        })
        .map(|line| Line::styled(line, style))
        .collect()
}

/// `mark text` wrapped, continuation lines hanging under the text.
fn item(
    theme: &Theme,
    mark: Span<'static>,
    lead: Option<String>,
    text: &str,
    width: usize,
    style: Style,
) -> Vec<Line<'static>> {
    let lead = lead.map(|l| format!("{l} ")).unwrap_or_default();
    let indent = 2 + lead.width();
    let pieces = wrap(&theme.clean(text), width.saturating_sub(indent).max(1));
    let Some((first, rest)) = pieces.split_first() else {
        return vec![empty_item_line(mark, lead, theme)];
    };
    let mut out = vec![first_item_line(mark, &lead, first, theme, style)];
    out.extend(
        rest.iter()
            .map(|piece| continuation_item_line(indent, piece, style)),
    );
    out
}

fn first_item_line(
    mark: Span<'static>,
    lead: &str,
    text: &str,
    theme: &Theme,
    style: Style,
) -> Line<'static> {
    Line::from(vec![
        mark,
        Span::raw(" "),
        Span::styled(lead.to_owned(), theme.dim()),
        Span::styled(text.to_owned(), style),
    ])
}

fn continuation_item_line(indent: usize, text: &str, style: Style) -> Line<'static> {
    Line::from(vec![
        Span::raw(" ".repeat(indent)),
        Span::styled(text.to_owned(), style),
    ])
}

fn empty_item_line(mark: Span<'static>, lead: String, theme: &Theme) -> Line<'static> {
    Line::from(vec![mark, Span::raw(" "), Span::styled(lead, theme.dim())])
}

fn chip(theme: &Theme, label: &str, value: String, style: Style) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!("{label} "), theme.dim()),
        Span::styled(value, style),
    ]
}

fn join_chips(theme: &Theme, chips: Vec<Vec<Span<'static>>>) -> Line<'static> {
    let sep = format!(" {} ", theme.glyphs().vertical);
    let mut spans = vec![Span::raw(" ")];
    for (index, chip) in chips.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(sep.clone(), theme.faint()));
        }
        spans.extend(chip);
    }
    Line::from(spans)
}

impl Details {
    /// The task row this details screen is about.
    fn row<'a>(&self, view: &'a View) -> Option<&'a TaskRow> {
        self.task_id()
            .and_then(|id| view.tasks.iter().find(|task| task.id == id))
    }

    /// The dashboard replaces the terminals while `m` is on and nothing
    /// narrows the text with a search.
    pub fn dashboard_visible(&self) -> bool {
        self.overview()
            && (self.show_info || self.split)
            && self.key != "conversation"
            && self.query.is_empty()
    }

    pub(super) fn fold_of(&self, id: &str) -> Fold {
        self.dash_fold.get(id).copied().unwrap_or_default()
    }

    /// Moves the card focus by `delta`; clamped at both ends.
    pub fn dash_move(&mut self, delta: isize) {
        let last = self.dash_ids.len().saturating_sub(1);
        self.dash_focus = (self.dash_focus as isize + delta).clamp(0, last as isize) as usize;
        self.dash_reveal = true;
    }

    pub fn dash_jump(&mut self, last: bool) {
        self.dash_focus = if last {
            self.dash_ids.len().saturating_sub(1)
        } else {
            0
        };
        self.dash_reveal = true;
    }

    /// Enter on a card: preview → full → folded → preview.
    pub fn dash_cycle(&mut self) {
        if let Some(id) = self.dash_ids.get(self.dash_focus).copied() {
            let next = match self.fold_of(id) {
                Fold::Preview => Fold::Full,
                Fold::Full => Fold::Folded,
                Fold::Folded => Fold::Preview,
            };
            self.dash_fold.insert(id, next);
            self.dash_reveal = true;
        }
    }

    /// `e`: everything open in full, or, when it already is, back to previews.
    pub fn dash_expand_all(&mut self) {
        let all_full = self
            .dash_ids
            .iter()
            .all(|id| self.fold_of(id) == Fold::Full);
        let next = if all_full { Fold::Preview } else { Fold::Full };
        for id in self.dash_ids.clone() {
            self.dash_fold.insert(id, next);
        }
    }

    pub fn dash_scroll(&mut self, delta: isize) {
        self.dash_scroll = (self.dash_scroll as isize + delta).max(0) as usize;
    }
}

/// The raw reason as one whole line, never cut.
fn theme_clean_one(theme: &Theme) -> impl Fn(&str) -> String + '_ {
    move |text| {
        theme
            .clean(text)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
}
