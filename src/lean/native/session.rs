use super::*;

impl Session {
    pub(super) fn start(
        core: &Core,
        spec: agent::Spec,
        signature: String,
        role: &str,
    ) -> Result<Self> {
        Self::build(core, spec, signature, role)
    }

    fn build(core: &Core, spec: agent::Spec, signature: String, role: &str) -> Result<Self> {
        let gateway = mcp::Gateway::start()?;
        gateway.set_role(role)?;
        let launch = launch::command(&spec, &gateway)?;
        let weak = core.weak.get().expect("core weak").clone();
        let terminal =
            terminal::Session::spawn(launch.command, &weak, agent::name(spec.provider), 120, 40)?;
        let screen = terminal.shared();
        Ok(Self {
            spec,
            signature,
            _settings: launch.settings,
            gateway,
            terminal: Mutex::new(terminal),
            screen,
            identity: Mutex::new(launch.session),
            started_ms: store::now_ms(),
            stop: AtomicBool::new(false),
            executing: AtomicBool::new(false),
            primed: AtomicBool::new(false),
            startup_failed: AtomicBool::new(false),
            #[cfg(test)]
            ready_limit: Mutex::new(READY_LIMIT),
            touched: Mutex::new(Instant::now()),
        })
    }

    pub(crate) fn identity(&self) -> Option<String> {
        self.identity.lock().ok()?.clone()
    }

    pub(crate) fn touch(&self) {
        if let Ok(mut touched) = self.touched.lock() {
            *touched = Instant::now();
        }
    }

    /// Fails with the CLI's exit status and last screen lines once it ended.
    pub(crate) fn check_alive(&self) -> Result<()> {
        if self.stop.load(Ordering::SeqCst) {
            return Err(invalid("the agent session was stopped"));
        }
        let mut terminal = self.terminal.lock().map_err(invalid)?;
        if let Some(status) = terminal.exited() {
            return Err(invalid(format!(
                "{} exited ({status}): {}",
                agent::name(self.spec.provider),
                terminal::tail(&self.screen, SCREEN_LINES)
            )));
        }
        Ok(())
    }

    /// Alive, and printed something within `idle` unless it waits for an answer.
    pub(super) fn check_active(&self, idle: Duration) -> Result<()> {
        self.check_alive()?;
        let quiet = terminal::quiet_for(&self.screen);
        if quiet < idle || self.gateway.awaiting_answer() {
            return Ok(());
        }
        Err(invalid(format!(
            "{} printed nothing for {} s: {}",
            agent::name(self.spec.provider),
            quiet.as_secs(),
            terminal::tail(&self.screen, SCREEN_LINES)
        )))
    }

    /// Require MCP tool discovery, then briefly let startup drawing settle.
    /// Output silence is a courtesy, not a readiness signal: interactive CLIs
    /// may redraw a spinner continuously, including between completed turns.
    fn wait_ready(&self, cancel: &AtomicBool) -> Result<()> {
        #[cfg(test)]
        let ready_limit = self.ready_limit.lock().map_or(READY_LIMIT, |limit| *limit);
        #[cfg(not(test))]
        let ready_limit = READY_LIMIT;
        self.wait_for_connection(cancel, ready_limit)?;
        self.wait_for_settle(cancel)
    }

