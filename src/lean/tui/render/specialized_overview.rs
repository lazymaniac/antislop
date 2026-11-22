use super::super::*;

impl Details {
    pub(super) fn draw_workbench_terminal(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &View,
        theme: &Theme,
    ) {
        let key = self.key.clone();
        self.draw_workbench_terminal_for(frame, area, view, &key, theme);
    }

    pub(super) fn draw_workbench_terminal_for(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &View,
        key: &str,
        theme: &Theme,
    ) {
        if area.is_empty() {
            self.native_areas = [Rect::default(); 2];
            return;
        }
        let content_area = self.text_area;
        let content_scroll = self.scroll;
        frame.render_widget(Block::default().style(theme.terminal()), area);
        let [header, inner, footer] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .areas(area);
        frame.render_widget(Block::default().style(theme.raised()), header);
        let role = key.split(':').next().unwrap_or(key);
        let round = self
            .selected_round
            .or_else(|| self.task.as_ref().and_then(|task| task["round"].as_u64()));
        let owner = key
            .split_once(':')
            .and_then(|(_, id)| {
                view.tasks
                    .iter()
                    .find(|task| task.id == id || task.uuid.as_deref() == Some(id))
            })
            .map(|task| format!("{} / {role}", task.title))
            .unwrap_or_else(|| role.to_owned());
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  PTY    ", theme.text()),
                Span::styled(
                    theme.chrome(&format!("{} {owner}", if theme.ascii { ">" } else { "›" })),
                    theme.accent(),
                ),
                Span::styled(
                    theme.chrome(&format!(
                        "    round {}",
                        round.map_or_else(|| "—".into(), |round| round.to_string())
                    )),
                    theme.dim(),
                ),
            ])),
            Rect::new(header.x, header.y, header.width, 1),
        );
        let inner = inner.inner(ratatui::layout::Margin::new(
            u16::from(area.width >= 30) * 2,
            0,
        ));
        self.native_areas[0] = inner;
        frame.render_widget(
            Paragraph::new(theme.chrome("  t focus terminal    PgUp scrollback    Ctrl+] return"))
                .style(theme.dim()),
            footer,
        );
        let editing = std::mem::replace(&mut self.editing, false);
        if !self.draw_native_panel(frame, key, 0, true, inner, theme) {
            let status = self
                .native
                .get(key)
                .and_then(|snapshot| snapshot.error.as_deref())
                .map(|error| theme.clean(error))
                .unwrap_or_else(|| {
                    theme
                        .chrome("native session has not started · t input/retry · Ctrl+K direction")
                });
            frame.render_widget(Paragraph::new(status).style(theme.dim()), inner);
        }
        if let Some(error) = self.native.get(key).and_then(|snapshot| {
            snapshot.error.as_deref().or_else(|| {
                snapshot
                    .screen
                    .as_ref()
                    .and_then(|screen| screen.error.as_deref())
            })
        }) {
            frame.render_widget(
                Paragraph::new(theme.clean(error)).style(theme.warning()),
                Rect::new(
                    header.x + 2,
                    header.y + 1,
                    header.width.saturating_sub(4),
                    1,
                ),
            );
        }
        self.editing = editing;
        self.text_area = content_area;
        self.scroll = content_scroll;
        self.native_areas[1] = Rect::default();
    }

    pub(super) fn draw_overview(
        &mut self,
        frame: &mut Frame,
        view: &View,
        body: Rect,
        theme: &Theme,
    ) {
        let keys = self.session_keys();
        self.active_panel = self.active_panel.min(keys.len().saturating_sub(1));
        let areas = session_areas(&keys, body);
        for (index, area) in areas.iter().enumerate() {
            let panel = if areas.len() == 1 {
                self.active_panel
            } else {
                index
            };
            self.draw_session_panel(frame, view, &keys[panel], panel, *area, theme);
        }
    }
    fn draw_session_panel(
        &mut self,
        frame: &mut Frame,
        view: &View,
        key: &str,
        panel: usize,
        area: Rect,
        theme: &Theme,
    ) {
        let active = panel == self.active_panel;
        let running = view.agents.get(key).is_some_and(|agent| agent.running);
        let block = session_block(self, key, active, running, theme);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.native_areas[panel] = Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(1),
        );
        if self.draw_native_panel(frame, key, panel, active, inner, theme) {
            return;
        }
        self.draw_composer_panel(frame, key, panel, active, inner, theme);
    }

    fn draw_native_panel(
        &self,
        frame: &mut Frame,
        key: &str,
        panel: usize,
        active: bool,
        inner: Rect,
        theme: &Theme,
    ) -> bool {
        if self.editing {
            return false;
        }
        let Some(snapshot) = self.native.get(key) else {
            return false;
        };
        let Some(screen) = &snapshot.screen else {
            return false;
        };
        super::super::native_view::render(
            frame,
            self.native_areas[panel],
            screen,
            self.native_focus && active,
        );
        let status = native_status(self, snapshot, screen, active);
        let status = if snapshot.error.is_none() && screen.error.is_none() {
            theme.chrome(&status)
        } else {
            status
        };
        if self.native_areas[panel].bottom() < inner.bottom() {
            draw_session_status(frame, inner, &status, theme);
        }
        true
    }

    fn draw_composer_panel(
        &mut self,
        frame: &mut Frame,
        key: &str,
        panel: usize,
        active: bool,
        inner: Rect,
        theme: &Theme,
    ) {
        if active {
            self.text_area = inner;
            self.scroll = 0;
        }
        let area = composer_status_area(inner);
        let draft_key = (self.conversation.clone(), self.composer_recipient(key));
        let drafts = self.drafts.borrow();
        let composer = drafts.get(&draft_key);
        let title = format!(
            "To {} {} {} {}",
            theme.glyphs().dot,
            self.composer_recipient(key),
            theme.glyphs().dot,
            composer_mode_label(composer.map_or_else(Default::default, |composer| composer.mode))
        );
        let editor = super::composer::panel_editor_area(area);
        let wrap_width = super::composer::panel_wrap_width(area);
        self.input_width[panel] = wrap_width;
        let status = composer_status(self, key, composer, active, theme);
        let lines = composer
            .map(|composer| wrap(&theme.clean(&composer.input.text), wrap_width))
            .unwrap_or_default();
        let cursor = composer.and_then(|composer| {
            (active && self.editing && editor.width > 0 && editor.height > 0).then(|| {
                let (column, row) = composer.input.visual_cursor(wrap_width);
                let first_line = row.saturating_sub(usize::from(editor.height.saturating_sub(1)));
                (column, row, first_line)
            })
        });
        super::composer::draw_panel(
            frame,
            area,
            super::composer::Panel {
                title: &title,
                status: &status,
                lines: &lines,
                focused: active && self.editing,
                cursor,
            },
            theme,
        );
    }
}

