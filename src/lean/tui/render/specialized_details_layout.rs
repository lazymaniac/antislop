use super::super::details::{RESULT, REVIEW, details_footer_hint_for, is_overview};
use super::super::*;
use ratatui::widgets::{BorderType, Padding, Scrollbar, ScrollbarOrientation, ScrollbarState};

pub(super) struct DetailAreas {
    pub(super) header: Rect,
    pub(super) summary: Rect,
    pub(super) body: Rect,
    pub(super) footer: Rect,
}

pub(super) fn detail_areas(area: Rect, overview: bool, summary_len: usize) -> DetailAreas {
    let height = if overview {
        (summary_len as u16).min(area.height.saturating_sub(6))
    } else {
        0
    };
    let [header, summary, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(height),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    DetailAreas {
        header,
        summary,
        body,
        footer,
    }
}

pub(super) fn clean_header_spans(spans: &mut [Span<'static>], theme: &Theme) {
    for span in spans {
        span.content = theme.clean(&span.content).into();
        if !theme.color {
            span.style = Style {
                fg: None,
                bg: None,
                ..span.style
            };
        }
    }
}

pub(super) fn latest_round(details: &Details, view: &View) -> u64 {
    details
        .task_id()
        .and_then(|id| view.tasks.iter().find(|task| task.id == id))
        .map_or_else(
            || {
                details
                    .task
                    .as_ref()
                    .and_then(|task| task["round"].as_u64())
                    .unwrap_or(0)
            },
            |task| task.round,
        )
}

pub(super) fn round_label(details: &Details, selected: u64) -> String {
    let availability = if details.history_unavailable {
        "incomplete history · "
    } else {
        ""
    };
    let mode = if details.selected_round.is_none() {
        "current"
    } else {
        "history · return to current to send"
    };
    format!("Round {selected} · {availability}{mode} · Left/Right choose · [] tabs")
}

pub(super) fn detail_block(details: &Details, body: Rect, theme: &Theme) -> Block<'static> {
    if !details.overview() {
        return Block::default().borders(Borders::TOP);
    }
    let title = if details.show_info {
        " Task info · m session "
    } else if details.follow {
        " Session · LIVE "
    } else {
        " Session · history · G live "
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.faint())
        .style(Style::default())
        .title(title)
        .padding(Padding::horizontal(u16::from(body.width >= 20)))
}

pub(super) fn document_key(tab: usize) -> Option<&'static str> {
    match tab {
        1 => Some("messages"),
        RESULT => Some("result"),
        REVIEW => Some("review"),
        _ => None,
    }
}

pub(super) fn position_record_scroll(
    lines: &[Line<'static>],
    tab: usize,
    show_info: bool,
    follow: &mut bool,
    scroll: &mut usize,
    reveal: &mut bool,
    height: usize,
) {
    let bottom = lines.len().saturating_sub(height);
    if tab == CHECKS && std::mem::take(reveal) {
        reveal_check_scroll(lines, scroll, height);
    }
    if is_overview(tab) && !show_info && *follow {
        *scroll = bottom;
    } else {
        *scroll = (*scroll).min(bottom);
    }
    *follow = is_overview(tab) && !show_info && *scroll >= bottom;
}

pub(super) fn draw_document_scrollbar(
    frame: &mut Frame,
    area: Rect,
    total: usize,
    scroll: usize,
    theme: &Theme,
) {
    draw_scrollbar(frame, area, total, scroll, theme);
}

pub(super) fn draw_record_scrollbar(
    frame: &mut Frame,
    area: Rect,
    total: usize,
    scroll: usize,
    theme: &Theme,
) {
    draw_scrollbar(frame, area, total, scroll, theme);
}

pub(super) fn draw_scrollbar(
    frame: &mut Frame,
    area: Rect,
    total: usize,
    scroll: usize,
    theme: &Theme,
) {
    let height = usize::from(area.height);
    if total <= height || area.width == 0 {
        return;
    }
    let mut state = ScrollbarState::new(total.saturating_sub(height).saturating_add(1))
        .position(scroll)
        .viewport_content_length(height);
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some(theme.glyphs().track))
            .thumb_symbol(theme.glyphs().thumb)
            .track_style(theme.faint())
            .thumb_style(theme.dim()),
        area,
        &mut state,
    );
}

pub(super) fn reveal_check_scroll(lines: &[Line<'static>], scroll: &mut usize, height: usize) {
    let Some(index) = lines
        .iter()
        .position(|line| line.style.add_modifier.contains(Modifier::REVERSED))
    else {
        return;
    };
    if index < *scroll {
        *scroll = index;
    } else if index >= scroll.saturating_add(height) {
        *scroll = index + 1 - height;
    }
}

pub(super) fn records_hint(details: &Details, width: u16) -> String {
    if details.searching {
        format!(
            "Search records: {}_ (Enter: confirm · Esc: cancel)",
            details.query
        )
    } else {
        details_footer_hint_for(&details.key, details.tab, usize::from(width))
    }
}

pub(super) fn overview_split(body: Rect) -> Option<[Rect; 2]> {
    if body.width >= 130 {
        let info = (body.width * 2 / 5).clamp(56, 72);
        let [info, _, terminal] = Layout::horizontal([
            Constraint::Length(info),
            Constraint::Length(2),
            Constraint::Min(60),
        ])
        .areas(body);
        Some([info, terminal])
    } else if body.height >= 34 {
        let info = (body.height * 9 / 20).max(14);
        let [info, terminal] =
            Layout::vertical([Constraint::Length(info), Constraint::Min(14)]).areas(body);
        Some([info, terminal])
    } else {
        None
    }
}

pub(super) fn overview_split_hint(searching: bool, query: &str, width: u16) -> String {
    if searching {
        format!("Search: {query}_ · Enter confirm · Esc cancel")
    } else if width >= 100 {
        "↑/↓ card · Enter fold · e all · i terminal · a message · ←/→ rounds · Ctrl+W worker/verifier · [] tabs · Esc back".into()
    } else {
        "↑/↓ card · Enter fold · i terminal · a message · Esc".into()
    }
}

pub(super) fn overview_hint(searching: bool, query: &str) -> String {
    if searching {
        format!("Search: {query}_ · Enter confirm · Esc cancel")
    } else {
        "i terminal · a message · m task info · o resume earlier round · Ctrl+] release · Esc back"
            .into()
    }
}

pub(super) fn dashboard_hint(searching: bool, query: &str, width: u16) -> String {
    if searching {
        format!("Search: {query}_ · Enter confirm · Esc cancel")
    } else if width >= 90 {
        "↑/↓ card · Enter fold/expand · e all · PgUp/PgDn scroll · ←/→ rounds · c comment · / search · m terminals · Esc back".into()
    } else if width >= 50 {
        "↑/↓ card · Enter fold · e all · m terminals · Esc".into()
    } else {
        "↑/↓ · Enter · m · Esc".into()
    }
}
