//! Ctrl+P palette of commands and `/` search for an agent of the graph:
//! type to narrow, arrows to choose, Enter to run, Esc to close. Also the
//! `?` help of the active view.

use super::graph::Graph;
use super::view::{View, agent_name, dim, truncate};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Accept,
    Resume,
    Cancel,
    Pause,
    Continue,
    Autonomous,
    Supervised,
    NewConversation,
    Find,
    Details,
    Diff,
    Comment,
    Filter,
    Help,
    Quit,
}

const COMMANDS: [(Command, &str); 15] = [
    (Command::Accept, "Uruchom zaproponowany plan · Ctrl+A"),
    (Command::Resume, "Wznów plan · Ctrl+R"),
    (Command::Cancel, "Anuluj trwającą pracę · Ctrl+X"),
    (
        Command::Pause,
        "Wstrzymaj nowe kroki (bieżące się dokończą) · /pause",
    ),
    (Command::Continue, "Kontynuuj wstrzymaną pracę · /continue"),
    (
        Command::Autonomous,
        "Tryb autonomiczny: plany i rundy ruszają same · /mode autonomous",
    ),
    (
        Command::Supervised,
        "Tryb nadzorowany: plany czekają na zgodę · /mode supervised",
    ),
    (Command::NewConversation, "Nowa rozmowa · Ctrl+N"),
    (Command::Find, "Znajdź agenta · /"),
    (Command::Details, "Szczegóły wybranego agenta · Enter"),
    (Command::Diff, "Diff zadania wybranego agenta · d"),
    (Command::Comment, "Skomentuj wybrany element · c"),
    (Command::Filter, "Następny filtr relacji grafu · f"),
    (Command::Help, "Pomoc · ?"),
    (Command::Quit, "Odłącz (praca trwa dalej) · q"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Run(Command),
    /// Select this graph node.
    Select(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Commands,
    Agents,
}

#[derive(Debug)]
pub struct Picker {
    pub kind: Kind,
    pub query: String,
    pub selected: usize,
}

impl Picker {
    pub fn new(kind: Kind) -> Self {
        Picker {
            kind,
            query: String::new(),
            selected: 0,
        }
    }

    /// The entries matching what was typed, each word anywhere in the label.
    pub fn items(&self, view: &View) -> Vec<(String, Action)> {
        let all: Vec<(String, Action)> = match self.kind {
            Kind::Commands => COMMANDS
                .iter()
                .map(|(command, label)| ((*label).to_owned(), Action::Run(*command)))
                .collect(),
            Kind::Agents => Graph::new(view)
                .nodes
                .iter()
                .map(|node| {
                    let task = node
                        .key
                        .split_once(':')
                        .and_then(|(_, id)| view.tasks.iter().find(|task| task.id == id));
                    let label = match task {
                        Some(task) => format!("{} · {}", agent_name(&node.key), task.title),
                        None => agent_name(&node.key),
                    };
                    (label, Action::Select(node.key.clone()))
                })
                .collect(),
        };
        let words: Vec<String> = self
            .query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        all.into_iter()
            .filter(|(label, _)| {
                let label = label.to_lowercase();
                words.iter().all(|word| label.contains(word.as_str()))
            })
            .collect()
    }

    pub fn draw(&mut self, frame: &mut Frame, view: &View) {
        let items = self.items(view);
        self.selected = self.selected.min(items.len().saturating_sub(1));
        let area = frame.area();
        let width = area.width.saturating_sub(4).min(64);
        let rows = (items.len() as u16).clamp(1, 12);
        let height = (rows + 3).min(area.height);
        let box_area = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + area.height.saturating_sub(height) / 3,
            width,
            height,
        );
        let inner = usize::from(width.saturating_sub(2));
        let start = self.selected.saturating_sub(usize::from(rows) - 1);
        let mut lines = vec![Line::raw(truncate(&format!("> {}", self.query), inner))];
        if items.is_empty() {
            lines.push(Line::styled("nic nie pasuje", dim()));
        }
        for (index, (label, _)) in items.iter().enumerate().skip(start).take(usize::from(rows)) {
            let style = if index == self.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            lines.push(Line::styled(truncate(label, inner), style));
        }
        let title = match self.kind {
            Kind::Commands => " polecenia · Enter wykonaj · Esc zamknij ",
            Kind::Agents => " znajdź agenta · Enter wybierz · Esc zamknij ",
        };
        frame.render_widget(Clear, box_area);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(title),
            ),
            box_area,
        );
        frame.set_cursor_position((
            box_area.x + 3 + self.query.chars().count() as u16,
            box_area.y + 1,
        ));
    }
}

