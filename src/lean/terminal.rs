//! Persistent, viewport-sized terminal state owned by the daemon.
use super::{Core, native};
use crate::{Result, require};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use native::invalid;
use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use unicode_segmentation::UnicodeSegmentation;
use wezterm_term::{Terminal, TerminalConfiguration, TerminalSize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    Key { event: KeyEvent },
    Paste { text: String },
    Mouse { event: MouseEvent },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub x: u16,
    pub y: u16,
    pub text: String,
    pub width: u16,
    pub foreground: Option<[u8; 3]>,
    pub background: Option<[u8; 3]>,
    pub modifiers: u16,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Screen {
    pub handle: String,
    pub revision: u64,
    pub columns: u16,
    pub rows: u16,
    pub cursor: Option<(u16, u16)>,
    pub cells: Vec<Cell>,
    pub error: Option<String>,
}

#[derive(Debug)]
struct Config;
impl TerminalConfiguration for Config {
    fn scrollback_size(&self) -> usize {
        0
    }
    fn color_palette(&self) -> wezterm_term::color::ColorPalette {
        Default::default()
    }
    fn unicode_version(&self) -> wezterm_term::UnicodeVersion {
        wezterm_term::UnicodeVersion::new(14)
    }
}
struct State {
    term: Terminal,
    revision: u64,
    error: Option<String>,
}
struct Handle {
    sequence: u64,
    seen: std::time::Instant,
}
pub struct Session {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    state: Arc<Mutex<State>>,
    handles: HashMap<String, Handle>,
    registration: Option<(std::sync::Weak<Core>, u32)>,
}

#[derive(Clone)]
struct Writer(std::sync::mpsc::SyncSender<Vec<u8>>);
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .try_send(bytes.to_vec())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::WouldBlock, e))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn size(columns: u16, rows: u16) -> Result<TerminalSize> {
    require(
        columns > 0 && rows > 0 && columns <= 500 && rows <= 200,
        "terminal dimensions must be within 1x1 and 500x200",
    )?;
    Ok(TerminalSize {
        rows: rows.into(),
        cols: columns.into(),
        ..Default::default()
    })
}
fn pty_size(columns: u16, rows: u16) -> PtySize {
    PtySize {
        cols: columns,
        rows,
        pixel_width: 0,
        pixel_height: 0,
    }
}

