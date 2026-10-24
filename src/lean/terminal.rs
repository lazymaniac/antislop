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
pub(crate) struct State {
    term: Terminal,
    revision: u64,
    error: Option<String>,
    /// When the CLI last printed anything.
    output: std::time::Instant,
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
            // WezTerm calls this on its own writer thread. Apply backpressure
            // there: WouldBlock would terminate that thread and lose later
            // keystrokes even after the PTY became writable again.
            .send(bytes.to_vec())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::BrokenPipe, e))?;
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
    /// Starts `command` in a new terminal that the daemon owns for as long
    /// as the session lives; views only attach to it.
    pub(crate) fn spawn(
        command: CommandBuilder,
        core: &std::sync::Weak<Core>,
        provider: &str,
        columns: u16,
        rows: u16,
    ) -> Result<Self> {
        let dimensions = size(columns, rows)?;
        let pair = NativePtySystem::default()
            .openpty(pty_size(columns, rows))
            .map_err(invalid)?;
        let mut child = pair.slave.spawn_command(command).map_err(invalid)?;
        let registration = child.process_id().map(|pid| (core.clone(), pid));
        drop(pair.slave);
        if let Some(pid) = child.process_id() {
            let Some(core) = core.upgrade() else {
                let _ = child.kill();
                return Err(invalid("daemon stopped"));
            };
            if let Err(error) = core.store.db().execute(
                "INSERT OR REPLACE INTO native_processes(pid,started_ms,provider) VALUES(?1,?2,?3)",
                rusqlite::params![pid, super::store::now_ms(), provider],
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
            output: std::time::Instant::now(),
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
                state.output = std::time::Instant::now();
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
                if event.kind == crossterm::event::KeyEventKind::Release {
                    state.term.key_up(key, mods).map_err(invalid)?;
                } else {
                    state.term.key_down(key, mods).map_err(invalid)?;
                }
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
                    MouseEventKind::ScrollLeft => (
                        wezterm_term::MouseEventKind::Press,
                        wezterm_term::MouseButton::WheelLeft(1),
                    ),
                    MouseEventKind::ScrollRight => (
                        wezterm_term::MouseEventKind::Press,
                        wezterm_term::MouseButton::WheelRight(1),
                    ),
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
        last.sequence = sequence;
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
            && let Some(core) = core.upgrade()
        {
            let _ = core
                .store
                .db()
                .execute("DELETE FROM native_processes WHERE pid=?1", [*pid]);
        }
    }
}

impl Core {
    /// Attaches a view to the role's terminal. A role without a running CLI
    /// (idle and released, or from an earlier round) is started again with
    /// `resume`; that is the only way to look at its past session.
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
        native.touch();
        let mut terminal = native.terminal.lock().map_err(invalid)?;
        terminal.attach(columns, rows)
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
        native.touch();
        let mut terminal = native.terminal.lock().map_err(invalid)?;
        terminal.snapshot(handle, Some(after))
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
        native.touch();
        let mut terminal = native.terminal.lock().map_err(invalid)?;
        terminal.input(handle, sequence, input)
    }
    pub fn terminal_resize(
        &self,
        conversation: &str,
        role: &str,
        handle: &str,
        columns: u16,
        rows: u16,
    ) -> Result<()> {
        size(columns, rows)?;
        let native = self
            .native
            .get(conversation, role)?
            .ok_or_else(|| invalid("native session ended"))?;
        let terminal = native.terminal.lock().map_err(invalid)?;
        require(
            terminal.handles.contains_key(handle),
            "unknown or detached terminal handle",
        )?;
        terminal.resize(columns, rows)
    }
    pub fn terminal_detach(&self, conversation: &str, role: &str, handle: &str) -> Result<()> {
        if let Some(native) = self.native.get(conversation, role)? {
            native
                .terminal
                .lock()
                .map_err(invalid)?
                .handles
                .remove(handle);
            native.touch();
        }
        Ok(())
    }
}

impl Session {
    /// Adds a view handle and fits the terminal to the view.
    fn attach(&mut self, columns: u16, rows: u16) -> Result<Screen> {
        self.resize(columns, rows)?;
        self.handles
            .retain(|_, handle| handle.seen.elapsed() < std::time::Duration::from_secs(30));
        require(
            self.handles.len() < 8,
            "too many attached terminal views; detach an existing view",
        )?;
        let handle = crate::id();
        self.handles.insert(
            handle.clone(),
            Handle {
                sequence: 0,
                seen: std::time::Instant::now(),
            },
        );
        self.snapshot(&handle, None)?
            .ok_or_else(|| invalid("terminal has no screen"))
    }

    fn resize(&self, columns: u16, rows: u16) -> Result<()> {
        let dimensions = size(columns, rows)?;
        let mut state = self.state.lock().map_err(invalid)?;
        if state.term.get_size() != dimensions {
            self.master.resize(pty_size(columns, rows)).map_err(invalid)?;
            state.term.resize(dimensions);
            state.revision = state.revision.wrapping_add(1);
        }
        Ok(())
    }