/// Where the help was opened, for the keys it lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Help {
    Chat,
    Graph,
    Details,
    Diff,
}

impl Help {
    fn lines(self) -> &'static [&'static str] {
        match self {
            Help::Chat => &[
                "Enter            wyślij wiadomość orkiestratorowi",
                "@t2 tekst        wiadomość do workera zadania t2 (kopia dla orkiestratora)",
                "Alt+Enter, Ctrl+J nowa linia",
                "↑/↓ PgUp/PgDn End przewijanie rozmowy",
                "Tab              panel zespołu",
                "Ctrl+A / Ctrl+R  uruchom / wznów plan",
                "Ctrl+X           anuluj trwającą pracę",
                "/pause, /continue wstrzymaj nowe kroki / kontynuuj",
                "/mode autonomous|supervised  tryb dla kolejnych działań",
                "Ctrl+N           nowa rozmowa",
                "Ctrl+P           paleta poleceń",
                "Esc              wyjdź (praca trwa dalej w demonie)",
            ],
            Help::Graph => &[
                "strzałki         najbliższy węzeł w tym kierunku",
                "Enter            szczegóły agenta",
                "/                znajdź agenta",
                "d                diff zadania",
                "c                komentarz do wybranego elementu",
                "f                filtr relacji: wszystko, zależności, weryfikacja, wiadomości, nadzór",
                "Ctrl+P           paleta poleceń",
                "Tab, Esc         czat",
                "q                wyjdź (praca trwa dalej w demonie)",
            ],
            Help::Details => &[
                "←/→, Tab         zakładki",
                "↑/↓ PgUp/PgDn    przewijanie; na Komentarzach wybór",
                "c                komentarz do agenta",
                "r / o / x        rozwiąż / otwórz / usuń komentarz",
                "d                diff zadania",
                "Esc, q           powrót do grafu",
            ],
            Help::Diff => &[
                "↑/↓ PgUp/PgDn    wybór linii (lub pliku)",
                "←/→              poprzedni / następny plik",
                "Tab              lista plików / linie",
                "c                komentarz do linii lub pliku",
                "R                recenzja kodu tej zmiany",
                "r                odśwież",
                "Esc, q           powrót",
            ],
        }
    }

    pub fn draw(self, frame: &mut Frame) {
        let lines = self.lines();
        let area = frame.area();
        let width = area.width.saturating_sub(4).min(84);
        let height = (lines.len() as u16 + 2).min(area.height);
        let box_area = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + area.height.saturating_sub(height) / 3,
            width,
            height,
        );
        let inner = usize::from(width.saturating_sub(2));
        frame.render_widget(Clear, box_area);
        frame.render_widget(
            Paragraph::new(
                lines
                    .iter()
                    .map(|line| Line::raw(truncate(line, inner)))
                    .collect::<Vec<_>>(),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(" pomoc · dowolny klawisz zamyka "),
            ),
            box_area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_narrows_commands_by_every_word() {
        let view = View::default();
        let mut picker = Picker::new(Kind::Commands);
        assert_eq!(picker.items(&view).len(), COMMANDS.len());
        picker.query = "plan wzn".into();
        let items = picker.items(&view);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].1, Action::Run(Command::Resume));
    }

    #[test]
    fn agents_are_found_by_role_or_task_title() {
        let mut view = View::default();
        view.apply(
            &serde_json::json!({"seq": 1, "kind": "plan_proposed", "payload": {
            "plan": "p",
            "tasks": [{"id": "t1", "title": "Parser"}, {"id": "t2", "title": "Docs"}]}}),
        );
        let mut picker = Picker::new(Kind::Agents);
        picker.query = "pars wery".into();
        let items = picker.items(&view);
        assert_eq!(
            items,
            [(
                "t1 weryfikator · Parser".to_owned(),
                Action::Select("verifier:t1".into())
            )]
        );
    }
}
