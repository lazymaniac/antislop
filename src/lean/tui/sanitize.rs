//! Terminal escape sequence sanitization.
//! Provider, tool, and agent output is data, never terminal instructions.
//! Strip CSI, OSC (including OSC52 clipboard escapes), DCS/APC/PM/SOS strings,
//! C0/C1 controls, and bidirectional overrides.
//! Preserve readable newlines and tabs (expanded to spaces).
//! An unfinished escape or control string consumes the remainder of the input.

pub fn sanitize(input: &str) -> String {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Text,
        Escape,
        Csi,
        String,
        StringEscape,
    }

    let mut state = State::Text;
    let mut output = String::with_capacity(input.len());

    for c in input.chars() {
        state = match state {
            State::Text => match c {
                '\u{1b}' => State::Escape,
                // C1 CSI
                '\u{9b}' => State::Csi,
                // C1 DCS, OSC, SOS, PM, APC
                '\u{90}' | '\u{9d}' | '\u{98}' | '\u{9e}' | '\u{9f}' => State::String,
                '\n' => {
                    output.push('\n');
                    State::Text
                }
                '\t' => {
                    output.push_str("    ");
                    State::Text
                }
                // Bidirectional text override and isolate controls:
                // U+202A..=U+202E (LRE, RLE, PDF, LRO, RLO)
                // U+2066..=U+2069 (LRI, RLI, FSI, PDI)
                // U+200E, U+200F (LRM, RLM)
                // U+061C (ALM)
                '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{061c}' => State::Text,
                // Other control characters (C0: 0x00..=0x1F, 0x7F; C1: 0x80..=0x9F)
                c if c.is_control() => State::Text,
                c => {
                    output.push(c);
                    State::Text
                }
            },
            State::Escape => match c {
                '[' => State::Csi,
                // OSC (']'), DCS ('P'), SOS ('X'), PM ('^'), APC ('_')
                ']' | 'P' | '_' | '^' | 'X' => State::String,
                _ => State::Text,
            },
            State::Csi => {
                // CSI parameter and intermediate bytes: 0x20..=0x3F. Final byte: 0x40..=0x7E.
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    State::Text
                } else {
                    State::Csi
                }
            }
            State::String => match c {
                // String terminator: BEL ('\x07') or C1 ST ('\u{9c}')
                '\u{7}' | '\u{9c}' => State::Text,
                // 7-bit ST: ESC '\'
                '\u{1b}' => State::StringEscape,
                _ => State::String,
            },
            State::StringEscape => {
                if c == '\\' {
                    State::Text
                } else {
                    State::String
                }
            }
        };
    }
    output
}

/// [`sanitize`] collapsed to one printable line for fields a view renders
/// inside a single row (titles, ids, paths, sections): every whitespace
/// run, newlines included, becomes one space so no control reaches the
/// line and no break shifts the row. Whitespace is folded on space
/// characters between clusters, so readable text and grapheme clusters
/// (emoji families, flags, combining marks) stay whole.
pub fn sanitize_line(input: &str) -> String {
    sanitize(input)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_and_newlines_preserved() {
        assert_eq!(sanitize("Hello world!"), "Hello world!");
        assert_eq!(
            sanitize("Line 1\nLine 2\n\nLine 4"),
            "Line 1\nLine 2\n\nLine 4"
        );
        assert_eq!(sanitize("col1\tcol2"), "col1    col2");
    }

    #[test]
    fn csi_colors_and_cursor_stripped() {
        assert_eq!(sanitize("\x1b[31mRed\x1b[0m"), "Red");
        assert_eq!(sanitize("\x1b[1;32;40mBold Green\x1b[m"), "Bold Green");
        assert_eq!(sanitize("\x1b[2J\x1b[HClear"), "Clear");
        // C1 CSI
        assert_eq!(sanitize("\u{9b}31mRed\u{9b}0m"), "Red");
    }

    #[test]
    fn osc52_and_clipboard_attacks_stripped() {
        // OSC 52 set clipboard terminated by BEL
        assert_eq!(
            sanitize("\x1b]52;c;cGFzc3dvcmQ=\x07Innocent text"),
            "Innocent text"
        );
        // OSC 52 terminated by ST (\x1b\\)
        assert_eq!(
            sanitize("\x1b]52;c;cGFzc3dvcmQ=\x1b\\Innocent text"),
            "Innocent text"
        );
        // C1 OSC with C1 ST
        assert_eq!(
            sanitize("\u{9d}52;c;cGFzc3dvcmQ=\u{9c}Innocent text"),
            "Innocent text"
        );
    }

    #[test]
    fn dcs_and_apc_stripped() {
        assert_eq!(sanitize("\x1bP1$txxxxx\x1b\\Data"), "Data");
        assert_eq!(sanitize("\x1b_Gpayload\x07Visible"), "Visible");
    }

    #[test]
    fn bidi_override_controls_stripped() {
        // RLO (Right-to-Left Override) U+202E spoofing filename
        assert_eq!(sanitize("report\u{202e}txt.exe"), "reporttxt.exe");
        // LRI / RLI / FSI isolates
        assert_eq!(sanitize("\u{2066}admin\u{2069}"), "admin");
    }

    #[test]
    fn unfinished_escape_consumes_tail() {
        assert_eq!(sanitize("Safe\x1b[31"), "Safe");
        assert_eq!(sanitize("Safe\x1b]52;c;something"), "Safe");
        assert_eq!(sanitize("Safe\x1bPdevice_command"), "Safe");
    }

    #[test]
    fn single_line_folds_breaks_without_splitting_clusters() {
        assert_eq!(sanitize_line("Fix\nthe  store\tpath"), "Fix the store path");
        // Escapes, controls and bidi never reach the line; text stays.
        assert_eq!(
            sanitize_line("\x1b[31mFix\u{7}\u{9b}31m the\u{202e}store\x1b]0;evil\u{7}"),
            "Fix thestore"
        );
        // Grapheme clusters stay whole: an emoji family and combining marks
        // survive the fold untouched.
        assert_eq!(sanitize_line("a\u{0301} 👩‍👩‍👧‍👦  b\n"), "a\u{0301} 👩‍👩‍👧‍👦 b");
        // Ids and paths come back byte-identical when they were clean.
        assert_eq!(sanitize_line("worker:t1"), "worker:t1");
        assert_eq!(sanitize_line("src/lean/tasks.rs"), "src/lean/tasks.rs");
        // An escape whose payload would have filled the row is gone with it.
        assert_eq!(sanitize_line("\x1b]52;c;AAAA\u{7}done"), "done");
    }
}

