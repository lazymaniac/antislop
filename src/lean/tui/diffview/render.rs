use super::header::{diff_footer_hint, diff_header_with_reviewer};
use super::*;

mod files;
mod lines;

impl DiffView {
    pub fn draw(&mut self, frame: &mut Frame, view: &View) {
        let area = frame.area();
        if area.width == 0 || area.height == 0 {
            return;
        }
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(area);
        draw_header_footer(frame, self, header, footer);
        self.draw_body(frame, body, view);
    }

    fn draw_body(&mut self, frame: &mut Frame, body: Rect, view: &View) {
        if !draw_placeholder(frame, body, &self.files) {
            return;
        }
        self.draw_loaded_body(frame, body, view);
    }

    fn draw_loaded_body(&mut self, frame: &mut Frame, body: Rect, view: &View) {
        let file_count = self.files().len();
        if body.width >= 90 {
            let width = (body.width / 3).min(44);
            let [list, rest] =
                Layout::horizontal([Constraint::Length(width), Constraint::Min(1)]).areas(body);
            files::draw_files(frame, list, self.files(), self.file, self.on_files, false);
            self.draw_lines(frame, rest, view, false, file_count);
        } else if self.on_files {
            files::draw_files(frame, body, self.files(), self.file, true, true);
        } else {
            self.draw_lines(frame, body, view, true, file_count);
        }
    }

    fn draw_lines(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &View,
        compact: bool,
        file_count: usize,
    ) {
        let Some(file) = self.files().get(self.file) else {
            return;
        };
        let target = format!("diff:{}:{}", self.task, file.path);
        let comments = lines::file_comments(view, &target);
        let width = usize::from(area.width);
        let (rows, cursor_row) = lines::diff_rows(lines::DiffRows {
            file,
            comments: &comments,
            task: &self.task,
            file_index: self.file,
            file_count,
            compact,
            selected_line: self.line,
            on_files: self.on_files,
            width,
        });
        let height = usize::from(area.height);
        self.scroll = lines::scroll_to_cursor(self.scroll, cursor_row, rows.len(), height);
        let end = (self.scroll + height).min(rows.len());
        frame.render_widget(Paragraph::new(rows[self.scroll..end].to_vec()), area);
    }
}

fn draw_placeholder(
    frame: &mut Frame,
    body: Rect,
    state: &Option<Result<Vec<FileDiff>, String>>,
) -> bool {
    match state {
        None => {
            render_body_message(frame, body, "Loading task diff from daemon…", dim());
            false
        }
        Some(Err(error)) => {
            render_diff_error(frame, body, error);
            false
        }
        Some(Ok(files)) if files.is_empty() => {
            render_body_message(
                frame,
                body,
                "No changes in this task. The task did not modify any files.",
                dim(),
            );
            false
        }
        Some(Ok(_)) => true,
    }
}

fn render_diff_error(frame: &mut Frame, body: Rect, error: &str) {
    render_body_message(
        frame,
        body,
        &format!("✗ Error loading changes: {error}"),
        Style::default().fg(super::super::theme::ayu::RED),
    );
}

fn render_body_message(frame: &mut Frame, area: Rect, message: &str, style: Style) {
    frame.render_widget(
        Paragraph::new(Span::styled(message.to_owned(), style)),
        area,
    );
}

fn draw_header_footer(frame: &mut Frame, view: &DiffView, header: Rect, footer: Rect) {
    let (added, removed) = view
        .files()
        .iter()
        .fold((0, 0), |(a, r), file| (a + file.added, r + file.removed));
    frame.render_widget(
        Paragraph::new(diff_header_with_reviewer(
            &view.task,
            &view.range,
            view.files().len(),
            added,
            removed,
            view.truncated,
            view.reviewer.as_ref(),
            usize::from(header.width),
        )),
        header,
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            diff_footer_hint(usize::from(footer.width)),
            dim(),
        )),
        footer,
    );
}
