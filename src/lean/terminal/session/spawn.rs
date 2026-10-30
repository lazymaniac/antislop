use super::super::*;
use crate::lean::{process_tree, store};

struct PtyChild {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    registration: Option<(std::sync::Weak<Core>, u32)>,
    marker: String,
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
        let pty = spawn_child(command, core, provider, columns, rows)?;
        let state = new_state(&*pty.master, dimensions)?;
        Ok(Self {
            master: pty.master,
            child: pty.child,
            state,
            handles: HashMap::new(),
            registration: pty.registration,
            marker: pty.marker,
        })
    }
}

fn spawn_child(
    mut command: CommandBuilder,
    core: &std::sync::Weak<Core>,
    provider: &str,
    columns: u16,
    rows: u16,
) -> Result<PtyChild> {
    let marker = process_marker(core)?;
    command.env(process_tree::MARKER, &marker);
    let pair = NativePtySystem::default()
        .openpty(pty_size(columns, rows))
        .map_err(invalid)?;
    let mut child = pair.slave.spawn_command(command).map_err(invalid)?;
    let registration = register_child(&mut child, core, provider)?;
    drop(pair.slave);
    Ok(PtyChild {
        master: pair.master,
        child,
        registration,
        marker,
    })
}

fn process_marker(core: &std::sync::Weak<Core>) -> Result<String> {
    core.upgrade()
        .map(|core| process_tree::marker(&core.state))
        .ok_or_else(|| invalid("daemon stopped"))
}

fn register_child(
    child: &mut Box<dyn portable_pty::Child + Send + Sync>,
    core: &std::sync::Weak<Core>,
    provider: &str,
) -> Result<Option<(std::sync::Weak<Core>, u32)>> {
    let Some(pid) = child.process_id() else {
        return Ok(None);
    };
    let Some(core) = core.upgrade() else {
        let _ = child.kill();
        return Err(invalid("daemon stopped"));
    };
    if let Err(error) = core.store.db().execute(
        "INSERT OR REPLACE INTO native_processes(pid,started_ms,provider) VALUES(?1,?2,?3)",
        rusqlite::params![pid, store::now_ms(), provider],
    ) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error.into());
    }
    Ok(Some((Arc::downgrade(&core), pid)))
}

fn new_state(master: &dyn MasterPty, dimensions: TerminalSize) -> Result<Arc<Mutex<State>>> {
    let writer = master.take_writer().map_err(invalid)?;
    let reader = master.try_clone_reader().map_err(invalid)?;
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
    start_writer(writer, rx, Arc::downgrade(&state));
    start_reader_thread(reader, Arc::downgrade(&state));
    Ok(state)
}

fn start_writer(
    mut writer: Box<dyn Write + Send>,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    state: std::sync::Weak<Mutex<State>>,
) {
    std::thread::spawn(move || {
        while let Ok(bytes) = rx.recv() {
            if let Err(error) = writer.write_all(&bytes) {
                if let Some(state) = state.upgrade() {
                    state.lock().expect("terminal").error = Some(error.to_string());
                }
                break;
            }
        }
    });
}

fn start_reader_thread(reader: Box<dyn Read + Send>, state: std::sync::Weak<Mutex<State>>) {
    std::thread::spawn(move || read_terminal(reader, state));
}

fn read_terminal(mut reader: Box<dyn Read + Send>, state: std::sync::Weak<Mutex<State>>) {
    let mut buffer = [0; 8192];
    let mut filter = Filter::default();
    let mut graphemes = GraphemeStream::default();
    while let Some(count) = read_chunk(reader.as_mut(), &mut buffer) {
        let Some(shared) = state.upgrade() else {
            break;
        };
        if !update_terminal(&shared, &mut graphemes, &mut filter, &buffer[..count]) {
            break;
        }
    }
    disconnect_terminal(&state);
}

fn read_chunk(reader: &mut dyn Read, buffer: &mut [u8]) -> Option<usize> {
    match reader.read(buffer) {
        Ok(0) | Err(_) => None,
        Ok(count) => Some(count),
    }
}

fn update_terminal(
    state: &Arc<Mutex<State>>,
    graphemes: &mut GraphemeStream,
    filter: &mut Filter,
    bytes: &[u8],
) -> bool {
    let mut state = state.lock().expect("terminal");
    if let Err(error) = graphemes.feed(&mut state.term, &filter.feed(bytes)) {
        state.error = Some(error.to_string());
        return false;
    }
    state.revision = state.revision.wrapping_add(1);
    state.output = std::time::Instant::now();
    true
}

fn disconnect_terminal(state: &std::sync::Weak<Mutex<State>>) {
    if let Some(state) = state.upgrade() {
        let mut state = state.lock().expect("terminal");
        state
            .error
            .get_or_insert_with(|| "native terminal disconnected; retry attachment".into());
        state.revision = state.revision.wrapping_add(1);
    }
}
