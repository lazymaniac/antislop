use crate::lean::tui::editor::*;

#[test]
fn ac_edit_01_cursor_insertion_in_middle_of_text() {
    let mut buf = InputBuffer::with_text("ac");
    buf.cursor = 1; // At 'c'
    buf.insert_char('b');
    assert_eq!(buf.text, "abc");
    assert_eq!(buf.cursor, 2);
}

#[test]
fn ac_edit_02_backspace_in_middle_of_text() {
    let mut buf = InputBuffer::with_text("abc");
    buf.cursor = 2; // At 'c'
    assert!(buf.backspace());
    assert_eq!(buf.text, "ac");
    assert_eq!(buf.cursor, 1);
}

#[test]
fn ac_edit_03_word_jump_navigation() {
    let mut buf = InputBuffer::with_text("hello beautiful world");
    assert_eq!(buf.cursor, 21);
    buf.move_word_left();
    assert_eq!(buf.cursor, 16); // 'w' in "world"
    buf.move_word_left();
    assert_eq!(buf.cursor, 6); // 'b' in "beautiful"
    buf.move_word_left();
    assert_eq!(buf.cursor, 0); // 'h' in "hello"
    buf.move_word_right();
    assert_eq!(buf.cursor, 6); // past "hello " to 'b'
    buf.move_word_right();
    assert_eq!(buf.cursor, 16); // past "beautiful " to 'w'
    buf.move_word_right();
    assert_eq!(buf.cursor, 21);
}

#[test]
fn ac_edit_04_multiline_vertical_navigation() {
    let mut buf = InputBuffer::with_text("line1\nline2");
    buf.cursor = 2; // 'n' of line1
    buf.move_down(20);
    assert_eq!(buf.cursor, 8); // 'n' of line2 (6 + 2)
    assert_eq!(buf.visual_cursor(20), (2, 1));
    buf.move_up(20);
    assert_eq!(buf.cursor, 2); // 'n' of line1
    assert_eq!(buf.visual_cursor(20), (2, 0));
}

#[test]
fn ac_edit_05_visual_cursor_boundary_tracking() {
    // Text wrapping across 3 lines at width 20:
    // "1234567890 12345678 " (20 cols) -> line 0
    // "1234567890 12345678 " (20 cols) -> line 1
    // "last line" -> line 2
    let text = "1234567890 12345678 1234567890 12345678 last line";
    let mut buf = InputBuffer::with_text(text);
    assert_eq!(buf.visual_line_count(20), 3);

    // Put cursor in line 1: index 25 is '5' in the second segment
    buf.cursor = 25;
    let (col, row) = buf.visual_cursor(20);
    assert_eq!(row, 1);
    assert_eq!(col, 5);
}

#[test]
fn ac_edit_06_polish_diacritic_character_editing() {
    let mut buf = InputBuffer::with_text("aąćb");
    // 'a' is 1 byte (idx 0)
    // 'ą' is 2 bytes (idx 1..3)
    // 'ć' is 2 bytes (idx 3..5)
    // 'b' is 1 byte (idx 5..6)
    buf.cursor = 3; // between 'ą' and 'ć'
    assert!(buf.backspace());
    assert_eq!(buf.text, "aćb");
    assert_eq!(buf.cursor, 1);
}

#[test]
fn ac_paste_01_crlf_newline_normalization() {
    let pasted = sanitize_paste("a\r\nb\r\nc");
    assert_eq!(pasted, "a\nb\nc");
    let lone_cr = sanitize_paste("x\ry\rz");
    assert_eq!(lone_cr, "x\ny\nz");
}

#[test]
fn ac_paste_02_middle_of_line_paste_insertion() {
    let mut buf = InputBuffer::with_text("ab");
    buf.cursor = 1;
    buf.insert_str("XYZ");
    assert_eq!(buf.text, "aXYZb");
    assert_eq!(buf.cursor, 4);
}

#[test]
fn ac_paste_03_single_line_search_query_paste() {
    let sanitized = sanitize_single_line_paste("alpha\nbeta\r\ngamma");
    assert_eq!(sanitized, "alpha beta gamma");
}

#[test]
fn ac_key_04_home_end_line_navigation() {
    let mut buf = InputBuffer::with_text("line1\nline2");
    buf.cursor = 8; // In middle of line 2 ('n')
    buf.move_home();
    assert_eq!(buf.cursor, 6); // start of line 2
    buf.move_end();
    assert_eq!(buf.cursor, 11); // end of line 2
}

