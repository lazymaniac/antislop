use super::*;

pub(super) struct WorkerStart {
    pub client: Client,
    pub conversation: String,
    pub role: String,
    pub dimensions: (u16, u16),
    pub updates: mpsc::Sender<Update>,
    pub output: Arc<std::sync::Mutex<Snapshot>>,
    pub stopped: Arc<AtomicBool>,
    pub received: mpsc::Receiver<Command>,
}

pub(super) struct Worker {
    client: Client,
    conversation: String,
    role: String,
    dimensions: (u16, u16),
    updates: mpsc::Sender<Update>,
    output: Arc<std::sync::Mutex<Snapshot>>,
    stopped: Arc<AtomicBool>,
    received: mpsc::Receiver<Command>,
    screen: Option<Screen>,
    sequence: u64,
    retry: bool,
    blocked: Option<String>,
}

impl Worker {
    pub(super) fn new(start: WorkerStart) -> Self {
        Self {
            client: start.client,
            conversation: start.conversation,
            role: start.role,
            dimensions: start.dimensions,
            updates: start.updates,
            output: start.output,
            stopped: start.stopped,
            received: start.received,
            screen: None,
            sequence: 0,
            retry: false,
            blocked: None,
        }
    }

    pub(super) fn run(mut self) {
        loop {
            self.open_if_needed();
            let closing = self.stopped.load(Ordering::SeqCst);
            match self.receive(closing) {
                Ok(Command::Retry) => {
                    self.retry_session();
                    continue;
                }
                Ok(command) => self.apply_command(command),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            self.read_screen();
            self.publish_screen();
        }
        self.detach();
    }

    fn receive(&self, closing: bool) -> std::result::Result<Command, mpsc::RecvTimeoutError> {
        if closing {
            self.received.try_recv().map_err(|error| match error {
                mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected => {
                    mpsc::RecvTimeoutError::Disconnected
                }
            })
        } else {
            self.received.recv_timeout(Duration::from_millis(80))
        }
    }

    fn open_if_needed(&mut self) {
        if self.screen.is_some() || !self.should_open() {
            return;
        }
        let request = Request::TerminalOpen {
            conversation: self.conversation.clone(),
            role: self.role.clone(),
            columns: self.dimensions.0,
            rows: self.dimensions.1,
            retry: self.retry,
        };
        let opened = self
            .client
            .call(&request)
            .and_then(|value| serde_json::from_value(value).map_err(Into::into));
        match opened {
            Ok(screen) => self.screen = Some(screen),
            Err(error) => self.set_error(error.to_string()),
        }
    }

    fn should_open(&self) -> bool {
        self.output
            .lock()
            .expect("native view")
            .error
            .as_deref()
            .is_none_or(|error| {
                error.contains("native session has not started")
                    || error.contains("has not reported this session's identifier")
            })
    }

    fn retry_session(&mut self) {
        if let Some(old) = self.screen.take() {
            let _ = self.client.call(&Request::TerminalDetach {
                conversation: self.conversation.clone(),
                role: self.role.clone(),
                handle: old.handle,
            });
        }
        *self.output.lock().expect("native view") = Snapshot::default();
        self.sequence = 0;
        self.blocked = None;
        self.retry = true;
    }

    fn apply_command(&mut self, command: Command) {
        if let Command::Resize(columns, rows) = command {
            self.dimensions = (columns, rows);
            self.send_command(Command::Resize(columns, rows));
            return;
        }
        if self.reject_blocked(&command) {
            return;
        }
        match self.screen.as_ref() {
            Some(screen) => self.send_command_to_screen(command, screen.clone()),
            None => self.reject_not_ready(command),
        }
    }

    fn reject_blocked(&self, command: &Command) -> bool {
        let (Command::Input(input), Some(error)) = (command, &self.blocked) else {
            return false;
        };
        self.reject_input(input.clone(), error.clone());
        true
    }

    fn reject_not_ready(&self, command: Command) {
        if let Command::Input(input) = command {
            self.reject_input(input, "native terminal is not ready".into());
        }
    }

    fn send_command(&mut self, command: Command) {
        let Some(screen) = self.screen.as_ref() else {
            return;
        };
        self.send_command_to_screen(command, screen.clone());
    }

    fn send_command_to_screen(&mut self, command: Command, screen: Screen) {
        let rejected_input = match &command {
            Command::Input(input) => Some(input.clone()),
            _ => None,
        };
        let request = self.request(command, &screen);
        let is_input = rejected_input.is_some();
        let result = if is_input {
            super::call_input(&self.client, &request)
        } else {
            self.client.call(&request)
        };
        self.record_result(result, rejected_input);
    }

    fn request(&self, command: Command, screen: &Screen) -> Request {
        match command {
            Command::Input(input) => Request::TerminalInput {
                conversation: self.conversation.clone(),
                role: self.role.clone(),
                handle: screen.handle.clone(),
                sequence: self.sequence + 1,
                input,
            },
            Command::Resize(columns, rows) => Request::TerminalResize {
                conversation: self.conversation.clone(),
                role: self.role.clone(),
                handle: screen.handle.clone(),
                columns,
                rows,
            },
            Command::Retry => unreachable!(),
        }
    }

    fn record_result(&mut self, result: Result<Value>, rejected_input: Option<Input>) {
        let is_input = rejected_input.is_some();
        match result {
            Ok(_) => {
                if is_input {
                    self.sequence += 1;
                }
                self.set_error(String::new());
            }
            Err(error) => {
                let message = error.to_string();
                if is_input {
                    self.blocked = Some(message.clone());
                }
                if let Some(input) = rejected_input {
                    self.reject_input(input, message.clone());
                }
                self.set_error(message);
            }
        }
    }

    fn read_screen(&mut self) {
        let Some(screen) = &self.screen else {
            return;
        };
        let request = Request::TerminalRead {
            conversation: self.conversation.clone(),
            role: self.role.clone(),
            handle: screen.handle.clone(),
            after: screen.revision,
        };
        match self.client.call(&request) {
            Ok(value) if !value.is_null() => match serde_json::from_value::<Screen>(value) {
                Ok(next) => self.screen = Some(next),
                Err(error) => self.set_error(error.to_string()),
            },
            Err(error) => self.set_error(error.to_string()),
            _ => {}
        }
    }

    fn publish_screen(&self) {
        self.output.lock().expect("native view").screen = self.screen.clone();
    }

    fn detach(&self) {
        if let Some(screen) = &self.screen {
            let _ = self.client.call(&Request::TerminalDetach {
                conversation: self.conversation.clone(),
                role: self.role.clone(),
                handle: screen.handle.clone(),
            });
        }
    }

    fn set_error(&self, error: String) {
        self.output.lock().expect("native view").error = (!error.is_empty()).then_some(error);
    }

    fn reject_input(&self, input: Input, error: String) {
        let _ = self.updates.send(Update::NativeInputRejected {
            conversation: self.conversation.clone(),
            role: self.role.clone(),
            input,
            error,
        });
    }
}
