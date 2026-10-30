use super::super::*;

#[derive(Default)]
struct TranscriptRows {
    extra: Vec<Row>,
    history_height: usize,
    found_top: Option<usize>,
}

struct ViewportRows {
    total: usize,
    start: usize,
    height: usize,
}

impl App {
    pub(crate) fn draw_transcript(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let width = area.width.saturating_sub(1);
        let mut rows = self.prepare_transcript_rows(width, theme);
        self.add_workflow_card(&mut rows.extra, width, theme);
        self.add_legacy_transcript(&mut rows, width, theme);
        self.add_empty_transcript(&mut rows.extra, rows.history_height, width, theme);
        self.draw_transcript_viewport(frame, area, rows, theme);
    }

    fn prepare_transcript_rows(&mut self, width: u16, theme: &Theme) -> TranscriptRows {
        self.sync_transcript_context(width, theme);
        let mut rows = TranscriptRows::default();
        for index in 0..self.view.cards.len() {
            self.update_transcript_entry(index, width, theme, &mut rows);
        }
        rows
    }

    fn sync_transcript_context(&mut self, width: u16, theme: &Theme) {
        let context = (self.conversation.clone(), width, *theme);
        if self.transcript_layout.context.as_ref() != Some(&context) {
            self.transcript_layout.entries.clear();
            self.transcript_layout.context = Some(context);
        }
        self.transcript_layout
            .entries
            .truncate(self.view.cards.len());
    }

    fn update_transcript_entry(
        &mut self,
        index: usize,
        width: u16,
        theme: &Theme,
        rows: &mut TranscriptRows,
    ) {
        let card_entry = self.view.cards[index].clone();
        let found = self.found == Some(index);
        if found {
            rows.found_top = Some(rows.history_height);
        }
        let comment = transcript_comment(self, &card_entry);
        if self.reuse_transcript_entry(index, &card_entry, &comment, found, rows) {
            return;
        }
        let card_rows = self.transcript_card_rows(&card_entry, found, width, theme);
        self.store_transcript_entry(index, card_entry, comment, found, card_rows, rows);
    }

    fn store_transcript_entry(
        &mut self,
        index: usize,
        source: TranscriptCard,
        comment: Option<(String, Option<String>)>,
        found: bool,
        card_rows: Vec<Row>,
        rows: &mut TranscriptRows,
    ) {
        rows.history_height += card_rows.len();
        let entry = TranscriptLayoutEntry {
            source,
            comment,
            found,
            rows: card_rows,
            end: rows.history_height,
        };
        if index < self.transcript_layout.entries.len() {
            self.transcript_layout.entries[index] = entry;
        } else {
            self.transcript_layout.entries.push(entry);
        }
    }

    fn reuse_transcript_entry(
        &mut self,
        index: usize,
        source: &TranscriptCard,
        comment: &Option<(String, Option<String>)>,
        found: bool,
        rows: &mut TranscriptRows,
    ) -> bool {
        let Some(cached) = self.transcript_layout.entries.get_mut(index) else {
            return false;
        };
        if cached.source != *source || cached.comment != *comment || cached.found != found {
            return false;
        }
        rows.history_height += cached.rows.len();
        cached.end = rows.history_height;
        true
    }

    fn add_workflow_card(&self, cards: &mut Vec<Row>, width: u16, theme: &Theme) {
        let Some((title, body)) = workflow_card_text(&self.view) else {
            return;
        };
        cards.extend(card(
            theme,
            CardProps {
                fill: theme.card(),
                accent: theme.accent(),
                head: vec![Span::styled(title, theme.title())],
                right: String::new(),
                body: &body,
                max_body: usize::MAX,
                width,
            },
        ));
    }

    fn add_legacy_transcript(&self, rows: &mut TranscriptRows, width: u16, theme: &Theme) {
        if !self.view.cards.is_empty() || !rows.extra.is_empty() || self.view.transcript.is_empty()
        {
            return;
        }
        for (index, entry) in self.view.transcript.iter().enumerate() {
            let style = if self.found == Some(index) {
                rows.found_top = Some(rows.extra.len());
                entry.style.add_modifier(Modifier::REVERSED)
            } else {
                entry.style
            };
            for line in wrap(&entry.text, usize::from(width)) {
                rows.extra.push(Row::new(
                    theme.card(),
                    vec![Span::styled(theme.clean(&line), style)],
                ));
            }
        }
    }