struct PinnedSelection {
    start: (usize, u16),
    end: (usize, u16),
    text: String,
}

/// A viewport-sized snapshot: never retains an agent's full output.
#[derive(Default)]
pub struct TextSelection {
    area: ratatui::layout::Rect,
    context: String,
    cells: Vec<(u16, u16, u16, String)>,
    anchor: Option<usize>,
    end: Option<usize>,
    dragging: bool,
    last_click: Option<(usize, std::time::Instant)>,
    origin: usize,
    pinned: Option<PinnedSelection>,
}

impl TextSelection {
    pub fn draw(
        &mut self,
        buf: &mut ratatui::buffer::Buffer,
        area: Option<ratatui::layout::Rect>,
        context: String,
    ) {
        self.draw_scrolled(buf, area, context, 0);
    }

    pub fn invalidate_context(&mut self, context: &str) {
        if self.context != context {
            self.anchor = None;
            self.end = None;
            self.pinned = None;
            self.dragging = false;
            self.last_click = None;
        }
    }

    pub fn draw_scrolled(
        &mut self,
        buf: &mut ratatui::buffer::Buffer,
        area: Option<ratatui::layout::Rect>,
        context: String,
        origin: usize,
    ) {
        use unicode_segmentation::UnicodeSegmentation;
        use unicode_width::UnicodeWidthStr;
        let area = area.unwrap_or_default().intersection(buf.area);
        let mut cells = Vec::new();
        for y in area.y..area.bottom() {
            let mut x = area.x;
            while x < area.right() {
                let symbol = buf[(x, y)].symbol();
                // Ratatui stores an extended grapheme in its leading cell.
                let symbol = symbol.graphemes(true).next().unwrap_or(" ");
                let width = symbol.width().max(1).min(usize::from(area.right() - x)) as u16;
                cells.push((x, y, width, symbol.to_owned()));
                x += width;
            }
        }
        let changed = self.area != area
            || self.context != context
            || (self.origin == origin && self.cells != cells);
        if !changed && self.origin != origin {
            if let Some((start, end)) = self.range() {
                let position = |index: usize| {
                    let cell = &self.cells[index];
                    (self.origin + usize::from(cell.1 - self.area.y), cell.0)
                };
                self.pinned = self.text().map(|text| PinnedSelection {
                    start: position(start),
                    end: position(end),
                    text,
                });
            }
            self.anchor = None;
            self.end = None;
            self.dragging = false;
            self.last_click = None;
        }
        if changed {
            self.anchor = None;
            self.end = None;
            self.dragging = false;
            self.last_click = None;
            self.pinned = None;
        }
        self.origin = origin;
        self.area = area;
        self.context = context;
        self.cells = cells;
        if let Some(pinned) = &self.pinned {
            for (x, y, width, _) in &self.cells {
                let position = (self.origin + usize::from(*y - area.y), *x);
                if position >= pinned.start && position <= pinned.end {
                    for col in *x..x + width {
                        let cell = &mut buf[(col, *y)];
                        cell.set_style(
                            cell.style()
                                .add_modifier(ratatui::style::Modifier::REVERSED),
                        );
                    }
                }
            }
        }
        if let Some((start, end)) = self.range() {
            for (x, y, width, _) in &self.cells[start..=end] {
                for col in *x..x + width {
                    let cell = &mut buf[(col, *y)];
                    cell.set_style(
                        cell.style()
                            .add_modifier(ratatui::style::Modifier::REVERSED),
                    );
                }
            }
        }
    }