fn composer_status_area(inner: Rect) -> Rect {
    let [_, area] = Layout::vertical([Constraint::Min(1), Constraint::Length(7.min(inner.height))])
        .areas(inner);
    area
}

fn session_areas(keys: &[String], body: Rect) -> Vec<Rect> {
    if keys.len() == 2 && body.width >= 80 {
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(body)
            .to_vec()
    } else {
        vec![body]
    }
}

fn session_block(
    details: &Details,
    key: &str,
    active: bool,
    running: bool,
    theme: &Theme,
) -> Block<'static> {
    let title = theme.chrome(&session_title(details.selected_round, key, active, running));
    Block::default()
        .borders(Borders::ALL)
        .border_set(theme.borders())
        .border_style(if active { theme.dim() } else { theme.faint() })
        .title(title)
}

fn session_title(round: Option<u64>, key: &str, active: bool, running: bool) -> String {
    format!(
        " {}Session · {} · {} ",
        if active { "> " } else { "" },
        key.split(':').next().unwrap_or(key),
        session_round_label(round, running)
    )
}

fn session_round_label(round: Option<u64>, running: bool) -> String {
    match (round, running) {
        (Some(round), _) => format!("round {round}"),
        (None, true) => "LIVE".into(),
        (None, false) => "idle".into(),
    }
}

fn native_status(
    details: &Details,
    snapshot: &super::super::native_view::Snapshot,
    screen: &crate::lean::terminal::Screen,
    active: bool,
) -> String {
    snapshot
        .error
        .as_deref()
        .or(screen.error.as_deref())
        .unwrap_or(if details.native_focus && active {
            "Ctrl+] preview"
        } else {
            "t native input · Ctrl+K message · r retry"
        })
        .to_owned()
}

