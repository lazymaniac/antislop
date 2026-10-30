use super::super::*;
use crate::lean::process_tree;
impl Session {
    pub(super) fn snapshot(&mut self, handle: &str, after: Option<u64>) -> Result<Option<Screen>> {
        let handle_state = self
            .handles
            .get_mut(handle)
            .ok_or_else(|| invalid("unknown or detached terminal handle"))?;
        handle_state.seen = std::time::Instant::now();
        let state = self.state.lock().map_err(invalid)?;
        if after == Some(state.revision) {
            return Ok(None);
        }
        Ok(Some(render_screen(handle, &state)))
    }

    pub(super) fn input(&mut self, handle: &str, sequence: u64, input: Input) -> Result<()> {
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
        apply_input(&mut state, input)?;
        last.sequence = sequence;
        Ok(())
    }
}

fn render_screen(handle: &str, state: &State) -> Screen {
    let dimensions = state.term.get_size();
    let cursor = state.term.cursor_pos();
    Screen {
        handle: handle.into(),
        revision: state.revision,
        columns: dimensions.cols as u16,
        rows: dimensions.rows as u16,
        cursor: if format!("{:?}", cursor.visibility) == "Visible" {
            Some((cursor.x as u16, cursor.y.max(0) as u16))
        } else {
            None
        },
        cells: render_cells(state, dimensions.rows, dimensions.cols),
        error: state.error.clone(),
    }
}

fn render_cells(state: &State, rows: usize, columns: usize) -> Vec<Cell> {
    let top = state.term.screen().scrollback_or_visible_row(0);
    let lines = state.term.screen().lines_in_phys_range(top..top + rows);
    let palette = state.term.palette();
    let mut cells = Vec::with_capacity(rows * columns);
    for (y, line) in lines.iter().enumerate() {
        for cell in line
            .visible_cells()
            .take_while(|cell| cell.cell_index() < columns)
        {
            cells.push(render_cell(cell, y, &palette));
        }
    }
    cells
}

fn render_cell(
    cell: wezterm_term::CellRef<'_>,
    y: usize,
    palette: &wezterm_term::color::ColorPalette,
) -> Cell {
    let attributes = cell.attrs();
    Cell {
        x: cell.cell_index() as u16,
        y: y as u16,
        text: cell.str().into(),
        width: cell.width() as u16,
        foreground: resolve_color(palette, attributes.foreground(), true),
        background: resolve_color(palette, attributes.background(), false),
        modifiers: cell_modifiers(attributes),
    }
}

fn resolve_color(
    palette: &wezterm_term::color::ColorPalette,
    attribute: wezterm_term::color::ColorAttribute,
    foreground: bool,
) -> Option<[u8; 3]> {
    use wezterm_term::color::ColorAttribute::Default;
    if attribute == Default {
        return None;
    }
    let (r, g, b, _) = if foreground {
        palette.resolve_fg(attribute)
    } else {
        palette.resolve_bg(attribute)
    }
    .to_srgb_u8();
    Some([r, g, b])
}

fn cell_modifiers(attributes: &wezterm_term::CellAttributes) -> u16 {
    let mut flags = 0;
    if attributes.intensity() == wezterm_term::Intensity::Bold {
        flags |= 1;
    }
    if attributes.intensity() == wezterm_term::Intensity::Half {
        flags |= 2;
    }
    if attributes.italic() {
        flags |= 4;
    }
    if attributes.underline() != wezterm_term::Underline::None {
        flags |= 8;
    }
    if attributes.reverse() {
        flags |= 16;
    }
    flags
}

fn apply_input(state: &mut State, input: Input) -> Result<()> {
    match input {
        Input::Paste { text } => apply_paste(state, &text),
        Input::Key { event } => apply_key(state, event),
        Input::Mouse { event } => apply_mouse(state, event),
    }
}

fn apply_paste(state: &mut State, text: &str) -> Result<()> {
    require(text.len() <= 1024 * 1024, "terminal paste exceeds 1 MiB")?;
    state.term.send_paste(text).map_err(invalid)
}

fn apply_key(state: &mut State, event: KeyEvent) -> Result<()> {
    let key = terminal_key(event.code)?;
    let mut mods = modifiers(event.modifiers);
    if event.code == KeyCode::BackTab {
        mods |= wezterm_term::KeyModifiers::SHIFT;
    }
    if event.kind == crossterm::event::KeyEventKind::Release {
        state.term.key_up(key, mods).map_err(invalid)
    } else {
        state.term.key_down(key, mods).map_err(invalid)
    }
}

fn terminal_key(code: KeyCode) -> Result<wezterm_term::KeyCode> {
    use wezterm_term::KeyCode as K;
    Ok(match code {
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
    })
}

fn apply_mouse(state: &mut State, event: MouseEvent) -> Result<()> {
    let (kind, button) = mouse_event(event.kind);
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
        .map_err(invalid)
}

fn mouse_event(kind: MouseEventKind) -> (wezterm_term::MouseEventKind, wezterm_term::MouseButton) {
    use wezterm_term::{MouseButton as B, MouseEventKind as K};
    match kind {
        MouseEventKind::Down(b) => (K::Press, mouse_button(b)),
        MouseEventKind::Up(b) => (K::Release, mouse_button(b)),
        MouseEventKind::Drag(b) => (K::Move, mouse_button(b)),
        MouseEventKind::Moved => (K::Move, B::None),
        MouseEventKind::ScrollUp => (K::Press, B::WheelUp(1)),
        MouseEventKind::ScrollDown => (K::Press, B::WheelDown(1)),
        MouseEventKind::ScrollLeft => (K::Press, B::WheelLeft(1)),
        MouseEventKind::ScrollRight => (K::Press, B::WheelRight(1)),
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
        let alive = self.child.try_wait().ok().flatten().is_none();
        // Before the CLI dies and its commands lose their parent link.
        let pid = self.child.process_id().filter(|_| alive);
        process_tree::kill_descendants(pid.map(|pid| pid as i32), &self.marker);
        if alive {
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