    fn add_empty_transcript(
        &self,
        cards: &mut Vec<Row>,
        history_height: usize,
        width: u16,
        theme: &Theme,
    ) {
        if history_height != 0 || !cards.is_empty() {
            return;
        }
        cards.extend(card(
            theme,
            CardProps {
                fill: theme.card(),
                accent: theme.accent(),
                head: vec![Span::styled(
                    "No messages in this conversation.",
                    theme.title(),
                )],
                right: String::new(),
                body: "Type a message in the composer below or press Ctrl+P to open the palette.",
                max_body: usize::MAX,
                width,
            },
        ));
    }

    fn draw_transcript_viewport(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        rows: TranscriptRows,
        theme: &Theme,
    ) {
        let total = rows.history_height + rows.extra.len();
        let height = usize::from(area.height);
        self.position_transcript_scroll(total, height, rows.found_top);
        let (start, end) = visible_rows(total, self.scroll, height);
        let visible = self.transcript_visible_rows(start, end, rows.history_height, &rows.extra);
        let position = ViewportRows {
            total,
            start,
            height,
        };
        self.paint_transcript_rows(frame, area, &visible, position, theme);
    }

    fn position_transcript_scroll(
        &mut self,
        total: usize,
        height: usize,
        found_top: Option<usize>,
    ) {
        if let Some(top) = found_top.filter(|_| self.scroll == usize::MAX) {
            self.scroll = total.saturating_sub(top + height);
        }
        self.scroll = self.scroll.min(total.saturating_sub(height));
    }

    fn transcript_visible_rows(
        &self,
        start: usize,
        end: usize,
        history_height: usize,
        extra: &[Row],
    ) -> Vec<Row> {
        let mut visible = Vec::with_capacity(end.saturating_sub(start));
        let first = self
            .transcript_layout
            .entries
            .partition_point(|entry| entry.end <= start);
        for entry in &self.transcript_layout.entries[first..] {
            let origin = entry.end - entry.rows.len();
            if origin >= end {
                break;
            }
            let from = start.saturating_sub(origin);
            let to = end.saturating_sub(origin).min(entry.rows.len());
            visible.extend_from_slice(&entry.rows[from..to]);
        }
        let from = start.saturating_sub(history_height).min(extra.len());
        let to = end.saturating_sub(history_height).min(extra.len());
        visible.extend_from_slice(&extra[from..to]);
        visible
    }

    fn paint_transcript_rows(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        visible: &[Row],
        position: ViewportRows,
        theme: &Theme,
    ) {
        let body = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
        draw_rows(frame.buffer_mut(), body, visible);
        self.text_area = Some(body);
        self.text_origin = position.start;
        let scroll_area = Rect::new(area.right().saturating_sub(1), area.y, 1, area.height);
        scrollbar(
            frame.buffer_mut(),
            theme,
            scroll_area,
            position.total,
            position.start,
            position.height,
        );
    }
}

fn visible_rows(total: usize, scroll: usize, height: usize) -> (usize, usize) {
    let end = total.saturating_sub(scroll);
    (end.saturating_sub(height), end)
}

fn transcript_comment(app: &App, entry: &TranscriptCard) -> Option<(String, Option<String>)> {
    let TranscriptCard::Comment { id, status, .. } = entry else {
        return None;
    };
    let state = app
        .view
        .comments
        .get(id)
        .map_or(status.as_str(), |_| app.view.comment_state(*id))
        .to_owned();
    Some((state, app.view.comment_error(*id).map(str::to_owned)))
}

fn workflow_card_text(view: &View) -> Option<(&'static str, String)> {
    view.workflow_phase.as_ref()?;
    Some(match view.stage() {
        view::Stage::Interview if view.interview_question.is_some() => ("Phase 1/3: Answer the interview question", "The orchestrator is waiting for your decision. /interview opens the question, suggested options and your own answer field.".to_owned()),
        view::Stage::Interview if view.requirements_summary.is_empty() => ("Phase 1/3: Interview and requirements", "Describe your goal and answer the orchestrator's questions. Agree on scope, constraints and acceptance criteria. A requirements summary will appear here for approval.".to_owned()),
        view::Stage::Interview => ("Phase 1/3: Review requirements", format!("{}\n\nReview the summary. Add comments in chat or choose Proceed to planning.", view.requirements_summary)),
        view::Stage::Planning => ("Phase 2/3: Planning", "The orchestrator is preparing the plan from approved requirements. The next step is your review and comments.".to_owned()),
        view::Stage::Review => ("Phase 2/3: Review and comment on the plan", "Ctrl+L opens the plan. Add comments, submit Review and resolve outstanding feedback. Ctrl+A accepts the plan and starts implementation.".to_owned()),
        _ => ("Phase 3/3: Implementation", "Follow task progress and verification in the graph. Open task details to inspect results, diffs or add comments.".to_owned()),
    })
}
