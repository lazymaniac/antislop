use super::*;
mod worker;
use worker::{Worker, WorkerStart};

pub(super) fn call_input(client: &Client, request: &Request) -> Result<Value> {
    // TerminalInput is deduplicated by handle and sequence. Reconnect with
    // the same request if the daemon accepted it but its reply was lost.
    let mut result = client.call(request);
    for _ in 0..2 {
        let transient = match &result {
            Err(crate::Error::Io(_)) => true,
            Err(crate::Error::Invalid(message)) => {
                message == "the daemon disconnected"
                    || message.starts_with("terminal input not accepted:")
            }
            _ => false,
        };
        if !transient {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
        result = client.call(request);
    }
    result
}
struct Launch {
    client: Client,
    conversation: String,
    role: String,
    dimensions: (u16, u16),
    updates: mpsc::Sender<Update>,
    identity: String,
}

impl View {
    pub(super) fn start(
        client: Client,
        conversation: String,
        role: String,
        columns: u16,
        rows: u16,
        updates: mpsc::Sender<Update>,
        identity: String,
    ) -> Self {
        Self::launch(Launch {
            client,
            conversation,
            role,
            dimensions: (columns, rows),
            updates,
            identity,
        })
    }

    fn launch(launch: Launch) -> Self {
        let state = Arc::new(std::sync::Mutex::new(Snapshot::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (commands, received) = mpsc::sync_channel(64);
        let worker = Worker::new(WorkerStart {
            client: launch.client,
            conversation: launch.conversation,
            role: launch.role,
            dimensions: launch.dimensions,
            updates: launch.updates,
            output: state.clone(),
            stopped: stop.clone(),
            received,
        });
        std::thread::spawn(move || worker.run());
        Self {
            state,
            commands,
            stop,
            dimensions: launch.dimensions,
            identity: launch.identity,
        }
    }

    pub(super) fn send(&self, command: Command) -> Result<()> {
        self.commands.try_send(command).map_err(|_| {
            crate::Error::Invalid("native terminal input queue is full or disconnected".into())
        })
    }

    pub(super) fn resize(&mut self, columns: u16, rows: u16) {
        if self.dimensions != (columns, rows) && self.send(Command::Resize(columns, rows)).is_ok() {
            self.dimensions = (columns, rows);
        }
    }
}
impl Drop for View {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}
