//! Shared geometry and role policy for the agent/session Workbench.
//!
//! Content and native terminal adapters receive these rectangles; neither
//! adapter decides the outer split or the active target.

use super::{NavigationFocus, WorkbenchPresentation, WorkbenchTab, WorkbenchTarget};
use ratatui::layout::{Constraint, Layout, Rect};

pub fn target_at(view: &super::view::View, row: usize) -> Option<(String, String)> {
    if row == 0 {
        return Some(("conversation".to_owned(), "Session".to_owned()));
    }
    let mut index = 1;
    for task in &view.tasks {
        if index == row {
            return Some((
                format!("worker:{}", task.id),
                format!("Worker: {}", task.title),
            ));
        }
        index += 1;
        let reviewer = format!("reviewer:{}", task.id);
        if view.agents.contains_key(&reviewer) {
            if index == row {
                return Some((reviewer, format!("Reviewer: {}", task.title)));
            }
            index += 1;
        }
    }
    None
}

const MIN_PANEL_HEIGHT: u16 = 8;
const LIST_WIDTH: u16 = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkbenchRole {
    Session,
    Worker,
    Reviewer,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchContext {
    pub target: WorkbenchTarget,
    pub role: WorkbenchRole,
    pub round: Option<u64>,
    pub focus: NavigationFocus,
}

pub fn context(target: &WorkbenchTarget, presentation: &WorkbenchPresentation) -> WorkbenchContext {
    let role = match target {
        WorkbenchTarget::Session => WorkbenchRole::Session,
        WorkbenchTarget::Agent(key) => {
            match key.split_once(':').map_or(key.as_str(), |(role, _)| role) {
                "worker" => WorkbenchRole::Worker,
                "reviewer" => WorkbenchRole::Reviewer,
                other => WorkbenchRole::Other(other.to_owned()),
            }
        }
    };
    WorkbenchContext {
        target: target.clone(),
        role,
        round: presentation.round,
        focus: presentation.focus,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkbenchAreas {
    pub list: Rect,
    pub content: Rect,
    pub separator: Rect,
    pub terminal: Rect,
    pub header: Rect,
    pub footer: Rect,
    pub list_visible: bool,
    pub split: bool,
}

/// Allocate the Workbench once for both task and session content adapters.
/// The 100x24 threshold is measured against the complete terminal viewport.
pub fn areas(
    viewport: Rect,
    presentation: &WorkbenchPresentation,
    focus: NavigationFocus,
) -> WorkbenchAreas {
    if viewport.is_empty() {
        return WorkbenchAreas::default();
    }
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(viewport);
    let wide_enough = viewport.width >= 100 && viewport.height >= 24;
    let list_visible = wide_enough && presentation.list_visible;
    let (list, work) = if list_visible {
        let [list, _, work] = Layout::horizontal([
            Constraint::Length(LIST_WIDTH),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .areas(body);
        (list, work)
    } else {
        (Rect::default(), body)
    };
    let can_split = viewport.height >= 24
        && presentation.split
        && !presentation.zoomed
        && work.height > MIN_PANEL_HEIGHT * 2;
    let (content, separator, terminal) = if can_split {
        let usable = work.height.saturating_sub(1);
        let percent = u16::from(presentation.terminal_percent.clamp(1, 99));
        let preferred_terminal = (u32::from(usable) * u32::from(percent) / 100) as u16;
        let terminal_height = preferred_terminal.clamp(MIN_PANEL_HEIGHT, usable - MIN_PANEL_HEIGHT);
        let content_height = usable - terminal_height;
        let [content, separator, terminal] = Layout::vertical([
            Constraint::Length(content_height),
            Constraint::Length(1),
            Constraint::Length(terminal_height),
        ])
        .areas(work);
        (content, separator, terminal)
    } else if presentation.zoomed && presentation.split {
        if focus == NavigationFocus::WorkbenchTerminal {
            (Rect::default(), Rect::default(), work)
        } else {
            (work, Rect::default(), Rect::default())
        }
    } else if viewport.height < 24 && focus == NavigationFocus::WorkbenchTerminal {
        (Rect::default(), Rect::default(), work)
    } else {
        (work, Rect::default(), Rect::default())
    };
    WorkbenchAreas {
        list,
        content,
        separator,
        terminal,
        header,
        footer,
        list_visible,
        split: can_split,
    }
}

/// Resize the PTY share from a pointer position, clamped to both 8-row minima.
pub fn resize_at(viewport: Rect, presentation: &mut WorkbenchPresentation, row: u16) -> bool {
    let current = areas(viewport, presentation, NavigationFocus::WorkbenchContent);
    if !current.split
        || current.content.height + current.separator.height + current.terminal.height == 0
    {
        return false;
    }
    let work_top = current.content.y;
    let work_bottom = current.terminal.bottom();
    let usable = work_bottom.saturating_sub(work_top).saturating_sub(1);
    let terminal = work_bottom
        .saturating_sub(row.saturating_add(1))
        .clamp(MIN_PANEL_HEIGHT, usable.saturating_sub(MIN_PANEL_HEIGHT));
    let percent = u32::from(terminal) * 100 / u32::from(usable.max(1));
    presentation.terminal_percent = percent.clamp(1, 99) as u8;
    true
}

pub fn tabs_for(key: &str, delivers_changes: bool) -> Vec<WorkbenchTab> {
    let role = key.split_once(':').map_or(key, |(role, _)| role);
    let mut tabs = vec![WorkbenchTab::Summary, WorkbenchTab::Conversation];
    if matches!(
        role,
        "worker" | "reviewer" | "planner" | "plan-reviewer" | "auditor" | "orchestrator"
    ) {
        tabs.push(WorkbenchTab::Results);
    }
    if role == "reviewer" {
        tabs.push(WorkbenchTab::Checks);
    }
    tabs.push(WorkbenchTab::Comments);
    if delivers_changes {
        tabs.push(WorkbenchTab::Diff);
    }
    tabs
}

pub fn default_tab(_key: &str, delivers_changes: bool) -> WorkbenchTab {
    if delivers_changes {
        WorkbenchTab::Diff
    } else {
        WorkbenchTab::Conversation
    }
}

pub fn neighbour_tab(
    key: &str,
    delivers_changes: bool,
    current: WorkbenchTab,
    direction: isize,
) -> WorkbenchTab {
    let tabs = tabs_for(key, delivers_changes);
    let at = tabs.iter().position(|tab| *tab == current).unwrap_or(0) as isize;
    tabs[(at + direction).rem_euclid(tabs.len() as isize) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_threshold_split_and_small_viewport_focus_are_bounded() {
        let presentation = WorkbenchPresentation::default();
        let below = areas(
            Rect::new(0, 0, 99, 24),
            &presentation,
            NavigationFocus::WorkbenchContent,
        );
        assert!(!below.list_visible);
        assert!(below.split);
        let at = areas(
            Rect::new(0, 0, 100, 24),
            &presentation,
            NavigationFocus::WorkbenchContent,
        );
        assert!(at.list_visible);
        assert!(at.split);
        assert!(at.content.height >= MIN_PANEL_HEIGHT);
        assert!(at.terminal.height >= MIN_PANEL_HEIGHT);
        let short = areas(
            Rect::new(0, 0, 80, 23),
            &presentation,
            NavigationFocus::WorkbenchTerminal,
        );
        assert!(!short.split);
        assert!(short.content.is_empty());
        assert_eq!(
            short.terminal.height,
            short.footer.y - short.header.bottom()
        );
    }

    #[test]
    fn split_resize_and_zoom_keep_a_valid_preference() {
        let mut presentation = WorkbenchPresentation::default();
        let viewport = Rect::new(0, 0, 120, 40);
        assert!(resize_at(viewport, &mut presentation, 12));
        assert!(presentation.terminal_percent > 40);
        let before = presentation.terminal_percent;
        presentation.zoomed = true;
        let zoom = areas(viewport, &presentation, NavigationFocus::WorkbenchTerminal);
        assert!(zoom.content.is_empty());
        assert_eq!(zoom.terminal.height, 38);
        assert_eq!(presentation.terminal_percent, before);
    }

    #[test]
    fn tab_policy_defaults_to_diff_only_for_delivery_agents() {
        assert_eq!(default_tab("worker:task", true), WorkbenchTab::Diff);
        assert_eq!(
            default_tab("reviewer:task", false),
            WorkbenchTab::Conversation
        );
        assert!(tabs_for("reviewer:task", false).contains(&WorkbenchTab::Checks));
        assert!(!tabs_for("worker:task", true).contains(&WorkbenchTab::Checks));
        assert!(!tabs_for("worker:task", false).contains(&WorkbenchTab::Diff));
    }

    #[test]
    fn context_identifies_target_role_round_and_focus() {
        let presentation = WorkbenchPresentation {
            round: Some(3),
            focus: NavigationFocus::WorkbenchTerminal,
            ..WorkbenchPresentation::default()
        };
        let context = context(
            &WorkbenchTarget::Agent("reviewer:task-1".into()),
            &presentation,
        );
        assert_eq!(context.role, WorkbenchRole::Reviewer);
        assert_eq!(context.round, Some(3));
        assert_eq!(context.focus, NavigationFocus::WorkbenchTerminal);
    }
}
