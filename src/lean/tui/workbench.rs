//! Shared geometry and role policy for the agent/session Workbench.
//!
//! Content and native terminal adapters receive these rectangles; neither
//! adapter decides the outer split or the active target.

use super::{NavigationFocus, WorkbenchPresentation, WorkbenchTab, WorkbenchTarget};
use ratatui::layout::{Constraint, Layout, Rect};

#[cfg(test)]
thread_local! {
    pub(super) static PROJECTION_BUILDS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

#[derive(Debug, Clone)]
pub struct SidebarRow {
    pub key: String,
    pub label: String,
    pub stage: bool,
    pub(super) metadata: bool,
}

pub fn sidebar(view: &super::view::View) -> Vec<SidebarRow> {
    let projection = super::work_units::WorkUnits::new(view);
    sidebar_with_units(view, &projection)
}

/// Derive rows from the same ownership projection used to paint selection.
pub(super) fn sidebar_with_units(
    view: &super::view::View,
    projection: &super::work_units::WorkUnits,
) -> Vec<SidebarRow> {
    #[cfg(test)]
    PROJECTION_BUILDS.with(|count| {
        let (units, rows) = count.get();
        count.set((units, rows + 1));
    });
    let mut rows = vec![SidebarRow {
        key: "conversation".into(),
        label: "SESSION TASKS".into(),
        stage: false,
        metadata: false,
    }];
    for unit in &projection.units {
        rows.push(SidebarRow {
            key: unit.card.clone(),
            label: String::new(),
            stage: false,
            metadata: false,
        });
        rows.push(SidebarRow {
            key: unit.card.clone(),
            label: unit.title.clone(),
            stage: false,
            metadata: false,
        });
        if let Some(task) = unit
            .task
            .as_deref()
            .and_then(|id| view.tasks.iter().find(|task| task.id == id))
        {
            rows.push(SidebarRow {
                key: unit.card.clone(),
                label: format!("  {} · round {}", task.kind, task.round),
                stage: false,
                metadata: true,
            });
        }
        for stage in &unit.stages {
            rows.push(SidebarRow {
                key: stage.id.0.clone(),
                label: format!("  {}", stage.label),
                stage: true,
                metadata: false,
            });
        }
    }
    rows
}

pub fn visible_start(rows: &[SidebarRow], target: &WorkbenchTarget, height: u16) -> usize {
    let selected = match target {
        WorkbenchTarget::Session => "conversation",
        WorkbenchTarget::Agent(key) => key,
    };
    let at = rows
        .iter()
        .position(|row| row.stage && row.key == selected)
        .or_else(|| rows.iter().position(|row| row.key == selected))
        .unwrap_or(0);
    at.saturating_sub(usize::from(height.saturating_sub(2)))
}

const MIN_PANEL_HEIGHT: u16 = 8;
const LIST_WIDTH: u16 = 30;

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
/// Secondary panes need 120 columns; the PTY split needs 30 rows.
/// Both thresholds are measured against the complete terminal viewport.
pub fn areas(
    viewport: Rect,
    presentation: &WorkbenchPresentation,
    focus: NavigationFocus,
) -> WorkbenchAreas {
    if viewport.is_empty() {
        return WorkbenchAreas::default();
    }
    let margin = ratatui::layout::Margin::new(
        if viewport.width >= 100 {
            2
        } else {
            u16::from(viewport.width >= 40)
        },
        u16::from(viewport.height >= 30),
    );
    let spacious = viewport.height >= 40;
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(if spacious { 4 } else { 2 }),
        Constraint::Min(0),
        Constraint::Length(if spacious { 4 } else { 2 }),
    ])
    .areas(viewport.inner(margin));
    let wide_enough = viewport.width >= 120;
    let list_visible = (wide_enough && presentation.list_visible)
        || (presentation.list_requested && viewport.width >= 60);
    let (list, work) = if list_visible {
        let [list, _, work] = Layout::horizontal([
            Constraint::Length(LIST_WIDTH),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .areas(body);
        (list, work)
    } else {
        (Rect::default(), body)
    };
    let can_split = viewport.height >= 30
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
    } else if viewport.height < 30 && focus == NavigationFocus::WorkbenchTerminal {
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
    if key != "conversation" {
        tabs.push(WorkbenchTab::Results);
    }
    if matches!(role, "worker" | "reviewer") {
        tabs.push(WorkbenchTab::Checks);
    }
    tabs.push(WorkbenchTab::Comments);
    if delivers_changes {
        tabs.push(WorkbenchTab::Diff);
    }
    tabs
}

pub fn default_tab(key: &str, delivers_changes: bool) -> WorkbenchTab {
    if delivers_changes && key.starts_with("worker:") {
        WorkbenchTab::Diff
    } else {
        WorkbenchTab::Summary
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

pub(super) fn tab_line(
    key: &str,
    delivers: bool,
    active: WorkbenchTab,
    theme: &super::Theme,
) -> ratatui::text::Line<'static> {
    use ratatui::text::Span;
    let mut spans = Vec::new();
    for (index, tab) in tabs_for(key, delivers).into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled("  ", theme.dim()));
        }
        let label = match tab {
            WorkbenchTab::Summary => "Summary",
            WorkbenchTab::Conversation => "Conversation",
            WorkbenchTab::Results => "Results",
            WorkbenchTab::Checks => "Checks",
            WorkbenchTab::Comments => "Comments",
            WorkbenchTab::Diff => "Diff",
        };
        spans.push(Span::styled(
            label,
            if tab == active {
                theme.accent()
            } else {
                theme.dim()
            },
        ));
    }
    ratatui::text::Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_threshold_split_and_small_viewport_focus_are_bounded() {
        let presentation = WorkbenchPresentation::default();
        let below = areas(
            Rect::new(0, 0, 119, 29),
            &presentation,
            NavigationFocus::WorkbenchContent,
        );
        assert!(!below.list_visible);
        assert!(!below.split);
        let at = areas(
            Rect::new(0, 0, 120, 30),
            &presentation,
            NavigationFocus::WorkbenchContent,
        );
        assert!(at.list_visible);
        assert!(at.split);
        assert!(at.content.height >= MIN_PANEL_HEIGHT);
        assert!(at.terminal.height >= MIN_PANEL_HEIGHT);
        let short = areas(
            Rect::new(0, 0, 80, 24),
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
        assert_eq!(zoom.terminal.height, 30);
        assert_eq!(presentation.terminal_percent, before);
    }

    #[test]
    fn tab_policy_defaults_to_diff_only_for_delivery_agents() {
        assert_eq!(default_tab("worker:task", true), WorkbenchTab::Diff);
        assert_eq!(default_tab("reviewer:task", false), WorkbenchTab::Summary);
        assert!(tabs_for("reviewer:task", false).contains(&WorkbenchTab::Checks));
        assert!(tabs_for("worker:task", true).contains(&WorkbenchTab::Checks));
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
