//! The graph's filter menu: which relations the graph draws, and which cards
//! it keeps by state and by provider. Every row is a checkbox switched on and
//! off by itself; an empty state or provider set keeps every card.

use super::graph::{Health, RELATIONS, Relations, Scope};
use super::theme::ayu;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    Relation(usize),
    State(Health),
    Provider(String),
}

#[derive(Debug, Clone, Default)]
pub struct FilterMenu {
    cursor: usize,
    /// Screen row of each checkbox as last drawn, for clicks.
    rows: Vec<(u16, Rect)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Close,
}

fn rows(providers: &[String]) -> Vec<Row> {
    (0..RELATIONS.len())
        .map(Row::Relation)
        .chain(Health::ALL.into_iter().map(Row::State))
        .chain(providers.iter().cloned().map(Row::Provider))
        .collect()
}

fn toggle(row: &Row, relations: &mut Relations, scope: &mut Scope) {
    match row {
        Row::Relation(index) => relations.toggle(*index),
        Row::State(state) => {
            if !scope.states.remove(state) {
                scope.states.insert(*state);
            }
        }
        Row::Provider(provider) => {
            if !scope.providers.remove(provider) {
                scope.providers.insert(provider.clone());
            }
        }
    }
}

fn checked(row: &Row, relations: Relations, scope: &Scope) -> bool {
    match row {
        Row::Relation(index) => relations.shown(*index),
        Row::State(state) => scope.states.contains(state),
        Row::Provider(provider) => scope.providers.contains(provider),
    }
}

impl FilterMenu {
    pub fn key(
        &mut self,
        key: KeyEvent,
        providers: &[String],
        relations: &mut Relations,
        scope: &mut Scope,
    ) -> Outcome {
        let rows = rows(providers);
        self.cursor = self.cursor.min(rows.len().saturating_sub(1));
        self.apply_key(key.code, &rows, relations, scope)
    }

    fn apply_key(
        &mut self,
        code: KeyCode,
        rows: &[Row],
        relations: &mut Relations,
        scope: &mut Scope,
    ) -> Outcome {
        match code {
            KeyCode::Esc | KeyCode::Char('f' | 'q') => return Outcome::Close,
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor = (self.cursor + 1).min(rows.len().saturating_sub(1));
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                if let Some(row) = rows.get(self.cursor) {
                    toggle(row, relations, scope);
                }
            }
            KeyCode::Char(digit @ '1'..='4') => {
                relations.toggle(usize::from(digit as u8 - b'1'));
            }
            KeyCode::Char('r') => {
                *relations = Relations::default();
                scope.states.clear();
                scope.providers.clear();
            }
            _ => {}
        }
        Outcome::Stay
    }

    /// Toggles the row under the pointer; a click outside closes the menu.
    pub fn click(
        &mut self,
        column: u16,
        row: u16,
        providers: &[String],
        relations: &mut Relations,
        scope: &mut Scope,
    ) -> Outcome {
        let hit = self.rows.iter().position(|(_, rect)| {
            rect.y == row && column >= rect.x && column < rect.x + rect.width
        });
        let Some(index) = hit else {
            let inside = self.rows.first().is_some_and(|(_, first)| {
                column >= first.x.saturating_sub(1) && column <= first.x + first.width
            });
            return if inside {
                Outcome::Stay
            } else {
                Outcome::Close
            };
        };
        self.cursor = index;
        if let Some(found) = rows(providers).get(index) {
            toggle(found, relations, scope);
        }
        Outcome::Stay
    }

    pub fn draw(
        &mut self,
        frame: &mut Frame,
        providers: &[String],
        relations: Relations,
        scope: &Scope,
    ) {
        let rows = rows(providers);
        self.cursor = self.cursor.min(rows.len().saturating_sub(1));
        let area = frame.area();
        // Three section headings, a blank line before two of them and the
        // footer, inside the border.
        let wanted = rows.len() as u16 + 3 + 2 + 2 + 2;
        let boxed = menu_area(area, wanted);
        frame.render_widget(Clear, boxed);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ayu::BLUE))
            .title(" Graph filters ");
        let inner = block.inner(boxed);
        frame.render_widget(block, boxed);
        let lines = self.draw_rows(&rows, relations, scope, inner);
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn draw_rows(
        &mut self,
        rows: &[Row],
        relations: Relations,
        scope: &Scope,
        inner: Rect,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        self.rows.clear();
        for (index, row) in rows.iter().enumerate() {
            if let Some(heading) = heading(row, index, rows) {
                append_heading(&mut lines, heading);
            }
            let y = inner.y + lines.len() as u16;
            if y < inner.y + inner.height {
                self.rows.push((y, Rect::new(inner.x, y, inner.width, 1)));
            }
            lines.push(row_line(
                row,
                checked(row, relations, scope),
                index == self.cursor,
            ));
        }
        lines.push(Line::default());
        lines.push(Line::styled(
            "space toggle · 1-4 relations · r reset · esc close",
            Style::default().fg(ayu::DIM),
        ));
        lines
    }
}