impl Session {
    fn spawn(native: &native::Session, columns: u16, rows: u16) -> Result<Self> {
        let dimensions = size(columns, rows)?;
        let pair = NativePtySystem::default()
            .openpty(pty_size(columns, rows))
            .map_err(invalid)?;
        let mut command = CommandBuilder::new(&native.spec.binary);
        command.cwd(&native.spec.cwd);
        command.env("TERM", "xterm-256color");
        match &native.backend {
            native::Backend::Codex { .. } => {
                command.args(["resume", &native.identity, "--remote", &native.endpoint]);
            }
            native::Backend::Opencode { .. } => {
                let address = native
                    .gateway_address
                    .lock()
                    .map_err(invalid)?
                    .clone()
                    .ok_or_else(|| invalid("OpenCode gateway is not ready"))?;
                command.args(["attach", &address, "--session", &native.identity]);
                command.env("OPENCODE_SERVER_PASSWORD", &native.gateway_password);
                command.env("OPENCODE_SERVER_USERNAME", "opencode");
            }
        }
        let mut child = pair.slave.spawn_command(command).map_err(invalid)?;
        let registration = child.process_id().map(|pid| (native.core.clone(), pid));
        drop(pair.slave);
        if let Some(pid) = child.process_id() {
            let Some(core) = native.core.upgrade() else {
                let _ = child.kill();
                return Err(invalid("daemon stopped"));
            };
            if let Err(error) = core.store.db().execute(
                "INSERT OR REPLACE INTO native_processes(pid,started_ms,provider) VALUES(?1,?2,?3)",
                rusqlite::params![
                    pid,
                    super::store::now_ms(),
                    super::agent::name(native.spec.provider)
                ],
            ) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.into());
            }
        }
        let mut writer = pair.master.take_writer().map_err(invalid)?;
        let mut reader = pair.master.try_clone_reader().map_err(invalid)?;
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(64);
        let state = Arc::new(Mutex::new(State {
            term: Terminal::new(
                dimensions,
                Arc::new(Config),
                "antislop",
                env!("CARGO_PKG_VERSION"),
                Box::new(Writer(tx)),
            ),
            revision: 1,
            error: None,
        }));
        let weak = Arc::downgrade(&state);
        std::thread::spawn(move || {
            while let Ok(bytes) = rx.recv() {
                if let Err(error) = writer.write_all(&bytes) {
                    if let Some(state) = weak.upgrade() {
                        state.lock().expect("terminal").error = Some(error.to_string());
                    }
                    break;
                }
            }
        });
        let weak = Arc::downgrade(&state);
        std::thread::spawn(move || {
            let mut buffer = [0; 8192];
            let mut filter = Filter::default();
            let mut graphemes = GraphemeStream::default();
            loop {
                let count = match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => count,
                    Err(_) => break,
                };
                let Some(state) = weak.upgrade() else {
                    break;
                };
                #[cfg(test)]
                if std::env::var("ANTISLOP_NATIVE_DEBUG").as_deref() == Ok("pty") {
                    eprintln!(
                        "PTY: {}",
                        String::from_utf8_lossy(&buffer[..count]).escape_debug()
                    );
                }
                let mut state = state.lock().expect("terminal");
                if let Err(error) = graphemes.feed(&mut state.term, &filter.feed(&buffer[..count]))
                {
                    state.error = Some(error.to_string());
                    break;
                }
                state.revision = state.revision.wrapping_add(1);
            }
            if let Some(state) = weak.upgrade() {
                let mut state = state.lock().expect("terminal");
                state
                    .error
                    .get_or_insert_with(|| "native terminal disconnected; retry attachment".into());
                state.revision = state.revision.wrapping_add(1);
            }
        });
        Ok(Self {
            master: pair.master,
            child,
            state,
            handles: HashMap::new(),
            registration,
        })
    }

    fn snapshot(&mut self, handle: &str, after: Option<u64>) -> Result<Option<Screen>> {
        let handle_state = self
            .handles
            .get_mut(handle)
            .ok_or_else(|| invalid("unknown or detached terminal handle"))?;
        handle_state.seen = std::time::Instant::now();
        let state = self.state.lock().map_err(invalid)?;
        if after == Some(state.revision) {
            return Ok(None);
        }
        let dimensions = state.term.get_size();
        let cursor = state.term.cursor_pos();
        let palette = state.term.palette();
        let top = state.term.screen().scrollback_or_visible_row(0);
        let lines = state
            .term
            .screen()
            .lines_in_phys_range(top..top + dimensions.rows);
        let mut cells = Vec::with_capacity(dimensions.rows * dimensions.cols);
        for (y, line) in lines.iter().enumerate() {
            for cell in line.visible_cells() {
                if cell.cell_index() >= dimensions.cols {
                    break;
                }
                let attributes = cell.attrs();
                let mut modifiers = 0;
                if attributes.intensity() == wezterm_term::Intensity::Bold {
                    modifiers |= 1;
                }
                if attributes.intensity() == wezterm_term::Intensity::Half {
                    modifiers |= 2;
                }
                if attributes.italic() {
                    modifiers |= 4;
                }
                if attributes.underline() != wezterm_term::Underline::None {
                    modifiers |= 8;
                }
                if attributes.reverse() {
                    modifiers |= 16;
                }
                let foreground =
                    if attributes.foreground() == wezterm_term::color::ColorAttribute::Default {
                        None
                    } else {
                        let (r, g, b, _) = palette.resolve_fg(attributes.foreground()).to_srgb_u8();
                        Some([r, g, b])
                    };
                let background =
                    if attributes.background() == wezterm_term::color::ColorAttribute::Default {
                        None
                    } else {
                        let (r, g, b, _) = palette.resolve_bg(attributes.background()).to_srgb_u8();
                        Some([r, g, b])
                    };
                cells.push(Cell {
                    x: cell.cell_index() as u16,
                    y: y as u16,
                    text: cell.str().into(),
                    width: cell.width() as u16,
                    foreground,
                    background,
                    modifiers,
                });
            }
        }
        Ok(Some(Screen {
            handle: handle.into(),
            revision: state.revision,
            columns: dimensions.cols as u16,
            rows: dimensions.rows as u16,
            cursor: if format!("{:?}", cursor.visibility) == "Visible" {
                Some((cursor.x as u16, cursor.y.max(0) as u16))
            } else {
                None
            },
            cells,
            error: state.error.clone(),
        }))
    }

    fn input(&mut self, handle: &str, sequence: u64, input: Input) -> Result<()> {
        let last = self
            .handles
            .get_mut(handle)
            .ok_or_else(|| invalid("unknown or detached terminal handle"))?;
        if sequence <= last.sequence {
            return Ok(());
        }
        require(
            sequence == last.sequence.saturating_add(1),
            "terminal input sequence has a gap",
        )?;
        last.sequence = sequence;
        last.seen = std::time::Instant::now();
        let mut state = self.state.lock().map_err(invalid)?;
        require(
            state.error.is_none(),
            state.error.as_deref().unwrap_or("terminal failed"),
        )?;
        match input {
            Input::Paste { text } => {
                require(text.len() <= 1024 * 1024, "terminal paste exceeds 1 MiB")?;
                state.term.send_paste(&text).map_err(invalid)?;
            }
            Input::Key { event } => {
                use wezterm_term::KeyCode as K;
                let key = match event.code {
                    KeyCode::Char(ch) => K::Char(ch),
                    KeyCode::Enter => K::Enter,
                    KeyCode::Esc => K::Escape,
                    KeyCode::Backspace => K::Backspace,
                    KeyCode::Tab | KeyCode::BackTab => K::Tab,
                    KeyCode::Up => K::UpArrow,
                    KeyCode::Down => K::DownArrow,
                    KeyCode::Left => K::LeftArrow,
                    KeyCode::Right => K::RightArrow,
                    KeyCode::Home => K::Home,
                    KeyCode::End => K::End,
                    KeyCode::PageUp => K::PageUp,
                    KeyCode::PageDown => K::PageDown,
                    KeyCode::Delete => K::Delete,
                    KeyCode::Insert => K::Insert,
                    KeyCode::F(n) => K::Function(n),
                    _ => return Err(invalid("unsupported terminal key")),
                };
                let mut mods = modifiers(event.modifiers);
                if event.code == KeyCode::BackTab {
                    mods |= wezterm_term::KeyModifiers::SHIFT;
                }
                state.term.key_down(key, mods).map_err(invalid)?;
            }
            Input::Mouse { event } => {
                let (kind, button) = match event.kind {
                    MouseEventKind::Down(b) => {
                        (wezterm_term::MouseEventKind::Press, mouse_button(b))
                    }
                    MouseEventKind::Up(b) => {
                        (wezterm_term::MouseEventKind::Release, mouse_button(b))
                    }
                    MouseEventKind::Drag(b) => {
                        (wezterm_term::MouseEventKind::Move, mouse_button(b))
                    }
                    MouseEventKind::Moved => (
                        wezterm_term::MouseEventKind::Move,
                        wezterm_term::MouseButton::None,
                    ),
                    MouseEventKind::ScrollUp => (
                        wezterm_term::MouseEventKind::Press,
                        wezterm_term::MouseButton::WheelUp(1),
                    ),
                    MouseEventKind::ScrollDown => (
                        wezterm_term::MouseEventKind::Press,
                        wezterm_term::MouseButton::WheelDown(1),
                    ),
                    _ => return Ok(()),
                };
                state
                    .term
                    .mouse_event(wezterm_term::MouseEvent {
                        kind,
                        button,
                        x: event.column.into(),
                        y: event.row.into(),
                        x_pixel_offset: 0,
                        y_pixel_offset: 0,
                        modifiers: modifiers(event.modifiers),
                    })
                    .map_err(invalid)?;
            }
        }
        Ok(())
    }
}