    /// Whether any view is attached (views that stopped polling expire).
    pub(crate) fn attached(&mut self) -> bool {
        self.handles
            .retain(|_, handle| handle.seen.elapsed() < std::time::Duration::from_secs(30));
        !self.handles.is_empty()
    }

    /// Types `text` into the CLI as the user would: one bracketed paste, then
    /// Enter. A pause lets the CLI take the paste in before it is submitted.
    pub(crate) fn submit(state: &Arc<Mutex<State>>, text: &str) -> Result<()> {
        use wezterm_term::{KeyCode as K, KeyModifiers as M};
        {
            let mut state = state.lock().map_err(invalid)?;
            require(
                state.error.is_none(),
                state.error.as_deref().unwrap_or("terminal failed"),
            )?;
            state.term.send_paste(text).map_err(invalid)?;
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
        let mut state = state.lock().map_err(invalid)?;
        state.term.key_down(K::Enter, M::NONE).map_err(invalid)?;
        state.term.key_up(K::Enter, M::NONE).map_err(invalid)?;
        Ok(())
    }

    /// Asks the CLI to stop what it is doing (Escape, twice for CLIs that
    /// want a confirming press).
    pub(crate) fn interrupt(state: &Arc<Mutex<State>>) -> Result<()> {
        use wezterm_term::{KeyCode as K, KeyModifiers as M};
        for _ in 0..2 {
            {
                let mut state = state.lock().map_err(invalid)?;
                state.term.key_down(K::Escape, M::NONE).map_err(invalid)?;
                state.term.key_up(K::Escape, M::NONE).map_err(invalid)?;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        Ok(())
    }

    pub(crate) fn shared(&self) -> Arc<Mutex<State>> {
        self.state.clone()
    }

    /// The CLI's exit status once it ended.
    pub(crate) fn exited(&mut self) -> Option<String> {
        self.child
            .try_wait()
            .ok()
            .flatten()
            .map(|status| status.to_string())
    }

    pub(crate) fn error(&self) -> Option<String> {
        self.state.lock().ok()?.error.clone()
    }
}

/// Time since the CLI last printed.
pub(crate) fn quiet_for(state: &Arc<Mutex<State>>) -> std::time::Duration {
    state
        .lock()
        .map_or(std::time::Duration::ZERO, |state| state.output.elapsed())
}

/// The last non-empty screen lines as plain text, for explaining why a turn
/// did not end (an approval question, a login prompt, a crash).
pub(crate) fn tail(state: &Arc<Mutex<State>>, count: usize) -> String {
    let Ok(state) = state.lock() else {
        return String::new();
    };
    let dimensions = state.term.get_size();
    let top = state.term.screen().scrollback_or_visible_row(0);
    let mut lines = state
        .term
        .screen()
        .lines_in_phys_range(top..top + dimensions.rows)
        .iter()
        .map(|line| {
            line.visible_cells()
                .map(|cell| cell.str().to_owned())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let keep = lines.len().saturating_sub(count);
    lines.drain(..keep);
    lines.join("\n")
}

/// Drop string controls (including bitmap protocols) before the emulator can
/// allocate image buffers. Keep ANSI/CSI streaming, with a bounded sequence.
#[derive(Default)]
struct Filter {
    mode: u8,
    sequence: Vec<u8>,
    osc: bool,
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
                    } else if byte == b']' {
                        self.mode = 3;
                    } else if !(0x20..=0x2f).contains(&byte) {
                        self.mode = 0;
                    }
                } else if self.mode == 3 {
                    if byte == 7 {
                        self.mode = 0;
                    } else if byte == 27 {
                        self.mode = 4;
                    }
                } else if self.mode == 4 {
                    self.mode = if byte == b'\\' { 0 } else { 3 };
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
    fn finish_string(&mut self, terminator: &[u8], output: &mut Vec<u8>) {
        // Preserve bounded palette queries needed by native CLI theme
        // detection. Graphics, clipboard and other string controls stay out.
        if self.osc && self.sequence.starts_with(b"\x1b]") {
            let query = std::str::from_utf8(&self.sequence[2..]).unwrap_or("");
            let parts: Vec<_> = query.split(';').collect();
            let dynamic = matches!(parts.as_slice(), ["10" | "11" | "12", "?"]);
            let indexed =
                matches!(parts.as_slice(), ["4", index, "?"] if index.parse::<u8>().is_ok());
            if dynamic || indexed {
                output.append(&mut self.sequence);
                output.extend_from_slice(terminator);
            }
        }
        self.sequence.clear();
        self.osc = false;
        self.mode = 0;
    }
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
                    b']' | b'P' | b'_' | b'^' => {
                        self.osc = byte == b']';
                        self.sequence.clear();
                        if self.osc {
                            self.sequence.extend_from_slice(b"\x1b]");
                        }
                        self.mode = 3;
                    }
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
                        self.finish_string(&[7], &mut output);
                    } else if byte == 27 {
                        self.mode = 4;
                    } else if self.osc {
                        if self.sequence.len() < 1024 {
                            self.sequence.push(byte);
                        } else {
                            self.sequence.clear();
                            self.osc = false;
                        }
                    }
                }
                4 => {
                    if byte == b'\\' || byte == 7 {
                        self.finish_string(&[27, byte], &mut output);
                    } else if byte != 27 {
                        self.osc = false;
                        self.sequence.clear();
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
    /// What a CLI that asked for bracketed paste receives for `text`.
    fn typed_into_a_cli(text: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        let received = dir.path().join("received");
        let script = format!(
            "import os,sys,tty\ntty.setraw(0)\nsys.stdout.write('\\x1b[?2004h');sys.stdout.flush()\n\
             buf=b''\nwhile True:\n d=os.read(0,65536)\n if not d: break\n buf+=d\n \
             if b'\\x1b[201~' in buf and b'\\r' in buf[buf.index(b'\\x1b[201~'):]:\n  \
             open({received:?},'wb').write(buf);break\n",
            received = received.to_string_lossy()
        );
        let config = crate::config::Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config).unwrap();
        let mut command = CommandBuilder::new("python3");
        command.args(["-c", &script]);
        let mut terminal =
            Session::spawn(command, core.weak.get().unwrap(), "python3", 120, 40).unwrap();
        let screen = terminal.shared();
        while quiet_for(&screen) < std::time::Duration::from_millis(600) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        Session::submit(&screen, text).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !received.exists() {
            assert!(std::time::Instant::now() < deadline, "nothing received");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        let bytes = std::fs::read(&received).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let start = text.find("\x1b[200~").unwrap() + 6;
        let end = text.find("\x1b[201~").unwrap();
        let _ = terminal.exited();
        text[start..end].to_owned()
    }

    #[test]
    fn a_typed_prompt_arrives_exactly_as_written() {
        let line = "{\"k\": \"zażółć gęślą jaźń 🙂 \\u00e9 tab\\there\"}";
        let big = format!("first\n{}\n\n  indented\ttab \"quoted\" 'single' `tick`\ntrailing\n", line.repeat(2000));
        let received = typed_into_a_cli(&big);
        assert_eq!(received.replace('\r', "\n"), big);
    }

    use super::*;
    #[test]
    fn split_palette_and_cursor_queries_receive_terminal_responses() {
        let (sender, received) = std::sync::mpsc::sync_channel(8);
        let mut term = Terminal::new(
            size(40, 4).unwrap(),
            Arc::new(Config),
            "test",
            "test",
            Box::new(Writer(sender)),
        );
        let mut filter = Filter::default();
        let mut graphemes = GraphemeStream::default();
        for &byte in b"\x1b]11;?\x1b\\\x1b[6n" {
            graphemes.feed(&mut term, &filter.feed(&[byte])).unwrap();
        }
        let mut output = String::new();
        for _ in 0..8 {
            output.push_str(
                &String::from_utf8(
                    received
                        .recv_timeout(std::time::Duration::from_secs(1))
                        .unwrap(),
                )
                .unwrap(),
            );
            if output.contains("11;rgb:") && output.contains("1;1R") {
                break;
            }
        }
        assert!(output.contains("11;rgb:"), "{output:?}");
        assert!(output.contains("1;1R"), "{output:?}");
        assert!(text(&term).trim().is_empty());
        assert!(filter.feed(b"\x1b]52;c;?\x07").is_empty());
    }
    #[test]
    fn writer_backpressure_keeps_the_complete_paste_and_later_input() {
        let (sender, received) = std::sync::mpsc::sync_channel(1);
        sender.send(b"occupied".to_vec()).unwrap();
        let mut writer = Writer(sender);
        let (ready, started) = std::sync::mpsc::sync_channel(1);
        let (finished, done) = std::sync::mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            ready.send(()).unwrap();
            let paste = "\x1b[200~draft 👩‍💻\x1b[201~";
            finished
                .send(writer.write_all(paste.as_bytes()).is_ok())
                .unwrap();
            writer.write_all(b"next").unwrap();
        });
        started.recv().unwrap();
        assert!(
            done.recv_timeout(std::time::Duration::from_millis(50))
                .is_err()
        );
        assert_eq!(received.recv().unwrap(), b"occupied");
        assert!(
            done.recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
        );
        assert_eq!(
            received.recv().unwrap(),
            "\x1b[200~draft 👩‍💻\x1b[201~".as_bytes()
        );
        assert_eq!(received.recv().unwrap(), b"next");
        worker.join().unwrap();
    }
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
