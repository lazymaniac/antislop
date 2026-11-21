use super::super::details::{RESULT, REVIEW, details_footer_hint_for, is_overview};
use super::super::view::fit_hint;
use super::super::*;
use ratatui::widgets::{Padding, Scrollbar, ScrollbarOrientation, ScrollbarState};

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
        span.style = theme.style(span.style);
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
    format!("Round {selected} · {availability}{mode} · ,/. choose · Ctrl+←/→ tabs")
}

pub(super) fn detail_block(details: &Details, body: Rect, theme: &Theme) -> Block<'static> {
    if !details.overview() {
        return Block::default()
            .borders(Borders::TOP)
            .border_set(theme.borders())
            .border_style(theme.separator(false));
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
        .border_set(theme.borders())
        .border_style(theme.faint())
        .style(Style::default())
        .title(theme.chrome(title))
        .padding(Padding::horizontal(u16::from(body.width >= 20)))
}

pub(super) fn document_key(details: &super::super::details::Details) -> Option<String> {
    match details.tab {
        RESULT if details.key.starts_with("reviewer:") => Some("reviewer_result".into()),
        RESULT => details
            .selected_result_document
            .as_ref()
            .filter(|key| details.has_result_key(key))
            .cloned()
            .or_else(|| details.visible_result_keys().first().cloned())
            .or_else(|| Some("result".into())),
        REVIEW => Some("review".into()),
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

pub(super) fn records_hint(details: &Details, width: u16, theme: &Theme) -> String {
    if details.searching {
        format!(
            "Search records: {}{}",
            theme.clean(&details.query),
            theme.chrome("_ (Enter: confirm · Esc: cancel)")
        )
    } else {
        theme.chrome(&details_footer_hint_for(
            &details.key,
            details.tab,
            usize::from(width),
        ))
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

pub(super) fn overview_split_hint(
    searching: bool,
    query: &str,
    width: u16,
    theme: &Theme,
) -> String {
    if searching {
        return search_hint(query, theme);
    }
    theme.chrome(&fit_hint(
        &[
            "↑/↓ card",
            "Enter fold",
            "t terminal",
            "Ctrl+K message",
            "e all",
            "Ctrl+←/→ tabs",
            ",/. rounds",
            "Ctrl+W stage",
            "Esc back",
        ],
        usize::from(width),
        true,
    ))
}

pub(super) fn overview_hint(searching: bool, query: &str, width: u16, theme: &Theme) -> String {
    if searching {
        return search_hint(query, theme);
    }
    theme.chrome(&fit_hint(
        &[
            "t terminal",
            "Ctrl+K message",
            "m task info",
            "o resume earlier round",
            "Ctrl+] release",
            "Esc back",
        ],
        usize::from(width),
        true,
    ))
}

pub(super) fn dashboard_hint(searching: bool, query: &str, width: u16, theme: &Theme) -> String {
    if searching {
        return search_hint(query, theme);
    }
    theme.chrome(&fit_hint(
        &[
            "↑/↓ card",
            "Enter fold/expand",
            "m terminals",
            "e all",
            "c comment",
            "/ search",
            ",/. rounds",
            "PgUp/PgDn scroll",
            "Esc back",
        ],
        usize::from(width),
        true,
    ))
}

fn search_hint(query: &str, theme: &Theme) -> String {
    format!(
        "Search: {}{}",
        theme.clean(query),
        theme.chrome("_ · Enter confirm · Esc cancel")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn overview_search_hints_separate_chrome_from_query() {
        let query = "Zażółć 👩‍💻 e\u{301} │ ·";
        for ascii in [false, true] {
            for color in [false, true] {
                let theme = Theme::new(color, ascii);
                let expected = format!(
                    "Search: {query}{}",
                    theme.chrome("_ · Enter confirm · Esc cancel")
                );
                for hint in [overview_split_hint, overview_hint, dashboard_hint] {
                    assert_eq!(hint(true, query, 160, &theme), expected);
                }
            }
        }
    }

    #[test]
    fn overview_details_block_renders_square_corners() {
        let area = Rect::new(0, 0, 24, 6);
        let details = Details::new("orchestrator".into());
        let theme = Theme::new(true, false);
        let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();

        terminal
            .draw(|frame| {
                frame.render_widget(detail_block(&details, area, &theme), area);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), "┌");
        assert_eq!(buffer[(area.width - 1, 0)].symbol(), "┐");
        assert_eq!(buffer[(0, area.height - 1)].symbol(), "└");
        assert_eq!(buffer[(area.width - 1, area.height - 1)].symbol(), "┘");
    }
}
