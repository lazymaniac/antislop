use super::*;

impl Session {
    pub(super) fn start(
        core: &Core,
        conversation: String,
        spec: agent::Spec,
        signature: String,
        role: &str,
    ) -> Result<Self> {
        Self::build(core, conversation, spec, signature, role)
    }

    fn build(
        core: &Core,
        conversation: String,
        spec: agent::Spec,
        signature: String,
        role: &str,
    ) -> Result<Self> {
        let gateway = mcp::Gateway::start()?;
        gateway.set_role(role)?;
        if let Some((id, _)) = super::super::task_role(role)
            && let Some(task) = core.store.task_by_id(id)?
        {
            gateway.set_task_contract(serde_json::from_value(task.spec)?)?;
        }
        let launch = launch::command(&spec, &gateway)?;
        let weak = core.weak.get().expect("core weak").clone();
        let terminal =
            terminal::Session::spawn(launch.command, &weak, agent::name(spec.provider), 120, 40)?;
        let screen = terminal.shared();
        Ok(Self {
            conversation,
            core: weak,
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
            approver: Mutex::new(None),
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
            return Err(StopCause::Cancelled.diagnostic("the agent session was stopped"));
        }
        let screen = terminal::tail(&self.screen, SCREEN_LINES);
        if let Some(cause) = stops::screen_stop(self.spec.provider, &screen)
            && !(cause == StopCause::UnexpectedDialog && self.approval().is_some())
        {
            return Err(cause.diagnostic(screen));
        }
        let mut terminal = self.terminal.lock().map_err(invalid)?;
        if let Some(status) = terminal.exited() {
            return Err(StopCause::ProcessExit.diagnostic(format!(
                "{} exited ({status}): {screen}",
                agent::name(self.spec.provider)
            )));
        }
        Ok(())
    }

    /// The approval dialog the CLI shows now, if it is one this runtime
    /// recognizes.
    fn approval(&self) -> Option<approvals::Approval> {
        approvals::detect(
            self.spec.provider,
            &terminal::tail(&self.screen, APPROVAL_LINES),
        )
    }

    /// Settles the approval dialog on screen, if any: asks whoever decides and
    /// presses the chosen answer. Returns once the dialog is gone.
    fn serve_approval(&self, cancel: &AtomicBool) -> Result<()> {
        let Some(approval) = self.approval() else {
            return Ok(());
        };
        let approver = self
            .approver
            .lock()
            .map_err(invalid)?
            .clone()
            .ok_or_else(|| {
                StopCause::UnexpectedDialog.diagnostic(format!(
                    "no one is set to answer this approval dialog:\n{}",
                    terminal::tail(&self.screen, SCREEN_LINES)
                ))
            })?;
        let shown = || {
            self.approval()
                .is_some_and(|now| now.fingerprint == approval.fingerprint)
        };
        match approver(&approval, cancel, &shown)? {
            approvals::Verdict::AnsweredInTerminal => Ok(()),
            approvals::Verdict::Choose(id) => {
                let choice = approval
                    .choice(&id)
                    .ok_or_else(|| invalid(format!("approval dialog has no choice {id}")))?;
                // The keys must make the dialog disappear; if it is still
                // there they are pressed once more, then the failure is
                // reported with the screen as it is.
                for attempt in 0..2 {
                    for key in &choice.keys {
                        if cancel.load(Ordering::SeqCst) {
                            return Err(invalid(mcp::CANCELLED));
                        }
                        if !terminal::Session::press_for_approval(
                            &self.screen,
                            self.spec.provider,
                            &approval.fingerprint,
                            *key,
                        )? {
                            // The old decision is stale. The caller's polling
                            // loop will recognize and route the replacement.
                            return Ok(());
                        }
                        std::thread::sleep(Duration::from_millis(80));
                    }
                    if self.dialog_gone(&approval, cancel)? {
                        return Ok(());
                    }
                    if attempt == 1 {
                        return Err(StopCause::UnexpectedDialog.diagnostic(format!(
                            "the approval dialog did not accept the answer:\n{}",
                            terminal::tail(&self.screen, SCREEN_LINES)
                        )));
                    }
                }
                Ok(())
            }
        }
    }

    /// Waits a few seconds for `approval` to leave the screen.
    fn dialog_gone(&self, approval: &approvals::Approval, cancel: &AtomicBool) -> Result<bool> {
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            let shown = self
                .approval()
                .is_some_and(|now| now.fingerprint == approval.fingerprint);
            if !shown {
                return Ok(true);
            }
            if cancel.load(Ordering::SeqCst) {
                return Err(invalid(mcp::CANCELLED));
            }
            if Instant::now() >= until {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Require MCP tool discovery, then briefly let startup drawing settle.
    /// Output silence is a courtesy, not a readiness signal: interactive CLIs
    /// may redraw a spinner continuously, including between completed turns.
    fn wait_ready(&self, cancel: &AtomicBool) -> Result<()> {
        self.wait_for_connection(cancel)?;
        self.wait_for_settle(cancel)
    }

    fn wait_for_connection(&self, cancel: &AtomicBool) -> Result<()> {
        while !self.gateway.connected() {
            if cancel.load(Ordering::SeqCst) {
                return Err(invalid(mcp::CANCELLED));
            }
            self.check_alive()?;
            self.serve_approval(cancel)?;
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
            self.serve_approval(cancel)?;
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
        // A failed/cancelled turn must never spawn provider history queries.
        if result.is_err()
            && self.identity().is_none()
            && let Some(id) = self.gateway.session()
        {
            *self.identity.lock().map_err(invalid)? = Some(id);
        }
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

    pub(crate) fn set_approver(&self, approver: Arc<approvals::Approver>) -> Result<()> {
        *self.approver.lock().map_err(invalid)? = Some(approver);
        Ok(())
    }

    pub(crate) fn set_protocol_observer(&self, observer: Arc<mcp::ProtocolObserver>) -> Result<()> {
        self.gateway.set_protocol_observer(observer)
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
        let readiness = Mutex::new(readiness::Tracker::default());
        self.gateway
            .wait(turn, cancel, || {
                self.check_alive()?;
                self.serve_approval(cancel)
                    .and_then(|()| self.remind_if_ready(turn, cancel, &readiness))
            })
            .inspect_err(|error| {
                if error.to_string() == mcp::CANCELLED {
                    let _ = terminal::Session::interrupt(&self.screen);
                }
            })
            .map_err(|error| self.describe_result_error(error))
    }

    fn remind_if_ready(
        &self,
        turn: &str,
        cancel: &AtomicBool,
        readiness: &Mutex<readiness::Tracker>,
    ) -> Result<()> {
        if cancel.load(Ordering::SeqCst) || self.approval().is_some() {
            return Ok(());
        }
        let core = self
            .core
            .upgrade()
            .ok_or_else(|| invalid("agent session lost its core"))?;
        let paused = core.store.paused(&self.conversation)?;
        let screen = terminal::tail(&self.screen, SCREEN_LINES);
        let ready =
            readiness
                .lock()
                .map_err(invalid)?
                .observe(self.spec.provider, &screen, false, paused);
        if ready && !cancel.load(Ordering::SeqCst) {
            self.gateway.while_pending(turn, || {
                if cancel.load(Ordering::SeqCst) {
                    return Ok(());
                }
                terminal::Session::submit(
                    &self.screen,
                    "Your response appears complete, but this MCP turn is still open. Continue any remaining work, then call the required result tool now. Do not finish in terminal prose.",
                )
            })?;
        }
        Ok(())
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
        use std::io::{Read, Seek};
        let directory = self.spec.cwd.join(".antislop/sessions");
        std::fs::create_dir_all(&directory).ok()?;
        let mut capture = tempfile::tempfile_in(directory).ok()?;
        let mut child = std::process::Command::new(&self.spec.binary)
            .args(&self.spec.args)
            .args(["session", "list", "--format", "json", "-n", "40"])
            .current_dir(&self.spec.cwd)
            .stdout(capture.try_clone().ok()?)
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => break,
                Ok(Some(_)) => return None,
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                Ok(None) => {}
            }
            if Instant::now() >= deadline || self.stop.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        capture.rewind().ok()?;
        let mut output = Vec::new();
        capture.take(256 * 1024).read_to_end(&mut output).ok()?;
        let sessions: serde_json::Value = serde_json::from_slice(&output).ok()?;
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