#[test]
fn ac_uni_02_double_width_cjk_width_calculation() {
    let buf = InputBuffer::with_text("漢");
    // '漢' is 3 bytes in UTF-8, width 2
    assert_eq!(buf.cursor, 3);
    let (col, row) = buf.visual_cursor(10);
    assert_eq!((col, row), (2, 0));
}

#[test]
fn ac_uni_03_combining_character_safety() {
    // Base 'e' + combining acute '\u{0301}' forms a single grapheme cluster
    let mut buf = InputBuffer::with_text("e\u{0301}");
    assert_eq!(buf.len(), 3);
    assert!(buf.backspace());
    assert_eq!(buf.text, "");
    assert_eq!(buf.cursor, 0);
}

#[test]
fn ac_uni_04_atomic_double_width_wrap() {
    // 1 column left on row, next char is '漢' (width 2)
    // With width 2: 'a' has width 1, so 1 column remains.
    // Next char '漢' must wrap atomically to the next line.
    let lines = compute_visual_lines("a漢", 2);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].width, 1);
    assert_eq!(lines[1].width, 2);
}

#[test]
fn delete_and_kill_actions() {
    let mut buf = InputBuffer::with_text("hello world");
    buf.cursor = 5; // at space
    buf.delete_to_end();
    assert_eq!(buf.text, "hello");
    buf.cursor = 3;
    buf.delete_to_start();
    assert_eq!(buf.text, "lo");

    let mut buf2 = InputBuffer::with_text("first second third");
    buf2.cursor = 18; // end
    buf2.delete_word_back();
    assert_eq!(buf2.text, "first second ");
    assert_eq!(buf2.cursor, 13);
}

#[test]
fn move_word_left_on_leading_whitespace_reaches_zero() {
    let mut buf = InputBuffer::with_text("   hello");
    buf.cursor = 2; // in leading spaces
    buf.move_word_left();
    assert_eq!(buf.cursor, 0);

    let mut buf2 = InputBuffer::with_text("   hello");
    buf2.cursor = 8; // end of hello
    buf2.move_word_left();
    assert_eq!(buf2.cursor, 3);
    buf2.move_word_left();
    assert_eq!(buf2.cursor, 0);
}

#[test]
fn soft_wrapped_vertical_navigation_preserves_lines() {
    let text = "1234567890 12345678 1234567890 12345678 last line";
    let mut buf = InputBuffer::with_text(text);
    assert_eq!(buf.visual_line_count(20), 3);

    // Put cursor in line 1 at the end (col >= line.width)
    buf.cursor = 39;
    assert_eq!(buf.visual_cursor(20).1, 1);

    // move_up must move to line 0, not stay on line 1
    buf.move_up(20);
    assert_eq!(buf.visual_cursor(20).1, 0);

    // move_down must move back to line 1, not skip to line 2
    buf.move_down(20);
    assert_eq!(buf.visual_cursor(20).1, 1);

    // move_down again moves to line 2
    buf.move_down(20);
    assert_eq!(buf.visual_cursor(20).1, 2);
}

#[test]
fn grapheme_zwj_and_combining_characters_editing() {
    let mut buf = InputBuffer::new();
    // Woman technologist: 👩‍💻 (\u{1F469}\u{200D}\u{1F4BB}) = 10 bytes
    let emoji = "👩‍💻";
    buf.insert_str(emoji);
    assert_eq!(buf.text, emoji);
    assert_eq!(buf.cursor, emoji.len());

    // Backspace removes the ENTIRE 10-byte ZWJ sequence in one press
    assert!(buf.backspace());
    assert_eq!(buf.text, "");
    assert_eq!(buf.cursor, 0);

    // Combining acute accent: e + \u{301}
    buf.insert_str("e\u{301}");
    assert_eq!(buf.cursor, "e\u{301}".len());
    // Left arrow moves past the whole combining character
    buf.move_left();
    assert_eq!(buf.cursor, 0);
    // Delete removes the whole combining character
    assert!(buf.delete());
    assert_eq!(buf.text, "");

    // Family emoji: 👨‍👩‍👧‍👦 = 25 bytes
    let family = "👨‍👩‍👧‍👦";
    buf.insert_str("start ");
    buf.insert_str(family);
    buf.insert_str(" end");
    buf.cursor = "start ".len() + family.len();
    buf.move_left();
    assert_eq!(buf.cursor, "start ".len());
    buf.move_right();
    assert_eq!(buf.cursor, "start ".len() + family.len());
}

