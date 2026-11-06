use super::super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl App {
    pub(in crate::lean::tui) fn graph_key(&mut self, key: KeyEvent) -> bool {
        self.selected = graph::card_key(&self.view, &self.selected);
        if self.graph_expanded.is_some()
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char(',' | '.'))
        {
            let before = self
                .details
                .as_ref()
                .and_then(|details| details.selected_round);
            if let Some(details) = &mut self.details {
                details.change_round(
                    &self.view,
                    if key.code == KeyCode::Char(',') {
                        -1
                    } else {
                        1
                    },
                );
            }
            if self
                .details
                .as_ref()
                .and_then(|details| details.selected_round)
                != before
            {
                self.fetch_task();
                if self.graph_tab == 3 {
                    self.open_graph_diff();
                }
            }
            return true;
        }
        if self.console_key(key) {
            return true;
        }
        let graph = self.graph_for_key();
        if let Some(handled) = self.graph_scroll_key(key, &graph) {
            return handled;
        }
        if let Some((dx, dy)) = graph_direction(key.code) {
            self.move_graph_selection(&graph, dx, dy);
            return true;
        }
        self.graph_action_key(key)
    }

    /// The expanded card's PTY tab is a read-only console until `i` enters it:
    /// the arrows and page keys scroll its history.
    fn console_key(&mut self, key: KeyEvent) -> bool {
        let console = self.graph_expanded.is_some()
            && self.graph_tab == 1
            && self
                .details
                .as_ref()
                .is_some_and(|details| details.selected_round.is_none() && !details.native_focus);
        if !console {
            return false;
        }
        let height = self.details.as_ref().map_or(10, |details| {
            i32::from(details.native_areas[0].height.max(2))
        });
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let lines = match key.code {
            KeyCode::Up => 1,
            KeyCode::Down => -1,
            KeyCode::PageUp => height - 1,
            KeyCode::PageDown => 1 - height,
            KeyCode::Char('u') if control => height / 2,
            KeyCode::Char('d') if control => -(height / 2),
            KeyCode::Char('i') if !control => {
                if let Some(details) = &mut self.details {
                    details.native_focus = true;
                }
                return true;
            }
            _ => return false,
        };
        self.scroll_console(lines);
        true
    }

    pub(in crate::lean::tui) fn scroll_console(&mut self, lines: i32) {
        self.native_send(crate::lean::terminal::Input::Scroll { lines });
    }

    fn graph_for_key(&self) -> Graph {
        Graph::new_with_expanded(
            &self.view,
            &self.graph_scope,
            &self.graph_collapsed,
            self.graph_grouping,
            self.graph_viewport,
            self.graph_expanded.clone(),
            self.graph_tab,
        )
    }

    fn graph_scroll_key(&mut self, key: KeyEvent, graph: &Graph) -> Option<bool> {
        let down = match key.code {
            KeyCode::PageDown => Some(true),
            KeyCode::PageUp => Some(false),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(true),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(false),
            _ => None,
        }?;
        let height = self.graph_viewport.map_or(10, |(_, height)| height);
        let amount = if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
            height.saturating_sub(2).max(1)
        } else {
            (height / 2).max(1)
        };
        self.pan_graph(graph, down, amount, false);
        Some(true)
    }

    fn move_graph_selection(&mut self, graph: &Graph, dx: i32, dy: i32) {
        if !graph.node(&self.selected).is_some_and(|node| node.visible) {
            self.select_visible_graph_node(graph);
        } else if let Some(next) = graph.neighbour(&self.selected, dx, dy) {
            self.selected = next;
        }
    }

    fn select_visible_graph_node(&mut self, graph: &Graph) {
        let worker = self
            .selected
            .split_once(':')
            .map(|(_, id)| format!("worker:{id}"));
        self.selected = worker
            .and_then(|key| visible_graph_key(graph, &key))
            .or_else(|| visible_graph_key(graph, ORCHESTRATOR))
            .or_else(|| {
                graph
                    .nodes
                    .iter()
                    .find(|node| node.visible)
                    .map(|node| node.key.clone())
            })
            .unwrap_or_else(|| ORCHESTRATOR.into());
    }

    fn graph_action_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => self.focus = Focus::Chat,
            KeyCode::Char('q') => return false,
            KeyCode::Char('?') => self.show_help(Help::Graph),
            KeyCode::Char('/') => self.picker = Some(Picker::new(Kind::Agents)),
            KeyCode::Char('p') => self.run_workflow(Command::Plan),
            KeyCode::Char('u') => self.open_unblock(None),
            KeyCode::Char('z' | ' ') => self.toggle_selected_collapse(),
            KeyCode::Char('g') => self.graph_grouping = self.graph_grouping.next(),
            KeyCode::Char('f') => self.open_graph_filters(),
            KeyCode::Char(digit @ '1'..='4') => {
                if self.graph_expanded.is_some() {
                    self.graph_tab = usize::from(digit as u8 - b'1');
                    if let Some(details) = &mut self.details {
                        // The console opens read-only; `i` types into it.
                        details.native_focus = false;
                    }
                    if self.graph_tab == 3 {
                        self.open_graph_diff();
                    }
                } else {
                    self.relations.toggle(usize::from(digit as u8 - b'1'));
                }
            }
            KeyCode::Char('d') => {
                self.graph_tab = 3;
                self.open_graph_diff();
            }
            KeyCode::Char('c') => self.open_graph_comment(),
            KeyCode::Enter => {
                if self.graph_expanded.as_deref() == Some(self.selected.as_str()) {
                    let key = self.graph_expanded.take();
                    self.graph_anim = key
                        .filter(|_| self.animate_graph)
                        .map(|key| (key, false, std::time::Instant::now()));
                    self.details = None;
                } else {
                    self.graph_expanded = Some(self.selected.clone());
                    self.graph_anim = self
                        .animate_graph
                        .then(|| (self.selected.clone(), true, std::time::Instant::now()));
                    self.graph_revealed = None;
                    self.graph_tab = 0;
                    self.open_details(self.selected.clone());
                }
            }
            _ => {}
        }
        true
    }

    fn open_graph_comment(&mut self) {
        let target = self.selected.clone();
        let input = self.get_comment_draft(&target);
        self.editor = Some(Editor { target, input });
    }

    fn open_graph_diff(&mut self) {
        let key = self
            .graph_expanded
            .as_deref()
            .unwrap_or(&self.selected)
            .to_owned();
        let historical = self
            .details
            .as_ref()
            .is_some_and(|details| details.selected_round.is_some());
        if historical {
            if let Some((_, task)) = key.split_once(':') {
                self.diff = Some(DiffView::new(task.to_owned()));
            }
        } else {
            self.open_diff(&key);
        }
    }
}

fn graph_direction(code: KeyCode) -> Option<(i32, i32)> {
    match code {
        KeyCode::Left => Some((-1, 0)),
        KeyCode::Right => Some((1, 0)),
        KeyCode::Up => Some((0, -1)),
        KeyCode::Down => Some((0, 1)),
        _ => None,
    }
}

fn visible_graph_key(graph: &Graph, key: &str) -> Option<String> {
    graph
        .node(key)
        .filter(|node| node.visible)
        .map(|node| node.key.clone())
}
