use super::super::*;

struct Piece<'a> {
    doc_index: usize,
    index: usize,
    text: &'a str,
    count: usize,
}

impl PlanView {
    pub(super) fn layout_rows(
        &self,
        view: &View,
        doc_lines: &[DocLine],
        width: u16,
        theme: &Theme,
    ) -> (Vec<VisualRow>, (usize, usize)) {
        let width = usize::from(width);
        let mut rows = whole_plan_rows(self, view, width, theme);
        let mut cursor_rows = (0, 0);
        for (index, line) in doc_lines.iter().enumerate() {
            let bounds = append_document_rows(self, view, line, index, width, theme, &mut rows);
            if index == self.cursor {
                cursor_rows = bounds;
            }
        }
        (rows, cursor_rows)
    }
}

fn whole_plan_rows(plan: &PlanView, view: &View, width: usize, theme: &Theme) -> Vec<VisualRow> {
    plan.whole_plan_comments(view)
        .into_iter()
        .flat_map(|comment| {
            comment_lines(view, comment, "     ", width, theme)
                .into_iter()
                .map(|line| VisualRow { doc: None, line })
        })
        .collect()
}

fn append_document_rows(
    plan: &PlanView,
    view: &View,
    doc: &DocLine,
    index: usize,
    width: usize,
    theme: &Theme,
    rows: &mut Vec<VisualRow>,
) -> (usize, usize) {
    let start = rows.len();
    let pieces = crate::lean::tui::view::wrap(&doc.text, width.saturating_sub(GUTTER).max(1));
    append_text_pieces(plan, doc, index, &pieces, theme, rows);
    append_line_comments(plan, view, doc, index, width, theme, rows);
    (start, rows.len().saturating_sub(1))
}

fn append_text_pieces(
    plan: &PlanView,
    doc: &DocLine,
    index: usize,
    pieces: &[String],
    theme: &Theme,
    rows: &mut Vec<VisualRow>,
) {
    for (piece_index, piece) in pieces.iter().enumerate() {
        let piece = Piece {
            doc_index: index,
            index: piece_index,
            text: piece,
            count: pieces.len(),
        };
        rows.push(text_piece_row(plan, doc, piece, theme));
    }
}

fn text_piece_row(plan: &PlanView, doc: &DocLine, piece: Piece<'_>, theme: &Theme) -> VisualRow {
    let selected = piece.doc_index == plan.cursor;
    VisualRow {
        doc: Some(piece.doc_index),
        line: Line::from(piece_spans(plan, doc, &piece, selected, theme)),
    }
}

fn piece_spans(
    plan: &PlanView,
    doc: &DocLine,
    piece: &Piece<'_>,
    selected: bool,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let mut spans = vec![
        Span::styled(
            gutter(doc.number, selected, piece.index),
            gutter_style(selected, theme),
        ),
        Span::styled(piece.text.to_owned(), document_style(doc, selected)),
    ];
    if piece.index + 1 == piece.count {
        append_heading_badge(&mut spans, plan, doc, theme);
    }
    spans
}

fn gutter_style(selected: bool, theme: &Theme) -> Style {
    if selected {
        Style::default()
            .fg(crate::lean::tui::theme::ayu::YELLOW)
            .add_modifier(Modifier::BOLD)
    } else {
        theme.dim()
    }
}

fn document_style(doc: &DocLine, selected: bool) -> Style {
    let style = match doc.state {
        SectionState::Done => Style::default()
            .fg(crate::lean::tui::theme::ayu::GREEN)
            .add_modifier(Modifier::UNDERLINED),
        SectionState::Problem => Style::default().fg(crate::lean::tui::theme::ayu::RED),
        SectionState::Working => Style::default().fg(crate::lean::tui::theme::ayu::BLUE),
        SectionState::Open => Style::default(),
    };
    let style = if doc.is_heading {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    };
    if selected {
        style.add_modifier(Modifier::REVERSED)
    } else {
        style
    }
}

fn gutter(number: usize, selected: bool, piece: usize) -> String {
    if piece == 0 {
        format!("{}{:>3} │ ", if selected { "▶" } else { " " }, number)
    } else {
        "     │ ".to_owned()
    }
}

fn append_heading_badge(
    spans: &mut Vec<Span<'static>>,
    plan: &PlanView,
    doc: &DocLine,
    theme: &Theme,
) {
    if !doc.is_heading {
        return;
    }
    if let Some((text, color)) = heading_badge(plan, doc, theme) {
        spans.push(Span::styled(
            text,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
}

fn heading_badge(
    plan: &PlanView,
    doc: &DocLine,
    theme: &Theme,
) -> Option<(String, ratatui::style::Color)> {
    match doc.state {
        SectionState::Done => Some((
            format!(" {} COMPLETED", theme.glyphs().done),
            crate::lean::tui::theme::ayu::GREEN,
        )),
        SectionState::Working => Some((
            format!(" {} IN PROGRESS", theme.spinner(plan.tick)),
            crate::lean::tui::theme::ayu::BLUE,
        )),
        SectionState::Problem => Some((
            format!(" {} NEEDS ATTENTION", theme.glyphs().problem),
            crate::lean::tui::theme::ayu::RED,
        )),
        SectionState::Open => None,
    }
}

fn append_line_comments(
    plan: &PlanView,
    view: &View,
    doc: &DocLine,
    index: usize,
    width: usize,
    theme: &Theme,
    rows: &mut Vec<VisualRow>,
) {
    for comment in plan.line_comments(view, doc.number) {
        rows.extend(
            comment_lines(view, comment, "     │   ", width, theme)
                .into_iter()
                .map(|line| VisualRow {
                    doc: Some(index),
                    line,
                }),
        );
    }
}

fn comment_lines(
    view: &View,
    comment: &Comment,
    indent: &str,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut text = format!(
        "{indent}✎ #{} [{}]: {}",
        comment.id,
        view.comment_state(comment.id),
        clean(&comment.text)
    );
    if let Some(error) = view.comment_error(comment.id) {
        text.push_str(&format!("\n{indent}  Error: {}", clean(error)));
    }
    let style = comment_style(comment, theme);
    crate::lean::tui::view::wrap(&text, width.max(1))
        .into_iter()
        .map(|line| Line::styled(line, style))
        .collect()
}

fn comment_style(comment: &Comment, theme: &Theme) -> Style {
    if comment.status == "open" {
        Style::default()
            .fg(crate::lean::tui::theme::ayu::YELLOW)
            .add_modifier(Modifier::BOLD)
    } else {
        theme.dim()
    }
}