    fn wait_for_connection(&self, cancel: &AtomicBool, ready_limit: Duration) -> Result<()> {
        let limit = Instant::now() + ready_limit;
        while !self.gateway.connected() {
            if cancel.load(Ordering::SeqCst) {
                return Err(invalid(mcp::CANCELLED));
            }
            self.check_alive()?;
            if Instant::now() >= limit {
                return Err(invalid(format!(
                    "{} did not connect to the antislop MCP gateway within {} s; the terminal shows:\n{}",
                    agent::name(self.spec.provider),
                    READY_LIMIT.as_secs(),
                    terminal::tail(&self.screen, SCREEN_LINES)
                )));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }

    fn wait_for_settle(&self, cancel: &AtomicBool) -> Result<()> {
        let settle_until = Instant::now() + SETTLE_LIMIT;
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(invalid(mcp::CANCELLED));
            }
            self.check_alive()?;
            if terminal::quiet_for(&self.screen) >= SETTLE || Instant::now() >= settle_until {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Runs one turn: types the prompt into the CLI and waits for it to end
    /// the turn through the gateway.
    pub(crate) fn run(
        &self,
        registry: &Registry,
        turn: &str,
        prompt: &str,
        cancel: &AtomicBool,
    ) -> Result<agent::Outcome> {
        self.check_alive()?;
        require(
            self.executing
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok(),
            "the agent session already has an executing turn",
        )?;
        let result = self.execute(registry, turn, prompt, cancel);
        self.executing.store(false, Ordering::SeqCst);
        self.touch();
        // No failover after the CLI accepted a prompt: a failed session is
        // dropped so the next turn starts a clean one that resumes it.
        match result {
            Err(error) if error.to_string() != mcp::CANCELLED => {
                self.stop.store(true, Ordering::SeqCst);
                Err(invalid(format!("{EXECUTION_FAILED}{error}")))
            }
            other => other,
        }
    }

    /// Whether the last run failed before the CLI took its prompt.
    pub(crate) fn failed_before_start(&self) -> bool {
        self.startup_failed.load(Ordering::SeqCst)
    }

    pub(crate) fn set_tool_handler(&self, handler: Arc<mcp::Handler>) -> Result<()> {
        self.gateway.set_handler(handler)
    }

    fn execute(
        &self,
        registry: &Registry,
        turn: &str,
        prompt: &str,
        cancel: &AtomicBool,
    ) -> Result<agent::Outcome> {
        self.submit_turn(turn, prompt, cancel)?;
        let finish = self.wait_for_result(turn, cancel)?;
        self.learn_identity(registry);
        Ok(agent::Outcome {
            session: self.identity(),
            text: finish.text,
        })
    }

    fn submit_turn(&self, turn: &str, prompt: &str, cancel: &AtomicBool) -> Result<()> {
        self.wait_ready(cancel).inspect_err(|error| {
            if error.to_string() != mcp::CANCELLED {
                self.startup_failed.store(true, Ordering::SeqCst);
            }
        })?;
        self.gateway.begin(turn)?;
        terminal::Session::submit(&self.screen, &turn_text(turn, prompt))?;
        self.primed.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn wait_for_result(&self, turn: &str, cancel: &AtomicBool) -> Result<mcp::Finish> {
        self.gateway
            .wait(turn, cancel, turn_timeout(), || {
                self.check_active(turn_idle())
            })
            .inspect_err(|error| {
                if error.to_string() == mcp::CANCELLED {
                    let _ = terminal::Session::interrupt(&self.screen);
                }
            })
            .map_err(|error| self.describe_result_error(error))
    }

    fn describe_result_error(&self, error: Error) -> Error {
        if ["no accepted result", "result rejected"]
            .iter()
            .any(|cause| error.to_string().contains(cause))
        {
            invalid(format!(
                "{error}; the terminal shows:\n{}",
                terminal::tail(&self.screen, SCREEN_LINES)
            ))
        } else {
            error
        }
    }

    /// Records the CLI's session identifier once it has shown one. Nothing
    /// is held while the others' identifiers are read.
    fn learn_identity(&self, registry: &Registry) {
        if self.identity().is_some() {
            return;
        }
        let found = self.gateway.session().or_else(|| {
            (self.spec.provider == crate::config::ProviderKind::Opencode)
                .then(|| self.opencode_session(&registry.claimed()))
                .flatten()
        });
        if let Some(id) = found
            && let Ok(mut identity) = self.identity.lock()
        {
            identity.get_or_insert(id);
        }
    }

    /// OpenCode does not announce its session. The one created in this
    /// directory since the process started, and not owned by another live
    /// session, is ours; with several candidates none is guessed.
    fn opencode_session(&self, claimed: &[String]) -> Option<String> {
        let output = std::process::Command::new(&self.spec.binary)
            .args(&self.spec.args)
            .args(["session", "list", "--format", "json", "-n", "40"])
            .current_dir(&self.spec.cwd)
            .output()
            .ok()?;
        let sessions: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
        let cwd = self.spec.cwd.canonicalize().ok()?;
        let mut candidates = sessions.as_array()?.iter().filter(|session| {
            session["created"]
                .as_i64()
                .is_some_and(|at| at >= self.started_ms - 1_000)
                && session["directory"].as_str().is_some_and(|directory| {
                    Path::new(directory).canonicalize().ok().as_deref() == Some(&cwd)
                })
                && session["id"]
                    .as_str()
                    .is_some_and(|id| !claimed.iter().any(|known| known == id))
        });
        let first = candidates.next()?;
        candidates
            .next()
            .is_none()
            .then(|| first["id"].as_str().map(str::to_owned))
            .flatten()
    }
}