fn draw_session_status(frame: &mut Frame, area: Rect, text: &str, theme: &Theme) {
    frame.render_widget(
        Paragraph::new(theme.clean(text)).style(theme.dim()),
        Rect::new(
            area.x,
            area.bottom().saturating_sub(1),
            area.width,
            1.min(area.height),
        ),
    );
}

fn composer_status(
    details: &Details,
    key: &str,
    composer: Option<&super::super::details::Composer>,
    active: bool,
    theme: &Theme,
) -> String {
    if let Some(status) = primary_composer_status(composer, active && details.editing, theme) {
        return status;
    }
    if let Some(error) = details
        .native
        .get(key)
        .and_then(|snapshot| snapshot.error.as_deref())
    {
        return error.to_owned();
    }
    if details.history_unavailable {
        return "No retained historical CLI snapshot; press o to open that earlier CLI session."
            .into();
    }
    if details.selected_round.is_some() {
        theme.chrome("Earlier round · o resume this session (read-only)")
    } else {
        theme.chrome("Attaching terminal… · r retry")
    }
}

fn primary_composer_status(
    composer: Option<&super::super::details::Composer>,
    editing: bool,
    theme: &Theme,
) -> Option<String> {
    let mode = composer.map_or_else(Default::default, |composer| composer.mode);
    if let Some(composer) = composer {
        if composer.sending {
            return Some("Sending...".into());
        }
        if let Some(error) = &composer.error {
            return Some(error.clone());
        }
    }
    if editing {
        return Some(theme.chrome(&format!(
            "Enter {} · Ctrl+T mode · Alt+Enter newline · Esc navigation",
            mode_label(mode)
        )));
    }
    composer?.delivery.clone()
}

/// What Enter does with the message in `mode`.
fn mode_label(mode: crate::lean::store::Delivery) -> &'static str {
    use crate::lean::store::Delivery;
    match mode {
        Delivery::Queue => "queue for next turn",
        Delivery::Steer => "steer the running turn",
        Delivery::Interrupt => "interrupt and steer",
    }
}

fn composer_mode_label(mode: crate::lean::store::Delivery) -> &'static str {
    use crate::lean::store::Delivery;
    match mode {
        Delivery::Queue => "Queue",
        Delivery::Steer => "Steer",
        Delivery::Interrupt => "Interrupt",
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    use crate::lean::store::Delivery;

    #[test]
    fn terminal_preview_preserves_content_scroll_and_excludes_inline_composer() {
        use ratatui::{Terminal, backend::TestBackend};
        for theme in [Theme::new(true, false), Theme::new(false, true)] {
            let mut details = Details::new("orchestrator".into());
            let content = Rect::new(2, 2, 40, 8);
            details.text_area = content;
            details.scroll = 17;
            let area = Rect::new(0, 12, 60, 10);
            let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
            terminal
                .draw(|frame| {
                    details.draw_workbench_terminal(frame, area, &View::default(), &theme);
                })
                .unwrap();
            assert_eq!(details.text_area, content);
            assert_eq!(details.scroll, 17);
            let inner = session_block(&details, "orchestrator", true, false, &theme).inner(area);
            assert_eq!(details.native_areas[0].width, inner.width.saturating_sub(2));
            let text = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(text.contains("native session has not started"));
            assert!(!text.contains("To "));
            assert_eq!(terminal.backend().buffer()[(0, 12)].symbol(), " ");
        }
    }

    #[test]
    fn the_composer_says_how_enter_delivers_the_message() {
        let mut composer = super::super::super::details::Composer::default();
        let hint = |composer: &super::super::super::details::Composer| {
            primary_composer_status(Some(composer), true, &Theme::new(true, false)).unwrap()
        };
        assert!(hint(&composer).starts_with("Enter queue for next turn · Ctrl+T mode"));
        composer.mode = Delivery::Interrupt;
        assert!(hint(&composer).starts_with("Enter interrupt and steer"));
        assert_eq!(
            primary_composer_status(None, true, &Theme::new(true, false)).as_deref(),
            Some("Enter queue for next turn · Ctrl+T mode · Alt+Enter newline · Esc navigation")
        );
        composer.delivery = Some("steered into the running turn".into());
        assert_eq!(
            primary_composer_status(Some(&composer), false, &Theme::new(true, false)).as_deref(),
            Some("steered into the running turn")
        );
    }
}
