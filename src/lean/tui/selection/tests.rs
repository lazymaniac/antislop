use super::*;
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
#[test]
fn double_click_word_and_backwards_drag() {
    let area = ratatui::layout::Rect::new(0, 0, 12, 2);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    buf.set_string(0, 0, "one café", ratatui::style::Style::default());
    buf.set_string(0, 1, "two", ratatui::style::Style::default());
    let mut selection = TextSelection::default();
    selection.draw_scrolled(&mut buf, Some(area), "chat".into(), 0);
    let mouse = |kind, column, row| MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    selection.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 6, 0));
    selection.mouse(mouse(MouseEventKind::Up(MouseButton::Left), 6, 0));
    selection.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 6, 0));
    assert_eq!(selection.text().as_deref(), Some("café"));
    selection.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 1));
    selection.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 4, 0));
    assert_eq!(selection.text().as_deref(), Some("café\ntwo"));
    buf.set_string(0, 0, "changed", ratatui::style::Style::default());
    selection.draw_scrolled(&mut buf, Some(area), "chat".into(), 0);
    assert!(selection.text().is_none());
}
#[test]
fn wrapped_rows_keep_whole_clusters_and_viewport_storage() {
    let area = ratatui::layout::Rect::new(0, 0, 4, 3);
    let mut buffer = ratatui::buffer::Buffer::empty(area);
    let rows = super::super::view::wrap("ab👩‍👩‍👧‍👦café", 4);
    for (y, row) in rows.iter().enumerate() {
        buffer.set_string(0, y as u16, row, ratatui::style::Style::default());
    }
    let mut selection = TextSelection::default();
    selection.draw_scrolled(&mut buffer, Some(area), "wrapped".into(), 0);
    let mouse = |kind, column, row| MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    // Start on the second display column of the family emoji.
    selection.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 3, 0));
    selection.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 3, 1));
    assert_eq!(selection.text().as_deref(), Some("👩‍👩‍👧‍👦\ncafé"));
    assert!(selection.cells.len() <= usize::from(area.width) * usize::from(area.height));
}
