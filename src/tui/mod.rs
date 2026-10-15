//! Observable, detachable terminal frontend. The daemon owns run lifetimes.
pub mod app;
pub mod backend;
pub mod editor;
pub mod graph;
pub mod inspector;
pub mod model;
pub mod options;
pub mod render;
pub mod sanitize;
pub mod theme;

use app::{App, Effect};
use backend::{Action, Backend, IpcBackend};
use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use std::{
    io::{self, IsTerminal},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
    }
}
enum WorkerRequest {
    Select(String),
    Create(String),
    BeginConversation(String),
    Action(String, Action),
}
enum WorkerUpdate {
    Runs(Vec<backend::RunChoice>),
    Created(std::result::Result<String, String>),
    Conversation(
        String,
        std::result::Result<backend::ConversationStarted, String>,
    ),
    Snapshot(Box<model::Snapshot>),
    Action(Action, std::result::Result<String, String>),
    Error(String),
}

pub fn run(state: &Path) -> crate::Result<()> {
    run_selected(state, None)
}
pub fn run_selected(state: &Path, run_id: Option<&str>) -> crate::Result<()> {
    run_options(options::Options {
        state: state.into(),
        run_id: run_id.map(str::to_owned),
        orchestrator: None,
    })
}
pub fn run_options(options: options::Options) -> crate::Result<()> {
    crate::require(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "TUI requires a terminal; use antislop --help for CLI commands",
    )?;
    crate::require(
        options.run_id.is_none() || options.orchestrator.is_none(),
        "--orchestrator cannot change an attached run",
    )?;
    let _daemon = connect_or_start_daemon(&options.state, &std::env::current_exe()?)?;
    let mut backend = IpcBackend::new(&options.state);
    if let Some(provider) = options.orchestrator {
        backend = backend.with_bootstrap_provider(provider);
    }
    run_with_backend(backend, options.run_id)
}
/// Detaching the frontend does not stop its daemon. A refused connection to an
/// owned socket may start a daemon; only its locked server removes stale sockets.
pub fn connect_or_start_daemon(
    state: &Path,
    executable: &Path,
) -> crate::Result<Option<std::process::Child>> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    if let Ok(metadata) = std::fs::symlink_metadata(state) {
        crate::require(
            metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == unsafe { libc::geteuid() },
            "unsafe UI state directory",
        )?;
        if metadata.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(state, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    crate::ipc::private_directory(state)?;
    let request = crate::ipc::Request {
        version: crate::ipc::PROTOCOL_VERSION,
        request_id: crate::id(),
        idempotency_key: None,
        operation: crate::ipc::Operation::List,
    };
    match state.join("architect.sock").symlink_metadata() {
        Ok(metadata) => {
            crate::require(
                metadata.file_type().is_socket() && metadata.uid() == unsafe { libc::geteuid() },
                "unsafe UI socket path",
            )?;
            match crate::ipc::call(state, &request) {
                Ok(response) => {
                    crate::require(response.error.is_none(), "daemon rejected UI connection")?;
                    return Ok(None);
                }
                Err(crate::Error::Io(error))
                    if matches!(
                        error.kind(),
                        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                    ) => {}
                Err(error) => return Err(error),
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    let mut child = std::process::Command::new(executable)
        .arg("daemon")
        .arg(state)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if state.join("architect.sock").exists() {
            match crate::ipc::call(state, &request) {
                Ok(response) => {
                    crate::require(response.error.is_none(), "daemon rejected UI connection")?;
                    return Ok(Some(child));
                }
                Err(crate::Error::Io(error))
                    if matches!(
                        error.kind(),
                        io::ErrorKind::ConnectionRefused
                            | io::ErrorKind::NotFound
                            | io::ErrorKind::ConnectionReset
                            | io::ErrorKind::ConnectionAborted
                            | io::ErrorKind::UnexpectedEof
                            | io::ErrorKind::BrokenPipe
                    ) => {}
                Err(crate::Error::Invalid(message)) if message == "daemon disconnected" => {}
                Err(error) => return Err(error),
            }
        }
        if let Some(status) = child.try_wait()? {
            return Err(crate::Error::Invalid(format!(
                "UI daemon exited: {status}. Run antislop daemon {} to inspect the error.",
                state.display()
            )));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(crate::Error::Invalid("UI daemon startup timed out".into()));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn run_with_backend<B: Backend>(
    mut backend: B,
    initial_run: Option<String>,
) -> crate::Result<()> {
    crate::require(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "TUI requires a terminal; use antislop --help for CLI commands",
    )?;
    // Install signal flags before entering raw mode. No terminal I/O happens in
    // signal handlers; the event loop returns through the restoration guard.
    let stop = Arc::new(AtomicBool::new(false));
    let mut registrations = vec![];
    for signal in [
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGHUP,
    ] {
        match signal_hook::flag::register(signal, stop.clone()) {
            Ok(id) => registrations.push(id),
            Err(error) => {
                for id in registrations {
                    signal_hook::low_level::unregister(id);
                }
                return Err(error.into());
            }
        }
    }
    struct Signals(Vec<signal_hook::SigId>);
    impl Drop for Signals {
        fn drop(&mut self) {
            for id in self.0.drain(..) {
                signal_hook::low_level::unregister(id);
            }
        }
    }
    let _signals = Signals(registrations);
    // The guard exists before the first terminal mutation, including failures.
    let _guard = TerminalGuard;
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
        previous(info);
    }));
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(io::stdout()))?;
    let (request_tx, request_rx) = mpsc::channel();
    let (update_tx, update_rx) = mpsc::channel();
    // IPC reads may wait for a daemon timeout; keep them off the input/render
    // thread so detach and signals always restore the terminal promptly.
    std::thread::spawn(move || {
        let mut selected = initial_run;
        let mut refresh = Instant::now() - Duration::from_secs(2);
        let mut list_refresh = refresh;
        loop {
            match request_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(WorkerRequest::Select(id)) => {
                    selected = Some(id);
                    refresh = Instant::now() - Duration::from_secs(2);
                }
                Ok(WorkerRequest::BeginConversation(text)) => {
                    let result = backend.begin_conversation(&text).map_err(|e| e.to_string());
                    if let Ok(started) = &result {
                        selected = Some(started.run_id.clone());
                        list_refresh = Instant::now() - Duration::from_secs(5);
                    }
                    if update_tx
                        .send(WorkerUpdate::Conversation(text, result))
                        .is_err()
                    {
                        break;
                    }
                    refresh = Instant::now() - Duration::from_secs(2);
                }
                Ok(WorkerRequest::Create(path)) => {
                    let result = backend
                        .create_run(Path::new(&path))
                        .map_err(|e| e.to_string());
                    if let Ok(id) = &result {
                        selected = Some(id.clone());
                        list_refresh = Instant::now() - Duration::from_secs(5);
                    }
                    if update_tx.send(WorkerUpdate::Created(result)).is_err() {
                        break;
                    }
                    refresh = Instant::now() - Duration::from_secs(2);
                }
                Ok(WorkerRequest::Action(id, action)) => {
                    let result = backend.act(&id, action.clone()).map_err(|e| e.to_string());
                    if update_tx
                        .send(WorkerUpdate::Action(action, result))
                        .is_err()
                    {
                        break;
                    }
                    refresh = Instant::now() - Duration::from_secs(2);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => (),
            }
            if list_refresh.elapsed() >= Duration::from_secs(5) {
                match backend.list_runs() {
                    Ok(runs) => {
                        if update_tx.send(WorkerUpdate::Runs(runs)).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        if update_tx.send(WorkerUpdate::Error(e.to_string())).is_err() {
                            break;
                        }
                    }
                };
                list_refresh = Instant::now();
            }
            if refresh.elapsed() >= Duration::from_millis(800) {
                if let Some(id) = &selected {
                    let update = match backend.snapshot(id) {
                        Ok(snapshot) => WorkerUpdate::Snapshot(Box::new(snapshot)),
                        Err(error) => WorkerUpdate::Error(error.to_string()),
                    };
                    if update_tx.send(update).is_err() {
                        break;
                    }
                }
                refresh = Instant::now();
            }
        }
    });
    let mut app = App::default();
    while !stop.load(Ordering::Relaxed) {
        while let Ok(update) = update_rx.try_recv() {
            match update {
                WorkerUpdate::Runs(runs) => {
                    app.runs = runs;
                    if app.snapshot.run_id.is_empty() && app.notice.starts_with("Connecting") {
                        app.notice =
                            "New conversation. i writes; Ctrl+S starts the interview. Ctrl+P > Select run reattaches.".into();
                    }
                }
                WorkerUpdate::Conversation(text, result) => {
                    app.sending = false;
                    match result {
                        Ok(started) => {
                            app.snapshot.run_id = started.run_id;
                            app.snapshot.intake = true;
                            if started.message_accepted && app.editor.text == text {
                                app.editor.set(String::new());
                            }
                            app.notice = sanitize::sanitize(&started.notice);
                        }
                        Err(error) => {
                            app.notice = sanitize::sanitize(&format!(
                                "Conversation failed: {error}. Draft retained."
                            ))
                        }
                    }
                }
                WorkerUpdate::Created(result) => {
                    app.sending = false;
                    match result {
                        Ok(id) => {
                            app.view = app::View::Main;
                            app.editing = false;
                            app.notice = format!(
                                "Created {id}. Draft retained; i resumes writing. Ctrl+P opens stage approvals."
                            );
                        }
                        Err(error) => {
                            app.notice = sanitize::sanitize(&format!("Creation failed: {error}"))
                        }
                    }
                }
                WorkerUpdate::Snapshot(snapshot) => {
                    app.update(*snapshot);
                    if app.notice.starts_with("Connecting")
                        || app.notice.starts_with("No run attached")
                    {
                        app.notice = "Attached. Run lifetime is managed by the daemon.".into();
                    }
                }
                WorkerUpdate::Action(action, result) => app.action_result(&action, result),
                WorkerUpdate::Error(error) => {
                    app.notice = sanitize::sanitize(&format!("Refresh failed: {error}"))
                }
            }
        }
        terminal.draw(|frame| render::draw(frame, &mut app))?;
        if event::poll(Duration::from_millis(100))? {
            for effect in app.handle(event::read()?) {
                match effect {
                    Effect::Detach => return Ok(()),
                    Effect::BeginConversation(text) => {
                        request_tx
                            .send(WorkerRequest::BeginConversation(text))
                            .map_err(|_| {
                                crate::Error::Invalid("UI backend worker disconnected".into())
                            })?;
                    }
                    Effect::CreateRun(path) => {
                        request_tx.send(WorkerRequest::Create(path)).map_err(|_| {
                            crate::Error::Invalid("UI backend worker disconnected".into())
                        })?;
                    }
                    Effect::SelectRun(id) => {
                        request_tx.send(WorkerRequest::Select(id)).map_err(|_| {
                            crate::Error::Invalid("UI backend worker disconnected".into())
                        })?;
                    }
                    Effect::Action(action) => {
                        request_tx
                            .send(WorkerRequest::Action(app.snapshot.run_id.clone(), action))
                            .map_err(|_| {
                                crate::Error::Invalid("UI backend worker disconnected".into())
                            })?;
                    }
                }
            }
        }
    }
    Ok(())
}