    fn range(&self) -> Option<(usize, usize)> {
        let (a, b) = (self.anchor?, self.end?);
        Some((a.min(b), a.max(b)))
    }

    pub fn mouse(&mut self, event: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};
        let hit = self.cells.iter().position(|(x, y, width, _)| {
            *y == event.row && event.column >= *x && event.column < x + width
        });
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.pinned = None;
                self.anchor = hit;
                self.end = None;
                self.dragging = hit.is_some();
                if let Some(index) = hit {
                    let now = std::time::Instant::now();
                    if self.last_click.is_some_and(|(previous, time)| {
                        previous == index
                            && now.duration_since(time) < std::time::Duration::from_millis(450)
                    }) {
                        let row = self.cells[index].1;
                        let text: String = self
                            .cells
                            .iter()
                            .filter(|c| c.1 == row)
                            .map(|c| c.3.as_str())
                            .collect();
                        let first = self.cells.iter().position(|c| c.1 == row).unwrap_or(index);
                        let offset: usize =
                            self.cells[first..index].iter().map(|c| c.3.len()).sum();
                        use unicode_segmentation::UnicodeSegmentation;
                        for (start, word) in text.split_word_bound_indices() {
                            if offset >= start && offset < start + word.len() {
                                let mut byte = 0;
                                for i in first..self.cells.len() {
                                    if self.cells[i].1 != row {
                                        break;
                                    }
                                    if byte == start {
                                        self.anchor = Some(i);
                                    }
                                    if byte < start + word.len() {
                                        self.end = Some(i);
                                    }
                                    byte += self.cells[i].3.len();
                                }
                                break;
                            }
                        }
                        self.dragging = false;
                    }
                    self.last_click = Some((index, now));
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if self.dragging => {
                if hit.is_some() {
                    self.end = hit;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => self.dragging = false,
            _ => {}
        }
    }

    pub fn text(&self) -> Option<String> {
        if let Some(pinned) = &self.pinned {
            return Some(pinned.text.clone());
        }
        let (start, end) = self.range()?;
        let mut text = String::new();
        let mut row = self.cells[start].1;
        for (_, y, _, symbol) in &self.cells[start..=end] {
            if *y != row {
                text.truncate(text.trim_end_matches(' ').len());
                text.push('\n');
                row = *y;
            }
            text.push_str(symbol);
        }
        Some(sanitize(text.trim_end_matches(' ')))
    }
}

/// Encode sanitized text only; terminal control input can never escape the payload.
pub fn clipboard_sequence(text: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let clean = sanitize(text);
    let mut encoded = String::new();
    for chunk in clean.as_bytes().chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        encoded.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        encoded.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    format!("\x1b]52;c;{encoded}\x07")
}

pub fn copy_text(text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    let result = stdout
        .write_all(clipboard_sequence(text).as_bytes())
        .and_then(|()| stdout.flush());
    drop(stdout);
    #[cfg(target_os = "macos")]
    {
        let _ = &result;
        // OSC 52 has no acknowledgement: also populate the local clipboard on macOS.
        let mut child = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()?;
        let clean = sanitize(text);
        let input = child.stdin.take();
        let writer = std::thread::spawn(move || {
            if let Some(mut stdin) = input {
                stdin.write_all(clean.as_bytes())
            } else {
                Ok(())
            }
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            if let Some(status) = child.try_wait()? {
                writer
                    .join()
                    .map_err(|_| std::io::Error::other("pbcopy stdin writer panicked"))??;
                return if status.success() {
                    Ok(())
                } else {
                    Err(std::io::Error::other(format!("pbcopy: {status}")))
                };
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                let _ = writer.join();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "pbcopy timed out",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        result
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    #[test]
    fn double_click_word_and_backwards_drag() {
        let area = ratatui::layout::Rect::new(0, 0, 12, 2);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        buf.set_string(0, 0, "one café", ratatui::style::Style::default());
        buf.set_string(0, 1, "two", ratatui::style::Style::default());
        let mut selection = TextSelection::default();
        selection.draw(&mut buf, Some(area), "chat".into());
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
        selection.draw(&mut buf, Some(area), "chat".into());
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
        selection.draw(&mut buffer, Some(area), "wrapped".into());
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

    #[test]
    fn osc52_padding_and_sanitization() {
        for (text, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("\x1b]52;c;evil\x07foo", "Zm9v"),
        ] {
            assert_eq!(clipboard_sequence(text), format!("\x1b]52;c;{encoded}\x07"));
        }
    }
}
