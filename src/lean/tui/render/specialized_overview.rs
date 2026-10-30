use super::super::*;

impl Details {
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
        draw_session_status(frame, inner, &status, theme);
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
        let input_area = self.draw_composer_status(frame, area, key, composer, active, theme);
        self.input_width[panel] = usize::from(input_area.width).max(1);
        if let Some(input) = composer.map(|composer| &composer.input) {
            self.draw_composer_input(frame, input, input_area, active, theme);
        }
    }

    fn draw_composer_status(
        &self,
        frame: &mut Frame,
        area: Rect,
        key: &str,
        composer: Option<&super::super::details::Composer>,
        active: bool,
        theme: &Theme,
    ) -> Rect {
        let status = composer_status(self, key, composer, active);
        draw_session_status(frame, area, &status, theme);
        Rect::new(
            area.x,
            area.y.saturating_add(1),
            area.width,
            area.height.saturating_sub(1),
        )
    }

    fn draw_composer_input(
        &self,
        frame: &mut Frame,
        input: &super::super::InputBuffer,
        area: Rect,
        active: bool,
        theme: &Theme,
    ) {
        let text = theme.clean(&input.text);
        let lines = wrap(&text, usize::from(area.width));
        let (column, row) = input.visual_cursor(usize::from(area.width));
        let start = row.saturating_sub(usize::from(area.height.saturating_sub(1)));
        frame.render_widget(
            Paragraph::new(
                lines
                    .into_iter()
                    .skip(start)
                    .take(usize::from(area.height))
                    .map(Line::raw)
                    .collect::<Vec<_>>(),
            ),
            area,
        );
        if active && self.editing && area.width > 0 && area.height > 0 {
            place_session_cursor(frame, area, column, row, start);
        }
    }
}

fn composer_status_area(inner: Rect) -> Rect {
    let [_, area] = Layout::vertical([Constraint::Min(1), Constraint::Length(4.min(inner.height))])
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
    let title = session_title(details.selected_round, key, active, running);
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(if active { theme.dim() } else { theme.faint() })
        .title(theme.clean(&title));
    if theme.ascii {
        block = block.border_set(ascii_border_set());
    }
    block
}

fn session_title(round: Option<u64>, key: &str, active: bool, running: bool) -> String {
    format!(
        " {}Session · {} · {} ",
        if active { "> " } else { "" },
        key.split(':').next().unwrap_or(key),
        session_round_label(round, running)
    )
}

fn ascii_border_set() -> ratatui::symbols::border::Set<'static> {
    ratatui::symbols::border::Set {
        top_left: "+",
        top_right: "+",
        bottom_left: "+",
        bottom_right: "+",
        vertical_left: "|",
        vertical_right: "|",
        horizontal_top: "-",
        horizontal_bottom: "-",
    }
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
            "Ctrl+] antislop"
        } else {
            "i terminal · a message · r retry"
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
) -> String {
    if let Some(status) = primary_composer_status(composer, active && details.editing) {
        return status.to_owned();
    }
    if let Some(error) = details
        .native
        .get(key)
        .and_then(|snapshot| snapshot.error.as_deref())
    {
        return error.to_owned();
    }
    if details.selected_round.is_some() {
        "Earlier round · o resume this session (read-only)".into()
    } else {
        "Attaching terminal… · r retry".into()
    }
}

fn primary_composer_status(
    composer: Option<&super::super::details::Composer>,
    editing: bool,
) -> Option<&str> {
    let value = composer?;
    value
        .sending
        .then_some("Sending...")
        .or(value.error.as_deref())
        .or(if editing {
            Some("Enter send · Alt+Enter newline · Esc navigation")
        } else {
            value.delivery.as_deref()
        })
}

fn place_session_cursor(frame: &mut Frame, area: Rect, column: u16, row: usize, start: usize) {
    frame.set_cursor_position((
        area.x
            .saturating_add(column.min(area.width.saturating_sub(1))),
        area.y
            .saturating_add(row.saturating_sub(start) as u16)
            .min(area.bottom().saturating_sub(1)),
    ));
}
