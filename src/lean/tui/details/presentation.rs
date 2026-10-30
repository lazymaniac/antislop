pub use super::checks::{CheckRow, check_rows};
use super::*;

pub fn details_footer_hint_for(key: &str, tab: usize, width: usize) -> String {
    let controls = details_footer_hint(tab, width);
    if width >= 45 {
        format!("1–{} / {controls}", tabs_for(key).len())
    } else {
        controls.to_owned()
    }
}

pub fn details_footer_hint(tab: usize, width: usize) -> &'static str {
    match tab {
        tab if is_overview(tab) => overview_hint(width),
        CHECKS => checks_hint(width),
        COMMENTS => comments_hint(width),
        _ => content_hint(width),
    }
}

fn overview_hint(width: usize) -> &'static str {
    match width {
        90.. => {
            "[] tabs · ←/→ rounds · ↑/↓ PgUp/PgDn · gg/G top/live · m dashboard · / search · c comment · Esc back"
        }
        65..=89 => {
            "[] tabs · ←/→ rounds · ↑/↓ scroll · gg/G top/live · m dashboard · / search · Esc back"
        }
        45..=64 => "[] tabs · ←/→ rounds · gg/G top/live · m dashboard · Esc",
        _ => "gg/G · m info · Esc back",
    }
}

fn checks_hint(width: usize) -> &'static str {
    match width {
        80.. => {
            "[] tabs · ←/→ rounds · ↑/↓ select check · c comment on check · / search · d diff · ? help · Esc back"
        }
        45..=79 => "[] tabs · ←/→ rounds · ↑/↓ select · c comment · Esc",
        _ => "c com · Esc back",
    }
}

fn comments_hint(width: usize) -> &'static str {
    match width {
        95.. => {
            "[] tabs · ←/→ rounds · ↑/↓ select · / search · c comment · r resolve · o reopen · x delete · d diff · ? help · Esc back"
        }
        65..=94 => {
            "[] tabs · ←/→ rounds · ↑/↓ select · / search · c add · r/o/x status · d diff · Esc back"
        }
        45..=64 => "[] tabs · ←/→ rounds · ↑/↓ select · c add · r/o/x · Esc",
        _ => "c com · Esc back",
    }
}

fn content_hint(width: usize) -> &'static str {
    match width {
        90.. => {
            "[] tabs · ←/→ rounds · ↑/↓ PgUp/PgDn · gg/G top/end · / search · c comment · d diff · Esc back"
        }
        65..=89 => "[] tabs · ←/→ rounds · ↑/↓ scroll · gg/G top/end · / search · Esc back",
        45..=64 => "[] tabs · ←/→ rounds · gg/G top/end · Esc",
        _ => "gg/G · ↑/↓ · Esc back",
    }
}

pub fn format_tabs<'a>(agent_key: &str, active_tab: usize, max_width: usize) -> Line<'a> {
    let tabs = TabStrip::new(agent_key, active_tab);
    if tabs.full_width() <= max_width {
        tabs.full()
    } else {
        tabs.narrow(max_width)
    }
}

struct TabStrip<'a> {
    key: &'a str,
    agent: String,
    visible: Vec<usize>,
    active: usize,
}

impl<'a> TabStrip<'a> {
    fn new(key: &'a str, active_tab: usize) -> Self {
        let agent = match key.split_once(':') {
            Some(("worker" | "verifier", id)) => format!(" {id} "),
            _ => format!(" {} ", agent_name(key)),
        };
        let visible = tabs_for(key);
        let active = visible
            .iter()
            .position(|tab| *tab == active_tab)
            .unwrap_or(0);
        Self {
            key,
            agent,
            visible,
            active,
        }
    }

    fn name(&self, index: usize) -> &'static str {
        tab_name(self.key, self.visible[index])
    }

    fn full_width(&self) -> usize {
        self.agent.len()
            + (0..self.visible.len())
                .map(|index| {
                    self.name(index).len()
                        + if index == self.active { 4 } else { 3 }
                        + if section_starts(self.key, self.visible[index]) {
                            2
                        } else {
                            0
                        }
                })
                .sum::<usize>()
    }

    fn agent_span(&self) -> Span<'static> {
        Span::styled(
            self.agent.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )
    }

    fn full(&self) -> Line<'static> {
        let mut spans = vec![self.agent_span()];
        for index in 0..self.visible.len() {
            if section_starts(self.key, self.visible[index]) {
                spans.push(Span::styled(" │", dim()));
            }
            spans.push(Span::raw(" "));
            spans.push(self.tab_span(index, false));
        }
        Line::from(spans)
    }

    fn narrow(&self, max_width: usize) -> Line<'static> {
        let (left, right) = self.window(max_width);
        let mut spans = vec![self.agent_span()];
        if left > 0 {
            spans.push(Span::styled("◀ ", dim()));
        }
        spans.extend((left..=right).map(|index| self.tab_span(index, true)));
        if right + 1 < self.visible.len() {
            spans.push(Span::styled("▶", dim()));
        }
        Line::from(spans)
    }

    fn window(&self, max_width: usize) -> (usize, usize) {
        let (mut left, mut right) = (self.active, self.active);
        let mut width = self.agent.len() + self.name(self.active).len() + 4;
        while left > 0 || right + 1 < self.visible.len() {
            let changed = self.extend_right(&mut right, &mut width, max_width)
                | self.extend_left(&mut left, &mut width, max_width);
            if !changed {
                break;
            }
        }
        (left, right)
    }

    fn extend_right(&self, right: &mut usize, width: &mut usize, max: usize) -> bool {
        if *right + 1 >= self.visible.len() {
            return false;
        }
        let extra = self.name(*right + 1).len() + 5;
        if *width + extra > max {
            return false;
        }
        *right += 1;
        *width += extra;
        true
    }

    fn extend_left(&self, left: &mut usize, width: &mut usize, max: usize) -> bool {
        if *left == 0 {
            return false;
        }
        let extra = self.name(*left - 1).len() + 5;
        if *width + extra > max {
            return false;
        }
        *left -= 1;
        *width += extra;
        true
    }

    fn tab_span(&self, index: usize, narrow: bool) -> Span<'static> {
        if index == self.active {
            let label = if narrow {
                format!("[{}] ", self.name(index))
            } else {
                format!("[{}]", self.name(index))
            };
            Span::styled(
                label,
                Style::default()
                    .fg(super::super::theme::ayu::YELLOW)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            let label = if narrow {
                format!("{} ", self.name(index))
            } else {
                format!(" {} ", self.name(index))
            };
            Span::styled(label, dim())
        }
    }
}
