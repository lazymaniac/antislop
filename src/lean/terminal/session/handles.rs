use super::super::*;

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
            self.master
                .resize(pty_size(columns, rows))
                .map_err(invalid)?;
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
