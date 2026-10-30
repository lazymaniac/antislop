use super::super::view::wrap;
use super::{Form, InputBuffer, Question, Theme};
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};

impl Form {
    pub fn draw(&mut self, frame: &mut Frame, screen: Rect, theme: &Theme) {
        let (area, block) = interview_box(screen, theme);
        let inner_width = usize::from(area.width.saturating_sub(3).max(1));
        let custom = self.selected == self.question.options.len();
        let (rows, selected_row, input_row) = self.rows(inner_width, custom, theme);
        let area = content_box(area, rows.len());
        let viewport = usize::from(area.height.saturating_sub(2).max(1));
        let (cursor_col, cursor_row) = self.input.visual_cursor(inner_width);
        let target = if custom {
            input_row + cursor_row
        } else {
            selected_row
        };
        let scroll = self.visible_scroll(target, viewport, rows.len());
        render_interview_rows(frame, area, rows, scroll, viewport, theme, block);
        self.place_cursor(frame, area, custom, target, cursor_col);
    }

    fn rows(
        &self,
        width: usize,
        custom: bool,
        theme: &Theme,
    ) -> (Vec<Line<'static>>, usize, usize) {
        let mut rows = question_rows(&self.question, width, theme);
        let option_selected = option_rows(
            &mut rows,
            &self.question.options,
            self.selected,
            width,
            theme,
        );
        let selected = if custom { rows.len() } else { option_selected };
        let input_row = append_answer_rows(
            &mut rows,
            &self.input,
            self.error.as_deref(),
            custom,
            width,
            theme,
        );
        append_interview_footer(&mut rows, self.sending, theme);
        (rows, selected, input_row)
    }

    fn visible_scroll(&mut self, target: usize, viewport: usize, row_count: usize) -> usize {
        if !self.manual_scroll {
            if target < self.scroll {
                self.scroll = target;
            }
            if target >= self.scroll + viewport {
                self.scroll = target + 1 - viewport;
            }
        }
        self.scroll = self.scroll.min(row_count.saturating_sub(viewport));
        self.scroll
    }

    fn place_cursor(&self, frame: &mut Frame, area: Rect, custom: bool, target: usize, col: u16) {
        if !custom || self.sending || self.manual_scroll || area.width < 3 || area.height < 3 {
            return;
        }
        frame.set_cursor_position((
            area.x + 1 + col.min(area.width.saturating_sub(3)),
            area.y
                + 1
                + (target.saturating_sub(self.scroll) as u16).min(area.height.saturating_sub(3)),
        ));
    }
}

fn content_box(area: Rect, row_count: usize) -> Rect {
    let height = row_count.saturating_add(2).min(usize::from(area.height)) as u16;
    Rect::new(
        area.x,
        area.y + area.height.saturating_sub(height) / 2,
        area.width,
        height,
    )
}

fn render_interview_rows(
    frame: &mut Frame,
    area: Rect,
    rows: Vec<Line<'static>>,
    scroll: usize,
    viewport: usize,
    theme: &Theme,
    block: Block<'static>,
) {
    let visible = rows
        .into_iter()
        .skip(scroll)
        .take(viewport)
        .collect::<Vec<_>>();
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(visible).style(theme.card()).block(block),
        area,
    );
}

fn question_rows(question: &Question, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let mut rows = Vec::new();
    append_wrapped(&mut rows, &question.context, width, theme.dim(), theme);
    rows.push(Line::raw(""));
    append_wrapped(&mut rows, &question.question, width, theme.title(), theme);
    rows.push(Line::raw(""));
    rows
}

fn append_answer_rows(
    rows: &mut Vec<Line<'static>>,
    input: &InputBuffer,
    error: Option<&str>,
    custom: bool,
    width: usize,
    theme: &Theme,
) -> usize {
    rows.push(Line::styled(
        "Other / own answer:",
        if custom { theme.accent() } else { theme.dim() },
    ));
    let input_row = rows.len();
    append_input_rows(rows, input, width, theme);
    if let Some(error) = error {
        append_wrapped(rows, error, width, theme.error(), theme);
    }
    input_row
}

fn append_interview_footer(rows: &mut Vec<Line<'static>>, sending: bool, theme: &Theme) {
    let footer = if sending {
        "Sending… draft preserved"
    } else {
        "Up/Down/Tab choose · Enter send · PgUp/PgDn scroll · Esc chat"
    };
    rows.push(Line::styled(footer, theme.dim()));
}

fn interview_box(screen: Rect, theme: &Theme) -> (Rect, Block<'static>) {
    let width = screen.width.min(94);
    let area = Rect::new(
        screen.x + screen.width.saturating_sub(width) / 2,
        screen.y,
        width,
        screen.height,
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Question · choose an answer ")
        .border_style(theme.accent());
    (area, block)
}

fn append_wrapped(
    rows: &mut Vec<Line<'static>>,
    text: &str,
    width: usize,
    style: ratatui::style::Style,
    theme: &Theme,
) {
    rows.extend(
        wrap(&theme.clean(text), width)
            .into_iter()
            .map(|line| Line::styled(line, style)),
    );
}

fn option_rows(
    rows: &mut Vec<Line<'static>>,
    options: &[crate::lean::interview::Choice],
    selected: usize,
    width: usize,
    theme: &Theme,
) -> usize {
    let mut selected_row = rows.len();
    for (index, option) in options.iter().enumerate() {
        let picked = selected == index;
        if picked {
            selected_row = rows.len();
        }
        append_option(rows, option, index, picked, width, theme);
    }
    selected_row
}

fn append_option(
    rows: &mut Vec<Line<'static>>,
    option: &crate::lean::interview::Choice,
    index: usize,
    picked: bool,
    width: usize,
    theme: &Theme,
) {
    append_option_label(rows, option, index, picked, width, theme);
    append_wrapped(
        rows,
        &format!("   {}", option.description),
        width,
        theme.dim(),
        theme,
    );
}

fn append_option_label(
    rows: &mut Vec<Line<'static>>,
    option: &crate::lean::interview::Choice,
    index: usize,
    picked: bool,
    width: usize,
    theme: &Theme,
) {
    let style = if picked {
        theme.accent().add_modifier(Modifier::REVERSED)
    } else {
        theme.text()
    };
    append_wrapped(
        rows,
        &format!(
            "{} {}. {}",
            if picked { ">" } else { " " },
            index + 1,
            option.label
        ),
        width,
        style,
        theme,
    );
}

fn append_input_rows(
    rows: &mut Vec<Line<'static>>,
    input: &InputBuffer,
    width: usize,
    theme: &Theme,
) {
    if input.is_empty() {
        rows.push(Line::styled("Type a different answer here…", theme.dim()));
    } else {
        rows.extend(
            super::super::visual_text_lines(&input.text, width)
                .iter()
                .map(|line| Line::styled(theme.clean(line), theme.text())),
        );
    }
}