#[test]
fn key_editing_keeps_combining_and_zwj_clusters_atomic() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let text = "e\u{301}👩‍💻";
    let mut buf = InputBuffer::with_text(text);
    buf.edit_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(buf.cursor, "e\u{301}".len());
    buf.edit_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(buf.cursor, text.len());
    buf.edit_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(buf.text, "e\u{301}");
    buf.edit_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert!(buf.text.is_empty());
}

#[test]
fn pasted_graphemes_remain_intact_at_narrow_wrap_boundaries() {
    let mut buf = InputBuffer::new();
    let pasted = "e\u{301}👩‍💻";
    buf.insert_str(pasted);
    assert_eq!(buf.text, pasted);
    assert_eq!(buf.visual_line_count(1), 2);
    assert!(buf.backspace());
    assert_eq!(buf.text, "e\u{301}");
    assert_eq!(buf.cursor, "e\u{301}".len());
}

#[test]
fn mutation_clamps_an_interior_cursor_to_a_whole_grapheme() {
    let family = "👨‍👩‍👧‍👦";
    let mut buf = InputBuffer::with_text(format!("e\u{301}{family}x"));
    buf.cursor = 1;
    assert!(buf.backspace());
    assert_eq!(buf.text, format!("{family}x"));
    assert_eq!(buf.cursor, 0);

    buf.cursor = family.len() - 1;
    assert!(buf.backspace());
    assert_eq!(buf.text, "x");
    assert_eq!(buf.cursor, 0);
}

#[test]
fn preferred_column_memory_across_short_lines() {
    // Line 0: "0123456789" (10 chars, cols 0..10)
    // Line 1: "01" (2 chars, cols 0..2)
    // Line 2: "0123456789" (10 chars, cols 0..10)
    let text = "0123456789\n01\n0123456789";
    let mut buf = InputBuffer::with_text(text);
    buf.cursor = 8; // On '8' in line 0 (col 8, row 0)
    assert_eq!(buf.visual_cursor(20), (8, 0));

    // Move down to Line 1 (short line with max col 2)
    buf.move_down(20);
    assert_eq!(buf.visual_cursor(20), (2, 1));
    assert_eq!(buf.preferred_col, Some(8));

    // Move down to Line 2 (longer line: should jump back to col 8!)
    buf.move_down(20);
    assert_eq!(buf.visual_cursor(20), (8, 2));
    assert_eq!(buf.preferred_col, Some(8));

    // Move up back to Line 1 (clamped to col 2)
    buf.move_up(20);
    assert_eq!(buf.visual_cursor(20), (2, 1));
    assert_eq!(buf.preferred_col, Some(8));

    // Move up to Line 0 (should jump back to col 8!)
    buf.move_up(20);
    assert_eq!(buf.visual_cursor(20), (8, 0));
    assert_eq!(buf.preferred_col, Some(8));

    // Horizontal movement resets preferred col
    buf.move_left();
    assert_eq!(buf.visual_cursor(20), (7, 0));
    assert_eq!(buf.preferred_col, None);

    // Subsequent move down now uses col 7
    buf.move_down(20);
    assert_eq!(buf.visual_cursor(20), (2, 1));
    assert_eq!(buf.preferred_col, Some(7));

    buf.move_down(20);
    assert_eq!(buf.visual_cursor(20), (7, 2));
}

#[test]
fn preferred_column_with_graphemes_and_double_width() {
    // Line 0: "abąćęł 12345" -> col 8 is '2'
    // Line 1: "漢" (width 2)
    // Line 2: "abąćęł 12345"
    let text = "abąćęł 12345\n漢\nabąćęł 12345";
    let mut buf = InputBuffer::with_text(text);
    // Find byte offset corresponding to visual col 8 in line 0
    buf.cursor = 0;
    while buf.visual_cursor(20).0 < 8 {
        buf.move_right();
    }
    assert_eq!(buf.visual_cursor(20), (8, 0));

    // Move down to Line 1 (clamped to CJK width 2)
    buf.move_down(20);
    assert_eq!(buf.visual_cursor(20), (2, 1));
    assert_eq!(buf.preferred_col, Some(8));

    // Move down to Line 2 (restores visual col 8)
    buf.move_down(20);
    assert_eq!(buf.visual_cursor(20), (8, 2));
    assert_eq!(buf.preferred_col, Some(8));
}
