use super::*;

pub(in crate::lean::tui) fn render(frame: &mut Frame, area: Rect, screen: &Screen, focused: bool) {
    draw_cells(frame, area, screen);
    draw_cursor(frame, area, screen, focused);
}

fn draw_cells(frame: &mut Frame, area: Rect, screen: &Screen) {
    for cell in &screen.cells {
        if !cell_visible(area, cell.x, cell.y, cell.width) {
            continue;
        }
        let style = cell_style(cell);
        frame
            .buffer_mut()
            .set_string(area.x + cell.x, area.y + cell.y, &cell.text, style);
    }
}

fn cell_visible(area: Rect, x: u16, y: u16, width: u16) -> bool {
    x < area.width && y < area.height && x.saturating_add(width) <= area.width
}

fn cell_style(cell: &crate::lean::terminal::Cell) -> ratatui::style::Style {
    use ratatui::style::{Color, Modifier, Style};
    let mut style = Style::default();
    if let Some([r, g, b]) = cell.foreground {
        style = style.fg(Color::Rgb(r, g, b));
    }
    if let Some([r, g, b]) = cell.background {
        style = style.bg(Color::Rgb(r, g, b));
    }
    for (bit, modifier) in [
        (1, Modifier::BOLD),
        (2, Modifier::DIM),
        (4, Modifier::ITALIC),
        (8, Modifier::UNDERLINED),
        (16, Modifier::REVERSED),
    ] {
        if cell.modifiers & bit != 0 {
            style = style.add_modifier(modifier);
        }
    }
    style
}

fn draw_cursor(frame: &mut Frame, area: Rect, screen: &Screen, focused: bool) {
    if !focused {
        return;
    }
    if let Some((x, y)) = screen
        .cursor
        .filter(|(x, y)| *x < area.width && *y < area.height)
    {
        frame.set_cursor_position((area.x + x, area.y + y));
    }
}
