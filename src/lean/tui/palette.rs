//! Ctrl+P palette of commands, `/` search for an agent of the graph and
//! Ctrl+F search of the conversation: type to narrow, arrows to choose,
//! Enter to run, Esc to close. Also the `?` help of the active view.

use super::graph::Graph;
use super::view::{View, agent_name, dim, truncate};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

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
    SwitchConversation,
    Tasks,
    Find,
    Search,
    Details,
    Diff,
    Comment,
    Filter,
    Help,
    Quit,
}

const COMMANDS: [(Command, &str); 18] = [
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
    (Command::SwitchConversation, "Przełącz rozmowę · Ctrl+O"),
    (Command::Tasks, "Zadania (karuzela) · t"),
    (Command::Find, "Znajdź agenta · /"),
    (Command::Search, "Szukaj w rozmowie · Ctrl+F"),
    (Command::Details, "Szczegóły wybranego agenta · Enter"),
    (Command::Diff, "Diff zadania wybranego agenta · d"),
    (Command::Comment, "Skomentuj wybrany element · c"),
    (Command::Filter, "Następny filtr relacji grafu · f"),
    (Command::Help, "Pomoc · ?"),
    (Command::Quit, "Odłącz (praca trwa dalej) · q"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationItem {
    pub id: String,
    pub project: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Run(Command),
    /// Select this graph node.
    Select(String),
    /// Show this entry of the conversation.
    Show(usize),
    /// Switch to another conversation.
    SwitchConversation(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Commands,
    Agents,
    History,
    Conversations,
}

#[derive(Debug)]
pub struct Picker {
    pub kind: Kind,
    pub query: String,
    pub selected: usize,
    pub conversations: Vec<ConversationItem>,
}

impl Picker {
    pub fn new(kind: Kind) -> Self {
        Picker {
            kind,
            query: String::new(),
            selected: 0,
            conversations: Vec::new(),
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
                .filter(|node| node.visible)
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
            // Newest first.
            Kind::History => view
                .transcript
                .iter()
                .enumerate()
                .rev()
                .filter(|(_, entry)| !entry.text.trim().is_empty())
                .map(|(index, entry)| (entry.text.trim().replace('\n', " "), Action::Show(index)))
                .collect(),
            Kind::Conversations => self
                .conversations
                .iter()
                .map(|c| (c.title.clone(), Action::SwitchConversation(c.id.clone())))
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
        self.draw_with_cursor(frame, view, true);
    }

    pub(super) fn draw_with_cursor(&mut self, frame: &mut Frame, view: &View, show_cursor: bool) {
        let items = self.items(view);
        self.selected = self.selected.min(items.len().saturating_sub(1));
        let area = frame.area();
        if area.width == 0 || area.height == 0 {
            return;
        }
        let width = if area.width >= 8 {
            area.width.saturating_sub(4).min(64)
        } else {
            area.width
        };
        let rows = items.len().clamp(1, 12) as u16;
        let height = (rows + 3).min(area.height).max(1);
        let box_area = Rect::new(
            area.x + (area.width.saturating_sub(width)) / 2,
            area.y + area.height.saturating_sub(height) / 3,
            width,
            height,
        );
        let inner = usize::from(width.saturating_sub(2));
        let visible_rows = usize::from(height.saturating_sub(3)).max(1);
        let start = if self.selected >= visible_rows {
            self.selected + 1 - visible_rows
        } else {
            0
        };
        let mut lines = vec![Line::raw(truncate(&format!("> {}", self.query), inner))];
        if items.is_empty() {
            let empty_msg = if self.query.is_empty() {
                match self.kind {
                    Kind::Commands => "Brak dostępnych poleceń.",
                    Kind::Agents => "Brak agentów w grafie.",
                    Kind::History => "Brak wpisów w historii rozmowy.",
                    Kind::Conversations => "Brak innych rozmów w tym projekcie.",
                }
            } else {
                "Nic nie pasuje do zapytania."
            };
            lines.push(Line::styled(truncate(empty_msg, inner), dim()));
        }
        for (index, (label, _)) in items.iter().enumerate().skip(start).take(visible_rows) {
            let style = if index == self.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            lines.push(Line::styled(truncate(label, inner), style));
        }
        let title = match self.kind {
            Kind::Commands if width >= 42 => " polecenia · Enter wykonaj · Esc zamknij ",
            Kind::Commands if width >= 20 => " polecenia · Esc ",
            Kind::Commands => " polecenia ",
            Kind::Agents if width >= 45 => " znajdź agenta · Enter wybierz · Esc zamknij ",
            Kind::Agents if width >= 20 => " agenci · Esc ",
            Kind::Agents => " agenci ",
            Kind::History if width >= 45 => " szukaj w rozmowie · Enter pokaż · Esc zamknij ",
            Kind::History if width >= 20 => " szukaj · Esc ",
            Kind::History => " szukaj ",
            Kind::Conversations if width >= 45 => " przełącz rozmowę · Enter otwórz · Esc zamknij ",
            Kind::Conversations if width >= 20 => " rozmowy · Esc ",
            Kind::Conversations => " rozmowy ",
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
        if show_cursor && box_area.width >= 3 && box_area.height >= 2 {
            let query_display_width = self.query.width();
            let prefix_width = 2;
            let max_col = inner.saturating_sub(1);
            let col = (prefix_width + query_display_width).min(max_col) as u16;
            frame.set_cursor_position((box_area.x + 1 + col, box_area.y + 1));
        }
    }
}

/// Where the help was opened, for the keys it lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Help {
    Chat,
    Graph,
    Tasks,
    Details,
    Diff,
    Comment,
    Picker,
}

impl Help {
    fn lines(self) -> &'static [&'static str] {
        match self {
            Help::Chat => &[
                "Enter, Ctrl+S    wyślij wiadomość orkiestratorowi",
                "@t2 tekst        wiadomość do workera t2 (kopia w czacie)",
                "Alt+Enter, Ctrl+J  nowa linia w szkicu",
                "↑/↓              wiersz w szkicu; historia wysłanych na skrajach",
                "PgUp/PgDn        przewijanie rozmowy",
                "Tab              panel zespołu",
                "Ctrl+A / Ctrl+R  uruchom/wznów (na pustym); Home/szukaj",
                "Ctrl+U / Ctrl+K  usuń do początku / końca linii",
                "Ctrl+W           usuń słowo w lewo",
                "Ctrl+E           przejdź na koniec linii",
                "Ctrl+X           anuluj trwającą pracę",
                "/pause, /continue  wstrzymaj nowe kroki / kontynuuj",
                "/mode autonomous   tryb autonomiczny dla kolejnych planów",
                "/mode supervised   tryb nadzorowany dla kolejnych planów",
                "Ctrl+N / Ctrl+O  nowa / przełącz rozmowę",
                "Ctrl+F           szukaj w rozmowie",
                "Ctrl+P           paleta poleceń",
                "Esc              czyść szkic; wyjdź gdy pusty (demon trwa)",
            ],
            Help::Graph => &[
                "strzałki         najbliższy węzeł w tym kierunku",
                "Enter            szczegóły agenta",
                "/                znajdź agenta",
                "t                karuzela zadań",
                "d                diff zadania",
                "c                komentarz do wybranego elementu",
                "z, Spacja        zwiń / rozwiń zadanie",
                "g                przełącz grupowanie (zależności / stan)",
                "f                filtr relacji: wszystko, zależności, weryfikacja, wiadomości, nadzór",
                "Ctrl+P           paleta poleceń",
                "Tab, Esc         czat",
                "q                wyjdź (praca trwa dalej w demonie)",
            ],
            Help::Tasks => &[
                "←/→, ↑/↓        poprzednie / następne zadanie w karuzeli",
                "w                przejdź do workera na grafie",
                "v                przejdź do weryfikatora na grafie",
                "Enter            szczegóły wybranego zadania",
                "d                diff zadania",
                "c                komentarz do zadania",
                "Ctrl+P           paleta poleceń",
                "Tab              przełącz na zespół",
                "Esc              przełącz na czat",
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
            Help::Comment => &[
                "Enter, Ctrl+S    wyślij komentarz",
                "Alt+Enter, Ctrl+J  nowa linia w komentarzu",
                "↑/↓              wiersz w komentarzu",
                "←/→              znak w lewo / prawo",
                "Alt/Ctrl+←/→     słowo w lewo / prawo",
                "Home/End         początek / koniec linii",
                "Alt/Ctrl+Home/End  początek / koniec komentarza",
                "Ctrl+A / Ctrl+E  początek / koniec linii",
                "Ctrl+U / Ctrl+K  usuń do początku / końca linii",
                "Ctrl+W, Alt+Backspace  usuń słowo w lewo",
                "Esc              anuluj komentarz (szkic czatu zostaje)",
            ],
            Help::Picker => &[
                "tekst            filtruj listę według wpisanych słów",
                "Backspace        usuń ostatni znak zapytania",
                "↑/↓              wybierz pozycję z listy",
                "Enter            zatwierdź wybraną pozycję",
                "Esc              zamknij listę (szkic czatu zostaje)",
            ],
        }
    }

    pub fn lines_for_width(self, inner: usize) -> Vec<String> {
        let lines = self.lines();
        let mut out = Vec::new();
        for &line in lines {
            if inner < 60 {
                let compact = compact_help_line(line);
                if compact.chars().count() <= inner {
                    out.push(compact);
                } else if let Some((key, desc)) = compact.split_once(": ") {
                    let prefix = format!("{key}: ");
                    let indent = "   ";
                    let mut cur = prefix;
                    for word in desc.split_whitespace() {
                        let cur_len = cur.chars().count();
                        let word_len = word.chars().count();
                        let needed = if cur.ends_with(": ") || cur.ends_with("   ") {
                            word_len
                        } else {
                            word_len + 1
                        };
                        if cur_len + needed > inner && cur_len > 3 {
                            out.push(cur);
                            cur = format!("{indent}{word}");
                        } else {
                            if !cur.ends_with(": ") && !cur.ends_with("   ") {
                                cur.push(' ');
                            }
                            cur.push_str(word);
                        }
                    }
                    if !cur.is_empty() {
                        out.push(cur);
                    }
                } else {
                    out.push(truncate(&compact, inner));
                }
            } else if line.chars().count() <= inner {
                out.push(line.to_string());
            } else {
                out.push(truncate(line, inner));
            }
        }
        out
    }

    pub fn max_scroll(self, area: Rect) -> usize {
        if area.width == 0 || area.height == 0 {
            return 0;
        }
        let width = if area.width >= 8 {
            area.width.saturating_sub(2).min(84)
        } else {
            area.width
        };
        let inner = usize::from(width.saturating_sub(2));
        let lines = self.lines_for_width(inner);
        let height = (lines.len() as u16 + 2).min(area.height).max(1);
        let visible_lines = usize::from(height.saturating_sub(2));
        lines.len().saturating_sub(visible_lines)
    }

    pub fn draw(self, frame: &mut Frame) {
        self.draw_scroll(frame, 0);
    }

    pub fn draw_scroll(self, frame: &mut Frame, scroll: usize) {
        let area = frame.area();
        if area.width == 0 || area.height == 0 {
            return;
        }
        let width = if area.width >= 8 {
            area.width.saturating_sub(2).min(84)
        } else {
            area.width
        };
        let inner = usize::from(width.saturating_sub(2));
        let lines = self.lines_for_width(inner);
        let height = (lines.len() as u16 + 2).min(area.height).max(1);
        let box_area = Rect::new(
            area.x + (area.width.saturating_sub(width)) / 2,
            area.y + area.height.saturating_sub(height) / 3,
            width,
            height,
        );
        let visible_lines = usize::from(height.saturating_sub(2));
        let max_scroll = lines.len().saturating_sub(visible_lines);
        let scroll = scroll.min(max_scroll);
        let title = if max_scroll > 0 {
            if width >= 44 {
                " pomoc · ↑/↓ przewijaj · Esc zamyka "
            } else if width >= 26 {
                " pomoc · ↑/↓ · Esc "
            } else {
                " pomoc "
            }
        } else if width >= 36 {
            " pomoc · Esc zamyka "
        } else if width >= 14 {
            " pomoc · Esc "
        } else {
            " pomoc "
        };
        let shown_lines: Vec<Line> = lines
            .iter()
            .skip(scroll)
            .take(visible_lines)
            .map(|line| Line::raw(line.clone()))
            .collect();
        frame.render_widget(Clear, box_area);
        frame.render_widget(
            Paragraph::new(shown_lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(title),
            ),
            box_area,
        );
    }
}

fn compact_help_line(line: &str) -> String {
    let mut parts = line.splitn(2, "  ");
    if let (Some(key), Some(desc)) = (parts.next(), parts.next()) {
        let key = key.trim();
        let desc = desc.trim();
        if !key.is_empty() && !desc.is_empty() {
            return format!("{key}: {desc}");
        }
    }
    line.to_string()
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
    fn the_conversation_is_searched_newest_first() {
        let mut view = View::default();
        for (seq, text) in [(1, "plan importu"), (2, "inne"), (3, "Import gotowy")] {
            view.apply(&serde_json::json!({"seq": seq, "kind": "message",
                "payload": {"author": "orchestrator", "text": text}}));
        }
        let mut picker = Picker::new(Kind::History);
        picker.query = "import".into();
        let shown: Vec<Action> = picker
            .items(&view)
            .into_iter()
            .map(|(_, action)| action)
            .collect();
        assert_eq!(shown.len(), 2);
        let Action::Show(newest) = shown[0] else {
            panic!("{shown:?}")
        };
        assert!(view.transcript[newest].text.contains("Import gotowy"));
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

    #[test]
    fn picker_cursor_bounded_and_display_width_aware() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let view = View::default();
        let mut picker = Picker::new(Kind::Commands);

        // 1. Standard ASCII query
        picker.query = "pla".into();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                picker.draw(frame, &view);
            })
            .unwrap();
        // Box width is min(80-4, 64) = 64. Left x is (80-64)/2 = 8.
        // Inner cursor: prefix "> " (2 cols) + "pla" (3 cols) = 5 cols from box_area.x + 1.
        // So cursor_x should be 8 + 1 + 5 = 14.
        let (cursor_x, cursor_y) = terminal.get_cursor_position().unwrap().into();
        assert_eq!(cursor_x, 14);
        assert_eq!(cursor_y, 6);

        // 2. Double-width CJK character takes 2 visual columns
        picker.query = "漢".into();
        terminal
            .draw(|frame| {
                picker.draw(frame, &view);
            })
            .unwrap();
        let (cursor_x, _) = terminal.get_cursor_position().unwrap().into();
        // "> " (2) + "漢" (2) = 4 cols -> 8 + 1 + 4 = 13
        assert_eq!(cursor_x, 13);

        // 3. Overflowing long query: cursor must be bounded within box interior!
        picker.query = "a".repeat(200);
        terminal
            .draw(|frame| {
                picker.draw(frame, &view);
            })
            .unwrap();
        let (cursor_x, _) = terminal.get_cursor_position().unwrap().into();
        // Inner width is 64 - 2 = 62. Max col is 61.
        // Bounded cursor_x should be 8 + 1 + 61 = 70.
        // Box right is 8 + 64 = 72, interior right is 71. Cursor <= 70 is safely inside border!
        assert_eq!(cursor_x, 70);
        assert!(cursor_x < 8 + 64 - 1);
    }

    #[test]
    fn picker_and_help_render_deterministically_on_narrow_and_tiny_terminals() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let view = View::default();
        let mut picker = Picker::new(Kind::Commands);

        // Narrow terminal: 40x12
        let backend = TestBackend::new(40, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                picker.draw(frame, &view);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        assert!(buffer.area.width == 40 && buffer.area.height == 12);

        // Tiny terminal: 10x4
        let backend = TestBackend::new(10, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                picker.draw(frame, &view);
            })
            .unwrap();

        // Extreme tiny terminal: 2x2 (should not panic)
        let backend = TestBackend::new(2, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                picker.draw(frame, &view);
                Help::Chat.draw(frame);
            })
            .unwrap();
    }

    #[test]
    fn picker_empty_state_and_narrow_title() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let view = View::default();
        let mut picker = Picker::new(Kind::History);
        assert!(picker.items(&view).is_empty());

        let backend = TestBackend::new(60, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                picker.draw(frame, &view);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let rendered: String = buffer.content.iter().map(|c| c.symbol()).collect();
        // Polish empty state for history
        assert!(
            rendered.contains("Brak wpisów w historii rozmowy") || rendered.contains("Brak wpisów")
        );
    }

    #[test]
    fn picker_at_60x12_keeps_selected_item_visible_when_items_len_ge_10() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let view = View::default();
        let mut picker = Picker::new(Kind::Commands);
        let items = picker.items(&view);
        assert!(items.len() >= 10, "Commands should have >= 10 items");

        // Select the last item (e.g. index 11)
        picker.selected = items.len() - 1;

        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                picker.draw(frame, &view);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        // Verify that the selected item is rendered with REVERSED modifier
        let mut found_reversed_selected = false;
        let last_item_label = &items[picker.selected].0;
        let last_word = last_item_label.split_whitespace().next().unwrap();

        for y in 0..12 {
            let row_chars: String = (0..60).map(|x| buffer[(x, y)].symbol()).collect();
            if row_chars.contains(last_word) {
                for x in 0..60 {
                    let cell = &buffer[(x, y)];
                    if cell.modifier.contains(Modifier::REVERSED) {
                        found_reversed_selected = true;
                        break;
                    }
                }
            }
        }
        assert!(
            found_reversed_selected,
            "Selected item at index {} must be visible with REVERSED modifier at 60x12",
            picker.selected
        );
    }

    #[test]
    fn help_at_60x12_renders_all_controls_and_documents_shortcuts() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        // 1. Initial page (scroll = 0): verify first set of shortcuts and full descriptions
        terminal
            .draw(|frame| {
                Help::Chat.draw(frame);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let rendered: String = buffer.content.iter().map(|c| c.symbol()).collect();

        // Check shortcuts and their complete descriptions
        assert!(rendered.contains("wyślij wiadomość orkiestratorowi"));
        assert!(rendered.contains("nowa linia w szkicu"));
        assert!(rendered.contains("wiersz w szkicu"));
        assert!(rendered.contains("przewijanie rozmowy"));
        assert!(rendered.contains("uruchom/wznów"));
        assert!(rendered.contains("usuń do początku / końca"));

        // 2. Scrolled page: verify subsequent shortcuts and descriptions
        terminal
            .draw(|frame| {
                Help::Chat.draw_scroll(frame, 8);
            })
            .unwrap();

        let buffer2 = terminal.backend().buffer();
        let rendered2: String = buffer2.content.iter().map(|c| c.symbol()).collect();

        assert!(rendered2.contains("anuluj trwającą pracę"));
        assert!(rendered2.contains("wstrzymaj nowe kroki / kontynuuj"));
        assert!(rendered2.contains("/mode autonomous"));
        assert!(rendered2.contains("/mode supervised"));
        assert!(rendered2.contains("szukaj w rozmowie"));
        assert!(rendered2.contains("paleta poleceń"));
        assert!(rendered2.contains("czyść szkic; wyjdź gdy pusty"));

        // 3. Graph help at 60x12: verify long filter relation description is fully accessible without clipping
        terminal
            .draw(|frame| {
                Help::Graph.draw(frame);
            })
            .unwrap();

        let buffer_graph = terminal.backend().buffer();
        let rendered_graph: String = buffer_graph.content.iter().map(|c| c.symbol()).collect();
        assert!(rendered_graph.contains("filtr relacji"));
        assert!(rendered_graph.contains("wszystko"));
        assert!(rendered_graph.contains("zależności"));
        assert!(rendered_graph.contains("weryfikacja"));
        assert!(rendered_graph.contains("wiadomości"));
        assert!(rendered_graph.contains("nadzór"));
    }
}