fn modifiers(mods: KeyModifiers) -> wezterm_term::KeyModifiers {
    let mut result = wezterm_term::KeyModifiers::empty();
    if mods.contains(KeyModifiers::CONTROL) {
        result |= wezterm_term::KeyModifiers::CTRL;
    }
    if mods.contains(KeyModifiers::ALT) {
        result |= wezterm_term::KeyModifiers::ALT;
    }
    if mods.contains(KeyModifiers::SHIFT) {
        result |= wezterm_term::KeyModifiers::SHIFT;
    }
    result
}
fn mouse_button(button: MouseButton) -> wezterm_term::MouseButton {
    match button {
        MouseButton::Left => wezterm_term::MouseButton::Left,
        MouseButton::Right => wezterm_term::MouseButton::Right,
        MouseButton::Middle => wezterm_term::MouseButton::Middle,
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            if let Some(pid) = self.child.process_id() {
                // A PTY child normally leads its own session/process group.
                // Never signal a group unless it belongs to this child.
                unsafe {
                    if libc::getpgid(pid as i32) == pid as i32 {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
            }
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        if let Some((core, pid)) = &self.registration
            && let Some(core) = core.upgrade() {
                let _ = core
                    .store
                    .db()
                    .execute("DELETE FROM native_processes WHERE pid=?1", [*pid]);
            }
    }
}

impl Core {
    pub fn terminal_open(
        &self,
        conversation: &str,
        role: &str,
        columns: u16,
        rows: u16,
        retry: bool,
    ) -> Result<Screen> {
        size(columns, rows)?;
        let native = match self.native.get(conversation, role)? {
            Some(native) if !retry || native.check_alive().is_ok() => native,
            _ => self.native.restore(self, conversation, role)?,
        };
        native.check_alive()?;
        require(
            !matches!(native.backend, native::Backend::Codex { .. })
                || native.started.load(std::sync::atomic::Ordering::SeqCst),
            "waiting for the first native turn to create the Codex rollout",
        )?;
        let mut terminal = native.terminal.lock().map_err(invalid)?;
        if retry
            && native
                .gateway_error
                .lock()
                .map_err(invalid)?
                .take()
                .is_some()
        {
            *terminal = None;
        }
        if retry
            && terminal
                .as_ref()
                .is_some_and(|t| t.state.lock().expect("terminal").error.is_some())
        {
            *terminal = None;
        }
        if terminal.is_none() {
            *terminal = Some(Session::spawn(&native, columns, rows)?);
        }
        let terminal = terminal.as_mut().expect("terminal");
        {
            let mut state = terminal.state.lock().map_err(invalid)?;
            let dimensions = size(columns, rows)?;
            if state.term.get_size() != dimensions {
                terminal
                    .master
                    .resize(pty_size(columns, rows))
                    .map_err(invalid)?;
                state.term.resize(dimensions);
                state.revision = state.revision.wrapping_add(1);
            }
        }
        terminal
            .handles
            .retain(|_, handle| handle.seen.elapsed() < std::time::Duration::from_secs(30));
        require(
            terminal.handles.len() < 8,
            "too many attached terminal views; detach an existing view",
        )?;
        let handle = crate::id();
        terminal.handles.insert(
            handle.clone(),
            Handle {
                sequence: 0,
                seen: std::time::Instant::now(),
            },
        );
        terminal
            .snapshot(&handle, None)?
            .ok_or_else(|| invalid("terminal has no screen"))
    }
    pub fn terminal_read(
        &self,
        conversation: &str,
        role: &str,
        handle: &str,
        after: u64,
    ) -> Result<Option<Screen>> {
        let native = self
            .native
            .get(conversation, role)?
            .ok_or_else(|| invalid("native session ended"))?;
        if let Some(error) = native.gateway_error.lock().map_err(invalid)?.as_ref() {
            return Err(invalid(format!("native attachment failed: {error}")));
        }
        let mut terminal = native.terminal.lock().map_err(invalid)?;
        terminal
            .as_mut()
            .ok_or_else(|| invalid("terminal is not open"))?
            .snapshot(handle, Some(after))
    }
    pub fn terminal_input(
        &self,
        conversation: &str,
        role: &str,
        handle: &str,
        sequence: u64,
        input: Input,
    ) -> Result<()> {
        let native = self
            .native
            .get(conversation, role)?
            .ok_or_else(|| invalid("native session ended"))?;
        let mut terminal = native.terminal.lock().map_err(invalid)?;
        terminal
            .as_mut()
            .ok_or_else(|| invalid("terminal is not open"))?
            .input(handle, sequence, input)
    }
    pub fn terminal_resize(
        &self,
        conversation: &str,
        role: &str,
        handle: &str,
        columns: u16,
        rows: u16,
    ) -> Result<()> {
        let dimensions = size(columns, rows)?;
        let native = self
            .native
            .get(conversation, role)?
            .ok_or_else(|| invalid("native session ended"))?;
        let terminal = native.terminal.lock().map_err(invalid)?;
        let terminal = terminal
            .as_ref()
            .ok_or_else(|| invalid("terminal is not open"))?;
        require(
            terminal.handles.contains_key(handle),
            "unknown or detached terminal handle",
        )?;
        terminal
            .master
            .resize(pty_size(columns, rows))
            .map_err(invalid)?;
        let mut state = terminal.state.lock().map_err(invalid)?;
        state.term.resize(dimensions);
        state.revision = state.revision.wrapping_add(1);
        Ok(())
    }
    pub fn terminal_detach(&self, conversation: &str, role: &str, handle: &str) -> Result<()> {
        if let Some(native) = self.native.get(conversation, role)?
            && let Some(terminal) = native.terminal.lock().map_err(invalid)?.as_mut()
        {
            terminal.handles.remove(handle);
        }
        Ok(())
    }
}

/// Drop string controls (including bitmap protocols) before the emulator can
/// allocate image buffers. Keep ANSI/CSI streaming, with a bounded sequence.
#[derive(Default)]
struct Filter {
    mode: u8,
    sequence: Vec<u8>,
}

/// wezterm flushes its printable run at each advance_bytes call. Retain the
/// last extended cluster across reads and update its cell when a combining
/// mark or ZWJ continuation arrives, including after a screen was published.
#[derive(Default)]
struct GraphemeStream {
    mode: u8,
    utf8: Vec<u8>,
    tail: Option<Tail>,
    dimensions: Option<(usize, usize)>,
}
struct Tail {
    text: String,
    position: Option<(usize, i64)>,
    width: usize,
}
impl GraphemeStream {
    fn feed(&mut self, term: &mut Terminal, bytes: &[u8]) -> Result<()> {
        let size = term.get_size();
        let dimensions = (size.cols, size.rows);
        if self.dimensions.is_some_and(|old| old != dimensions)
            && let Some(tail) = &mut self.tail
        {
            let cursor = term.cursor_pos();
            let top = term.screen().scrollback_or_visible_row(0);
            tail.position = term
                .screen()
                .lines_in_phys_range(top..top + size.rows)
                .iter()
                .enumerate()
                .flat_map(|(y, line)| {
                    line.visible_cells()
                        .filter(|cell| cell.str() == tail.text)
                        .map(move |cell| (cell.cell_index(), y as i64))
                })
                .min_by_key(|(x, y)| {
                    y.abs_diff(cursor.y) as usize * size.cols + (x + tail.width).abs_diff(cursor.x)
                });
            if tail.position.is_none() {
                self.tail = None;
            }
        }
        self.dimensions = Some(dimensions);
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            if self.mode != 0 {
                term.advance_bytes([byte]);
                if self.mode == 1 {
                    if byte == b'[' {
                        self.mode = 2;
                    } else if !(0x20..=0x2f).contains(&byte) {
                        self.mode = 0;
                    }
                } else if (0x40..=0x7e).contains(&byte) {
                    self.mode = 0;
                }
                index += 1;
                continue;
            }
            if byte < 0x20 || byte == 0x7f {
                if !self.utf8.is_empty() {
                    self.utf8.clear();
                    self.character(term, '\u{fffd}')?;
                }
                self.tail = None;
                term.advance_bytes([byte]);
                if byte == 27 {
                    self.mode = 1;
                }
                index += 1;
                continue;
            }
            if self.utf8.is_empty() && byte.is_ascii() {
                let end = index
                    + bytes[index..]
                        .iter()
                        .take_while(|b| **b >= 0x20 && **b < 0x7f)
                        .count();
                let run = &bytes[index..end];
                let mut start = 0;
                if self.tail.as_ref().is_some_and(|tail| {
                    format!("{}{}", tail.text, run[0] as char)
                        .graphemes(true)
                        .count()
                        == 1
                }) {
                    self.character(term, run[0] as char)?;
                    start = 1;
                }
                if start < run.len() {
                    let rest = &run[start..];
                    if rest.len() > 1 {
                        self.tail = None;
                        term.advance_bytes(&rest[..rest.len() - 1]);
                    }
                    self.character(term, rest[rest.len() - 1] as char)?;
                }
                index = end;
                continue;
            }
            self.utf8.push(byte);
            index += 1;
            loop {
                match std::str::from_utf8(&self.utf8) {
                    Ok(text) => {
                        let text = text.to_owned();
                        self.utf8.clear();
                        for ch in text.chars() {
                            self.character(term, ch)?;
                        }
                        break;
                    }
                    Err(error) if error.error_len().is_none() => break,
                    Err(error) => {
                        let remove = (error.valid_up_to() + error.error_len().unwrap_or(1))
                            .min(self.utf8.len());
                        self.utf8.drain(..remove);
                        self.character(term, '\u{fffd}')?;
                        if self.utf8.is_empty() {
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn character(&mut self, term: &mut Terminal, ch: char) -> Result<()> {
        let mut text = ch.to_string();
        if let Some(previous) = self.tail.take() {
            let combined = format!("{}{ch}", previous.text);
            if combined.graphemes(true).count() == 1 {
                require(
                    combined.len() <= 4096,
                    "native terminal grapheme exceeds the 4 KiB cell limit",
                )?;
                let width = wezterm_term::grapheme_column_width(
                    &combined,
                    Some(&wezterm_term::UnicodeVersion::new(14)),
                );
                if let Some((x, y)) = previous.position {
                    if width == previous.width {
                        let attributes = term
                            .screen_mut()
                            .get_cell(x, y)
                            .map(|cell| cell.attrs().clone());
                        if let Some(attributes) = attributes {
                            term.increment_seqno();
                            let sequence = term.current_seqno();
                            term.screen_mut().set_cell(
                                x,
                                y,
                                &wezterm_term::Cell::new_grapheme_with_width(
                                    &combined, width, attributes,
                                ),
                                sequence,
                            );
                            self.tail = Some(Tail {
                                text: combined,
                                position: Some((x, y)),
                                width,
                            });
                            return Ok(());
                        }
                    }
                    // A variation selector or a regional-indicator pair can
                    // change width. Reprint from its original cell so the
                    // emulator applies its own wrapping and cursor rules.
                    term.advance_bytes(format!("\x1b[{};{}H", y + 1, x + 1));
                }
                text = combined;
            }
        }
        let width = wezterm_term::grapheme_column_width(
            &text,
            Some(&wezterm_term::UnicodeVersion::new(14)),
        );
        let before = term.cursor_pos();
        if width > 0 {
            term.advance_bytes(text.as_bytes());
        }
        let after = term.cursor_pos();
        let columns = term.get_size().cols;
        let mut candidates = vec![];
        if before.y == after.y && after.x >= before.x && before.x + width <= columns {
            candidates.push(before.x);
        }
        candidates.extend([
            after.x.saturating_sub(width),
            (after.x + 1).saturating_sub(width),
        ]);
        let position = if width == 0 {
            None
        } else {
            candidates
                .into_iter()
                .filter(|x| *x < columns)
                .find(|x| {
                    term.screen_mut()
                        .get_cell(*x, after.y)
                        .is_some_and(|cell| cell.str() == text)
                })
                .map(|x| (x, after.y))
        };
        self.tail = Some(Tail {
            text,
            position,
            width,
        });
        Ok(())
    }
}
impl Filter {
    fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            match self.mode {
                0 => {
                    if byte == 27 {
                        self.mode = 1;
                    } else {
                        output.push(byte);
                    }
                }
                1 => match byte {
                    b'[' => {
                        self.sequence = vec![27, b'['];
                        self.mode = 2;
                    }
                    b']' | b'P' | b'_' | b'^' => self.mode = 3,
                    _ => {
                        output.extend_from_slice(&[27, byte]);
                        self.mode = 0;
                    }
                },
                2 => {
                    if self.sequence.len() < 1024 {
                        self.sequence.push(byte);
                    } else {
                        self.sequence.clear();
                        self.mode = 5;
                    }
                    if (0x40..=0x7e).contains(&byte) {
                        output.append(&mut self.sequence);
                        self.mode = 0;
                    }
                }
                3 => {
                    if byte == 7 {
                        self.mode = 0;
                    } else if byte == 27 {
                        self.mode = 4;
                    }
                }
                4 => {
                    if byte == b'\\' || byte == 7 {
                        self.mode = 0;
                    } else if byte != 27 {
                        self.mode = 3;
                    }
                }
                _ => {
                    if (0x40..=0x7e).contains(&byte) {
                        self.mode = 0;
                    }
                }
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn emulator(columns: usize, rows: usize) -> Terminal {
        Terminal::new(
            TerminalSize {
                cols: columns,
                rows,
                ..Default::default()
            },
            Arc::new(Config),
            "antislop",
            "test",
            Box::new(std::io::sink()),
        )
    }
    fn text(term: &Terminal) -> String {
        let top = term.screen().scrollback_or_visible_row(0);
        term.screen()
            .lines_in_phys_range(top..top + term.get_size().rows)
            .iter()
            .map(|line| {
                line.visible_cells()
                    .map(|cell| cell.str().to_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn ansi_and_utf8_sequences_survive_split_reads_and_alternate_screen() {
        let mut term = emulator(40, 4);
        let mut filter = Filter::default();
        for bytes in [
            b"\x1b[3".as_slice(),
            b"1mred\x1b[0",
            b"m \xf0\x9f",
            b"\x90\xa2",
        ] {
            term.advance_bytes(filter.feed(bytes));
        }
        assert!(text(&term).contains("red 🐢"));
        term.advance_bytes(b"\x1b[?1049hALT");
        assert!(text(&term).contains("ALT"));
        term.advance_bytes(b"\x1b[?1049l");
        assert!(text(&term).contains("red 🐢"));
    }
    #[test]
    fn unicode_clusters_and_screen_storage_stay_bounded() {
        let mut term = emulator(40, 3);
        term.advance_bytes("👩‍💻 e\u{301} 漢".as_bytes());
        assert!(text(&term).contains("👩‍💻"));
        assert!(text(&term).contains("e\u{301}"));
        for _ in 0..1000 {
            term.advance_bytes(b"line\r\n");
        }
        assert!(term.screen().scrollback_rows() <= 3);
        assert!(term.screen().lines_in_phys_range(0..10).len() <= 3);
    }
    #[test]
    fn zwj_and_combining_clusters_survive_byte_by_byte_output() {
        let mut term = emulator(40, 3);
        let mut stream = GraphemeStream::default();
        for byte in "👩‍💻 e\u{301}".as_bytes() {
            stream.feed(&mut term, &[*byte]).unwrap();
            let _ = text(&term); // Publishing a screen cannot split the cluster.
        }
        assert!(text(&term).contains("👩‍💻"));
        assert!(text(&term).contains("e\u{301}"));
    }
    #[test]
    fn a_cluster_can_continue_after_a_resize_reflows_its_cell() {
        let mut term = emulator(40, 4);
        let mut stream = GraphemeStream::default();
        stream.feed(&mut term, "12345👩".as_bytes()).unwrap();
        term.resize(TerminalSize {
            cols: 4,
            rows: 4,
            ..Default::default()
        });
        stream.feed(&mut term, "‍💻".as_bytes()).unwrap();
        let text = text(&term);
        assert!(text.contains("👩‍💻"));
        assert!(!text.replace("👩‍💻", "").contains("👩"));
    }
    #[test]
    fn string_controls_and_oversized_sequences_cannot_allocate_image_buffers() {
        let mut filter = Filter::default();
        let mut term = emulator(20, 2);
        term.advance_bytes(filter.feed(b"before\x1b]1337;File=huge"));
        term.advance_bytes(filter.feed(&vec![b'x'; 1024 * 1024]));
        term.advance_bytes(filter.feed(b"\x1b\\after"));
        assert!(text(&term).contains("beforeafter"));
        assert!(filter.sequence.len() <= 1024);
        let mut huge = vec![27, b'['];
        huge.extend_from_slice(&vec![b'1'; 4096]);
        huge.extend_from_slice(b"mOK");
        term.advance_bytes(filter.feed(&huge));
        assert!(text(&term).contains("OK"));
    }
    #[test]
    fn terminal_dimensions_are_bounded_before_allocation() {
        assert!(size(0, 1).is_err());
        assert!(size(501, 20).is_err());
        assert!(size(80, 201).is_err());
        assert!(size(12, 5).is_ok());
    }
}