fn menu_area(area: Rect, wanted: u16) -> Rect {
    let width = area.width.saturating_sub(4).clamp(12, 44);
    let height = wanted.min(area.height);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn append_heading(lines: &mut Vec<Line<'static>>, heading: &'static str) {
    if !lines.is_empty() {
        lines.push(Line::default());
    }
    lines.push(Line::styled(heading, Style::default().fg(ayu::DIM)));
}

fn heading(row: &Row, index: usize, rows: &[Row]) -> Option<&'static str> {
    let first =
        index == 0 || std::mem::discriminant(&rows[index - 1]) != std::mem::discriminant(row);
    if !first {
        return None;
    }
    Some(match row {
        Row::Relation(_) => "Relations",
        Row::State(_) => "Cards by state (none = all)",
        Row::Provider(_) => "Cards by provider (none = all)",
    })
}

fn row_line(row: &Row, checked: bool, current: bool) -> Line<'static> {
    let mark = if checked { "[x] " } else { "[ ] " };
    let label = match row {
        Row::Relation(index) => format!("{} {}", index + 1, RELATIONS[*index]),
        Row::State(state) => state.name().to_owned(),
        Row::Provider(provider) => provider.clone(),
    };
    let style = if current {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    Line::from(vec![Span::styled(format!(" {mark}{label}"), style)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn press(menu: &mut FilterMenu, code: KeyCode, relations: &mut Relations, scope: &mut Scope) {
        let providers = vec!["claude".to_owned(), "codex".to_owned()];
        menu.key(
            KeyEvent::new(code, KeyModifiers::NONE),
            &providers,
            relations,
            scope,
        );
    }

    #[test]
    fn rows_toggle_relations_states_and_providers_independently() {
        let mut menu = FilterMenu::default();
        let mut relations = Relations::default();
        let mut scope = Scope::default();
        press(&mut menu, KeyCode::Char('4'), &mut relations, &mut scope);
        assert!(relations.supervises && relations.depends);
        press(&mut menu, KeyCode::Char(' '), &mut relations, &mut scope);
        assert!(!relations.depends);
        for _ in 0..4 {
            press(&mut menu, KeyCode::Down, &mut relations, &mut scope);
        }
        press(&mut menu, KeyCode::Enter, &mut relations, &mut scope);
        assert!(scope.states.contains(&Health::Working));
        for _ in 0..20 {
            press(&mut menu, KeyCode::Down, &mut relations, &mut scope);
        }
        press(&mut menu, KeyCode::Enter, &mut relations, &mut scope);
        assert!(scope.providers.contains("codex"));
        press(&mut menu, KeyCode::Char('r'), &mut relations, &mut scope);
        assert_eq!(relations, Relations::default());
        assert!(!scope.is_filtered());
    }
}
