//! Daemon-owned native sessions. The gateway is the only endpoint supplied to
//! the interactive CLI; executable turns always pass through the durable mail
//! queue and the core's existing role lock.
mod codex;
mod opencode;

use super::{Core, agent, daemon, store};
use crate::{Error, Result, config::ProviderKind, require};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
pub(crate) const EXECUTION_FAILED: &str =
    "native backend execution failed (automatic retry disabled): ";
const UNSUPPORTED_PROTOCOL: &str = "native backend protocol unsupported: ";

pub(super) fn invalid(error: impl std::fmt::Display) -> Error {
    Error::Invalid(error.to_string())
}

#[derive(Default)]
pub struct Registry {
    sessions: Mutex<HashMap<(String, String), Arc<Session>>>,
}

pub struct Session {
    pub(crate) core: Weak<Core>,
    pub(crate) conversation: String,
    pub(crate) role: String,
    pub(crate) spec: agent::Spec,
    signature: String,
    pub(crate) identity: String,
    pub(crate) endpoint: String,
    pub(crate) gateway_address: Mutex<Option<String>>,
    pub(crate) gateway_password: String,
    pub(crate) backend: Backend,
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) active: Mutex<Option<Active>>,
    pub(crate) completed: Mutex<std::collections::VecDeque<(u64, Value)>>,
    pub(crate) terminal: Mutex<Option<super::terminal::Session>>,
    pub(crate) gateway_error: Mutex<Option<String>>,
    pub(crate) started: AtomicBool,
    pub(crate) executing: AtomicBool,
    pub(crate) user_interrupted: AtomicBool,
    pub(crate) touched: Mutex<Instant>,
    pub(crate) scope_round: Option<u32>,
    process: Process,
}

pub(crate) struct Active {
    pub turn: Value,
    pub mail: Vec<u64>,
}

pub(crate) enum Backend {
    Codex {
        socket: PathBuf,
        rpc: Box<Mutex<codex::Rpc>>,
    },
    Opencode {
        endpoint: String,
        password: String,
    },
}

impl Registry {
    pub(crate) fn reap_idle(&self) -> Result<()> {
        let retired = {
            let mut sessions = self.sessions.lock().map_err(invalid)?;
            let keys = sessions
                .iter()
                .filter(|(_, session)| {
                    Arc::strong_count(session) == 1
                        && !session.executing.load(Ordering::SeqCst)
                        && session
                            .touched
                            .lock()
                            .is_ok_and(|at| at.elapsed() > Duration::from_secs(5))
                        && session
                            .terminal
                            .lock()
                            .is_ok_and(|terminal| terminal.is_none())
                })
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| sessions.remove(&key))
                .collect::<Vec<_>>()
        };
        drop(retired);
        Ok(())
    }
    pub(crate) fn restore(
        &self,
        core: &Core,
        conversation: &str,
        role: &str,
    ) -> Result<Arc<Session>> {
        self.restore_with(core, conversation, role, capable)
    }

    fn restore_with(
        &self,
        core: &Core,
        conversation: &str,
        role: &str,
        probe: impl FnOnce(&agent::Spec) -> bool,
    ) -> Result<Arc<Session>> {
        let turn = core
            .store
            .last_turn(conversation, role)?
            .ok_or_else(|| invalid("native session has not started"))?;
        let context = core
            .store
            .run_context(conversation, &turn.id)?
            .ok_or_else(|| invalid("native attachment unavailable for legacy JSON history"))?;
        require(
            context.backend != store::RunBackend::Cli,
            "native attachment unavailable: this role uses the CLI JSON backend",
        )?;
        require(
            context.cwd.is_dir(),
            "native working directory no longer exists; use JSON history or start new work",
        )?;
        let config = core.config_snapshot();
        let profile = config
            .profiles
            .get(&context.profile)
            .ok_or_else(|| invalid("native session profile no longer exists"))?;
        require(
            agent::name(profile.provider) == context.provider,
            "native session provider changed; start a new turn before attaching",
        )?;
        let provider = config
            .providers
            .get(&profile.provider)
            .ok_or_else(|| invalid("native provider is not configured"))?;
        let (binary, args) = crate::config::resolve_command(provider, profile)?;
        let spec = agent::Spec {
            provider: profile.provider,
            binary,
            args,
            model: profile.model.clone(),
            effort: profile
                .provider
                .native_effort(profile.effort)
                .map(str::to_owned),
            cwd: context.cwd,
            add_dirs: context.add_dirs,
            access: match context.access {
                store::RunAccess::ReadOnly => agent::Access::ReadOnly,
                store::RunAccess::Write => agent::Access::Write,
            },
            resume: context.provider_thread_id.or(context.provider_session_id),
        };
        require(
            spec.resume.is_some(),
            "waiting for the native provider's session identifier",
        )?;
        let execution_signature =
            super::rounds::spec_signature(&context.profile, profile.effort, &spec)?;
        if super::task_role(role).is_some() {
            let recorded: Option<String> = core
                .store
                .db()
                .query_row(
                    "SELECT json_extract(payload,'$.execution_signature') FROM events
                 WHERE turn_id=?1 AND kind='turn_started' ORDER BY seq DESC LIMIT 1",
                    [&turn.id],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            require(
                recorded.as_deref() == Some(execution_signature.as_str()),
                "native execution profile changed; start a fresh task turn before attaching",
            )?;
        }
        let scope = role_round(core, conversation, role)?
            .map(|(task, round)| {
                format!(
                    "{:?}:{}:{}",
                    super::SessionPolicy::TaskRound { task, round },
                    context.profile,
                    execution_signature
                )
            })
            .unwrap_or_default();
        self.prepare_scoped_with(core, conversation, role, &spec, &scope, probe)?
            .ok_or_else(|| {
                invalid("native attachment is unsupported by the current CLI or launcher")
            })
    }
    pub(crate) fn shutdown(&self) -> Result<()> {
        let mut sessions = self.sessions.lock().map_err(invalid)?;
        for session in sessions.values() {
            session.stop.store(true, Ordering::SeqCst);
            session.terminal.lock().map_err(invalid)?.take();
            session.process.terminate();
        }
        sessions.clear();
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn prepare(
        &self,
        core: &Core,
        conversation: &str,
        role: &str,
        spec: &agent::Spec,
    ) -> Result<Option<Arc<Session>>> {
        self.prepare_scoped(core, conversation, role, spec, "")
    }

    pub(crate) fn prepare_scoped(
        &self,
        core: &Core,
        conversation: &str,
        role: &str,
        spec: &agent::Spec,
        scope: &str,
    ) -> Result<Option<Arc<Session>>> {
        self.prepare_scoped_with(core, conversation, role, spec, scope, capable)
    }

    fn prepare_scoped_with(
        &self,
        core: &Core,
        conversation: &str,
        role: &str,
        spec: &agent::Spec,
        scope: &str,
        probe: impl FnOnce(&agent::Spec) -> bool,
    ) -> Result<Option<Arc<Session>>> {
        // Resume changes after the first turn, but does not replace the backend.
        let mut stable = spec.clone();
        stable.resume = None;
        let signature = if scope.is_empty() {
            format!("{role}:{stable:?}")
        } else {
            format!("{role}:{scope}:{stable:?}")
        };
        let key = (conversation.to_owned(), role.to_owned());
        let mut sessions = self.sessions.lock().map_err(invalid)?;
        if let Some(session) = sessions
            .get(&key)
            .filter(|s| s.signature == signature && !s.stop.load(Ordering::SeqCst))
        {
            session.check_alive()?;
            return Ok(Some(session.clone()));
        }
        if let Some(old) = sessions.remove(&key) {
            old.stop.store(true, Ordering::SeqCst);
            old.terminal.lock().map_err(invalid)?.take();
            old.process.terminate();
        }
        // Launchers may configure authentication or replace CLI semantics.
        // Probe only unwrapped provider executables, before any prompt is sent.
        if !spec.args.is_empty()
            || !matches!(spec.provider, ProviderKind::Codex | ProviderKind::Opencode)
        {
            return Ok(None);
        }
        if !probe(spec) {
            return Ok(None);
        }
        let session = match Session::start(core, conversation, role, spec.clone(), signature) {
            Ok(session) => Arc::new(session),
            // An older CLI may expose the server flag but lack the RPC API.
            // This decision is made before submitting any executable prompt.
            Err(Error::Invalid(message)) if message.starts_with(UNSUPPORTED_PROTOCOL) => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        session.start_gateway()?;
        sessions.insert(key, session.clone());
        Ok(Some(session))
    }

    pub(crate) fn get(&self, conversation: &str, role: &str) -> Result<Option<Arc<Session>>> {
        Ok(self
            .sessions
            .lock()
            .map_err(invalid)?
            .get(&(conversation.into(), role.into()))
            .cloned())
    }
}

fn capable(spec: &agent::Spec) -> bool {
    // Shell/Python wrappers are custom launchers, even without prefix args.
    // Do not execute them speculatively: some launchers treat --help as work.
    if let Ok(mut file) = std::fs::File::open(&spec.binary) {
        let mut prefix = [0; 128];
        if let Ok(count) = file.read(&mut prefix) {
            let prefix = String::from_utf8_lossy(&prefix[..count]);
            if prefix.starts_with("#!") && !prefix.lines().next().unwrap_or("").contains("node") {
                return false;
            }
        }
    }
    let expected = agent::name(spec.provider);
    if spec.binary.file_name().and_then(|name| name.to_str()) != Some(expected) {
        return false;
    }
    let commands: &[(&[&str], &str)] = match spec.provider {
        ProviderKind::Codex => &[
            (&["resume", "--help"], "--remote"),
            (&["app-server", "--help"], "--listen"),
        ],
        ProviderKind::Opencode => &[
            (&["attach", "--help"], "--session"),
            (&["serve", "--help"], "--hostname"),
        ],
        _ => return false,
    };
    commands.iter().all(|(args, marker)| {
        let mut command = Command::new(&spec.binary);
        command.args(*args).current_dir(&spec.cwd);
        let Ok(process) = Process::spawn(command, None) else {
            return false;
        };
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(Some(status)) = process.child.lock().expect("probe child").try_wait() {
                while process.readers.load(Ordering::SeqCst) != 0 && Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(2));
                }
                return status.success() && process.diagnostics().contains(marker);
            }
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    })
}

impl Session {
    fn start(
        core: &Core,
        conversation: &str,
        role: &str,
        spec: agent::Spec,
        signature: String,
    ) -> Result<Self> {
        let directory = core.state.join("native");
        daemon::private_directory(&directory)?;
        let id = crate::id();
        let stop = Arc::new(AtomicBool::new(false));
        let (process, backend, identity, endpoint) = match spec.provider {
            ProviderKind::Codex => codex::start(core, &directory, &id, &spec)?,
            ProviderKind::Opencode => opencode::start(core, &spec)?,
            _ => return Err(invalid("provider has no native adapter")),
        };
        let started = spec.resume.is_some();
        let scope_round = role_round(core, conversation, role)?.map(|(_, round)| round);
        Ok(Self {
            core: core.weak.get().expect("core weak").clone(),
            conversation: conversation.into(),
            role: role.into(),
            spec,
            signature,
            identity,
            endpoint,
            backend,
            stop,
            gateway_address: Mutex::new(None),
            gateway_password: crate::id(),
            active: Mutex::new(None),
            completed: Mutex::new(std::collections::VecDeque::new()),
            terminal: Mutex::new(None),
            gateway_error: Mutex::new(None),
            started: AtomicBool::new(started),
            executing: AtomicBool::new(false),
            user_interrupted: AtomicBool::new(false),
            touched: Mutex::new(Instant::now()),
            scope_round,
            process,
        })
    }

    fn start_gateway(self: &Arc<Self>) -> Result<()> {
        match self.backend {
            Backend::Codex { .. } => codex::gateway(self),
            Backend::Opencode { .. } => opencode::gateway(self),
        }
    }

    pub(crate) fn backend_kind(&self) -> store::RunBackend {
        match self.backend {
            Backend::Codex { .. } => store::RunBackend::CodexAppServer,
            Backend::Opencode { .. } => store::RunBackend::OpencodeServer,
        }
    }
    pub(crate) fn attachment_failed(&self, error: &crate::Error) {
        *self.gateway_error.lock().expect("gateway error") = Some(error.to_string());
    }

    pub(crate) fn check_alive(&self) -> Result<()> {
        require(
            !self.stop.load(Ordering::SeqCst),
            "native backend stopped; retry requires a new session",
        )?;
        if let Some(status) = self.process.child.lock().map_err(invalid)?.try_wait()? {
            self.stop.store(true, Ordering::SeqCst);
            return Err(invalid(format!(
                "native backend exited ({status}): {}",
                self.process.diagnostics()
            )));
        }
        Ok(())
    }

    pub(crate) fn run(
        &self,
        turn: &str,
        prompt: &str,
        mail: &[store::Mail],
        cancel: &AtomicBool,
        on_event: &mut dyn FnMut(agent::Event),
    ) -> Result<agent::Outcome> {
        self.check_alive()?;
        require(
            self.executing
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok(),
            "native session already has an executing turn",
        )?;
        self.user_interrupted.store(false, Ordering::SeqCst);
        let result = match &self.backend {
            Backend::Codex { rpc, .. } => codex::run(
                self,
                &mut *rpc.lock().map_err(invalid)?,
                turn,
                prompt,
                mail,
                cancel,
                on_event,
            ),
            Backend::Opencode { .. } => opencode::run(self, turn, prompt, cancel, on_event),
        };
        self.executing.store(false, Ordering::SeqCst);
        if let Some(mut active) = self.active.lock().map_err(invalid)?.take() {
            active.turn["status"] = if result.is_ok() {
                "completed".into()
            } else {
                "failed".into()
            };
            let mut completed = self.completed.lock().map_err(invalid)?;
            for mail in active.mail {
                completed.push_back((mail, active.turn.clone()));
            }
            while completed.len() > 128 {
                completed.pop_front();
            }
        }
        // No failover after an executing backend accepted a prompt.
        if result
            .as_ref()
            .is_err_and(|error| !error.to_string().contains("native turn interrupted"))
        {
            self.stop.store(true, Ordering::SeqCst);
            self.process.terminate();
            if let Err(error) = result {
                return Err(invalid(format!("{EXECUTION_FAILED}{error}")));
            }
        }
        // Unopened terminals have no draft to retain. Their provider history
        // is durable, so idle backends can be restored on demand rather than
        // accumulating a server for every completed graph role.
        *self.touched.lock().map_err(invalid)? = Instant::now();
        result
    }

    pub(crate) fn enqueue(&self, operation: &str, text: &str) -> Result<Value> {
        self.check_alive()?;
        let core = self
            .core
            .upgrade()
            .ok_or_else(|| invalid("daemon stopped"))?;
        let latest = core
            .store
            .last_turn(&self.conversation, &self.role)?
            .ok_or_else(|| invalid("managed role has no turn"))?;
        let task = super::task_role(&self.role)
            .map(|(id, _)| core.store.task_by_id(id))
            .transpose()?
            .flatten();
        require(
            latest
                .session_id
                .as_deref()
                .is_none_or(|id| id == self.identity),
            "managed session changed; reopen the native terminal",
        )?;
        require(
            task.as_ref()
                .is_none_or(|task| Some(task.round) == self.scope_round),
            "task round changed; reopen the native terminal",
        )?;
        use sha2::Digest;
        let digest = sha2::Sha256::digest(
            format!(
                "{}:{}:{}:{operation}",
                self.conversation, self.role, self.identity
            )
            .as_bytes(),
        );
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&digest[..16]);
        // UUIDv8 carries a deterministic, session-scoped operation key.
        bytes[6] = (bytes[6] & 0x0f) | 0x80;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let message = store::AgentMessage {
            conversation: self.conversation.clone(),
            plan: latest.plan_id.clone(),
            recipient: self.role.clone(),
            operation_id: uuid::Uuid::from_bytes(bytes).to_string(),
            expected_round: task.map(|task| task.round),
            expected_run: Some(latest.id),
            text: text.into(),
        };
        core.send_to_agent(&message)
    }

    pub(crate) fn interrupt(&self) -> Result<()> {
        self.check_alive()?;
        require(
            self.executing.load(Ordering::SeqCst),
            "no active native turn to interrupt",
        )?;
        let core = self
            .core
            .upgrade()
            .ok_or_else(|| invalid("daemon stopped"))?;
        let turn = core
            .store
            .last_turn(&self.conversation, &self.role)?
            .ok_or_else(|| invalid("no active turn"))?;
        core.store.append_event(
            &self.conversation,
            Some(&turn.id),
            turn.plan_id.as_deref(),
            "native_control",
            &json!({"operation":"interrupt","role":self.role}),
        )?;
        self.user_interrupted.store(true, Ordering::SeqCst);
        match &self.backend {
            Backend::Codex { socket, .. } => {
                let active = self.active.lock().map_err(invalid)?;
                let active = active
                    .as_ref()
                    .ok_or_else(|| invalid("no active native turn"))?;
                let mut rpc = codex::Rpc::connect(socket)?;
                rpc.call(
                    "thread/resume",
                    codex::thread_params(&self.spec, Some(&self.identity)),
                )?;
                rpc.call(
                    "turn/interrupt",
                    json!({"threadId":self.identity,"turnId":active.turn["id"]}),
                )?;
            }
            Backend::Opencode { .. } => {
                opencode::request(
                    self,
                    "POST",
                    &format!("/session/{}/abort", self.identity),
                    Some(json!({})),
                )?;
            }
        }
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Backend::Codex { socket, .. } = &self.backend {
            let _ = std::fs::remove_file(socket);
            if let Some(path) = self.endpoint.strip_prefix("unix://") {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

pub(crate) struct Process {
    pub child: Mutex<Child>,
    diagnostic: Arc<Mutex<Vec<u8>>>,
    registered: Mutex<Option<(Weak<Core>, u32)>>,
    readers: Arc<std::sync::atomic::AtomicUsize>,
}

fn role_round(core: &Core, conversation: &str, role: &str) -> Result<Option<(String, u32)>> {
    let row = core
        .store
        .db()
        .query_row(
            "SELECT s.task_id,s.round FROM turns t
        LEFT JOIN turn_scopes s ON s.turn_id=t.id WHERE t.conversation_id=?1 AND t.role=?2
        ORDER BY t.started_ms DESC,t.rowid DESC LIMIT 1",
            rusqlite::params![conversation, role],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<u32>>(1)?,
                ))
            },
        )
        .optional()?;
    Ok(row.and_then(|(task, round)| task.zip(round)))
}

impl Process {
    fn terminate(&self) {
        if let Ok(mut child) = self.child.lock() {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            // The process is the leader of the group created at spawn.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.wait();
        }
    }
    fn spawn(
        mut command: Command,
        ready: Option<std::sync::mpsc::SyncSender<String>>,
    ) -> Result<Self> {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .map_err(invalid)?;
        let diagnostic = Arc::new(Mutex::new(Vec::new()));
        let readers = Arc::new(std::sync::atomic::AtomicUsize::new(2));
        for stream in [
            Box::new(child.stdout.take().expect("stdout")) as Box<dyn Read + Send>,
            Box::new(child.stderr.take().expect("stderr")),
        ] {
            let diagnostic = diagnostic.clone();
            let ready = ready.clone();
            let readers = readers.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream);
                let mut buffer = [0; 4096];
                while let Ok(count) = reader.read(&mut buffer) {
                    if count == 0 {
                        break;
                    }
                    let mut kept = diagnostic.lock().expect("diagnostic");
                    kept.extend_from_slice(&buffer[..count]);
                    if kept.len() > 16384 {
                        let remove = kept.len() - 16384;
                        kept.drain(..remove);
                    }
                    if let Some(ready) = &ready {
                        let text = String::from_utf8_lossy(&kept);
                        if let Some(start) = text.find("http://127.0.0.1:") {
                            let endpoint: String = text[start..]
                                .chars()
                                .take_while(|ch| ch.is_ascii_alphanumeric() || ":/.".contains(*ch))
                                .collect();
                            let complete = start + endpoint.len() < text.len();
                            if complete
                                && endpoint
                                    .trim_end_matches('/')
                                    .strip_prefix("http://")
                                    .is_some_and(|address| {
                                        address.parse::<std::net::SocketAddr>().is_ok()
                                    })
                            {
                                let _ = ready.try_send(endpoint);
                            }
                        }
                    }
                }
                readers.fetch_sub(1, Ordering::SeqCst);
            });
        }
        Ok(Self {
            child: Mutex::new(child),
            diagnostic,
            registered: Mutex::new(None),
            readers,
        })
    }
    fn diagnostics(&self) -> String {
        String::from_utf8_lossy(&self.diagnostic.lock().expect("diagnostic")).into_owned()
    }
    fn register(&self, core: &Core, provider: &str) -> Result<()> {
        let pid = self.child.lock().map_err(invalid)?.id();
        core.store.db().execute(
            "INSERT OR REPLACE INTO native_processes(pid,started_ms,provider) VALUES(?1,?2,?3)",
            rusqlite::params![
                self.child.lock().map_err(invalid)?.id(),
                store::now_ms(),
                provider
            ],
        )?;
        *self.registered.lock().map_err(invalid)? =
            Some((core.weak.get().expect("core weak").clone(), pid));
        Ok(())
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Ok(child) = self.child.get_mut() {
            if child.try_wait().ok().flatten().is_none() {
                // Give an idle provider time to flush its session recorder.
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGINT);
                }
                let until = Instant::now() + Duration::from_secs(1);
                while Instant::now() < until && child.try_wait().ok().flatten().is_none() {
                    std::thread::sleep(Duration::from_millis(10));
                }
                // SAFETY: the child was spawned as the leader of its own group.
                if child.try_wait().ok().flatten().is_none() {
                    unsafe {
                        libc::kill(-(child.id() as i32), libc::SIGKILL);
                    }
                }
            }
            let _ = child.wait();
        }
        if let Ok(Some((core, pid))) = self.registered.get_mut()
            && let Some(core) = core.upgrade()
        {
            let _ = core
                .store
                .db()
                .execute("DELETE FROM native_processes WHERE pid=?1", [*pid]);
        }
    }
}

pub(crate) fn recover(store: &store::Store, state: &Path) -> Result<()> {
    store.db().execute_batch(
        "CREATE TABLE IF NOT EXISTS native_processes(
        pid INTEGER PRIMARY KEY,started_ms INTEGER NOT NULL,provider TEXT NOT NULL)",
    )?;
    let rows = {
        let db = store.db();
        let mut query = db.prepare("SELECT pid,started_ms,provider FROM native_processes")?;
        query
            .query_map([], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    for (pid, started, provider) in rows {
        agent::reap(pid, started, &provider);
        store
            .db()
            .execute("DELETE FROM native_processes WHERE pid=?1", [pid])?;
    }
    let directory = state.join("native");
    if directory.exists() {
        use std::os::unix::fs::FileTypeExt;
        daemon::private_directory(&directory)?;
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let stem = name
                .strip_suffix(".sock")
                .or_else(|| name.strip_suffix(".gate"));
            if stem.is_some_and(|stem| {
                stem.len() == 16
                    && stem
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
            }) && entry.file_type()?.is_socket()
            {
                std::fs::remove_file(entry.path())?;
            }
        }
    }
    Ok(())
}

pub(crate) fn text_input(input: &Value) -> Result<String> {
    let parts = input
        .as_array()
        .ok_or_else(|| invalid("managed terminal accepts text messages only"))?;
    let mut text = String::new();
    for part in parts {
        require(
            part["type"] == "text",
            "managed terminal accepts text messages only",
        )?;
        let value = part["text"]
            .as_str()
            .ok_or_else(|| invalid("text input is missing text"))?;
        require(
            text.len() + value.len() <= 1024 * 1024,
            "native message exceeds 1 MiB",
        )?;
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(value);
    }
    require(!text.trim().is_empty(), "native message is empty")?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn restart_config(directory: &Path, provider: ProviderKind) -> crate::config::Config {
        let mut config =
            crate::config::Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
        config.profiles.insert(
            "fixture".into(),
            crate::config::Profile {
                provider,
                binary: Some(directory.join(agent::name(provider))),
                args: vec![],
                command: None,
                model: None,
                effort: crate::config::Effort::Low,
            },
        );
        config
            .providers
            .entry(provider)
            .or_insert_with(|| crate::config::ProviderConfig {
                binary: directory.join(agent::name(provider)),
                args: vec![],
                command: None,
            });
        config
    }

    #[test]
    fn daemon_restart_reaps_native_processes_and_restores_the_same_identity() {
        const CHILD: &str = "ANTISLOP_NATIVE_RESTART_FIXTURE";
        if let Some(directory) = std::env::var_os(CHILD) {
            let directory = PathBuf::from(directory);
            let provider = if directory.join("codex").exists() {
                ProviderKind::Codex
            } else {
                ProviderKind::Opencode
            };
            let core = Core::open(
                &directory.join(".antislop"),
                restart_config(&directory, provider),
            )
            .unwrap();
            let conversation = core
                .store
                .create_conversation(directory.to_str().unwrap())
                .unwrap();
            let turn = core
                .store
                .start_turn(
                    &conversation.id,
                    "orchestrator",
                    agent::name(provider),
                    0,
                    None,
                )
                .unwrap();
            let spec = agent::Spec {
                provider,
                binary: directory.join(agent::name(provider)),
                args: vec![],
                model: None,
                effort: None,
                cwd: directory.clone(),
                add_dirs: vec![],
                access: agent::Access::ReadOnly,
                resume: None,
            };
            let session = core
                .native
                .prepare_scoped_with(&core, &conversation.id, "orchestrator", &spec, "", |_| true)
                .unwrap()
                .unwrap();
            core.store
                .record_run_context(&store::RunContext {
                    turn_id: turn.id.clone(),
                    conversation: conversation.id.clone(),
                    role: "orchestrator".into(),
                    provider: agent::name(provider).into(),
                    profile: "fixture".into(),
                    cwd: directory.clone(),
                    add_dirs: vec![],
                    access: store::RunAccess::ReadOnly,
                    backend: session.backend_kind(),
                    provider_thread_id: (provider == ProviderKind::Codex)
                        .then(|| session.identity.clone()),
                    provider_session_id: (provider == ProviderKind::Opencode)
                        .then(|| session.identity.clone()),
                    provider_turn_id: None,
                })
                .unwrap();
            core.store.set_session(&turn.id, &session.identity).unwrap();
            session.started.store(true, Ordering::SeqCst);
            let screen = core
                .terminal_open(&conversation.id, "orchestrator", 40, 8, false)
                .unwrap();
            std::fs::write(core.state.join("restart-fixture.json"), json!({"conversation":conversation.id,"turn":turn.id,"identity":session.identity,"handle":screen.handle}).to_string()).unwrap();
            // Simulate an abruptly lost daemon: no destructors or shutdown.
            std::process::exit(0);
        }
        for provider in [ProviderKind::Codex, ProviderKind::Opencode] {
            let directory = tempfile::tempdir_in("/private/tmp").unwrap();
            let binary = directory.path().join(agent::name(provider));
            std::fs::write(&binary, include_str!("native/test_backend.py")).unwrap();
            std::fs::set_permissions(&binary, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .unwrap();
            let child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "lean::native::tests::daemon_restart_reaps_native_processes_and_restores_the_same_identity", "--nocapture"])
                .env(CHILD, directory.path()).output().unwrap();
            assert!(
                child.status.success(),
                "{}",
                String::from_utf8_lossy(&child.stderr)
            );
            let state = directory.path().join(".antislop");
            let saved: Value =
                serde_json::from_slice(&std::fs::read(state.join("restart-fixture.json")).unwrap())
                    .unwrap();
            let core = Core::open(&state, restart_config(directory.path(), provider)).unwrap();
            let conversation = saved["conversation"].as_str().unwrap();
            let turn = core
                .store
                .last_turn(conversation, "orchestrator")
                .unwrap()
                .unwrap();
            assert_eq!(turn.id, saved["turn"]);
            assert_eq!(turn.status, store::TurnStatus::Interrupted);
            let count: u32 = core
                .store
                .db()
                .query_row("SELECT count(*) FROM native_processes", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "orphan process registrations survived restart");
            let session = core
                .native
                .restore_with(&core, conversation, "orchestrator", |_| true)
                .unwrap();
            assert_eq!(session.identity, saved["identity"]);
            let screen = core
                .terminal_open(conversation, "orchestrator", 40, 8, false)
                .unwrap();
            assert_ne!(screen.handle, saved["handle"]);
            assert!(
                core.terminal_input(
                    conversation,
                    "orchestrator",
                    saved["handle"].as_str().unwrap(),
                    1,
                    super::super::terminal::Input::Paste {
                        text: "stale".into()
                    }
                )
                .is_err()
            );
            let context = core
                .store
                .run_context(conversation, &turn.id)
                .unwrap()
                .unwrap();
            assert_eq!(context.backend, session.backend_kind());
            core.native.shutdown().unwrap();
        }
    }
    #[test]
    fn unsupported_server_protocol_falls_back_before_sending_a_prompt() {
        for provider in [ProviderKind::Codex, ProviderKind::Opencode] {
            let directory = tempfile::tempdir_in("/private/tmp").unwrap();
            let binary = directory.path().join("native-fixture");
            std::fs::write(&binary, include_str!("native/test_backend.py")).unwrap();
            std::fs::set_permissions(&binary, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .unwrap();
            let config =
                crate::config::Config::from_toml(include_str!("../../examples/antislop.toml"))
                    .unwrap();
            let core = Core::open(&directory.path().join(".antislop"), config).unwrap();
            std::fs::write(core.state.join("unsupported-native-protocol"), "").unwrap();
            let conversation = core
                .store
                .create_conversation(directory.path().to_str().unwrap())
                .unwrap();
            let spec = agent::Spec {
                provider,
                binary,
                args: vec![],
                model: None,
                effort: None,
                cwd: directory.path().into(),
                add_dirs: vec![],
                access: agent::Access::ReadOnly,
                resume: None,
            };
            assert!(
                core.native
                    .prepare_scoped_with(&core, &conversation.id, "orchestrator", &spec, "", |_| {
                        true
                    })
                    .unwrap()
                    .is_none()
            );
            assert!(
                core.native
                    .get(&conversation.id, "orchestrator")
                    .unwrap()
                    .is_none()
            );
            let calls =
                std::fs::read_to_string(directory.path().join("native-calls.jsonl")).unwrap();
            assert!(!calls.contains("turn/start"));
            assert!(!calls.contains("prompt_async"));
            let processes: u32 = core
                .store
                .db()
                .query_row("SELECT count(*) FROM native_processes", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(processes, 0, "failed preflight left a backend behind");
        }
    }
    #[test]
    fn native_messages_reject_non_text_and_empty_input() {
        assert_eq!(
            text_input(&json!([{"type":"text","text":"one"},{"type":"text","text":"two"}]))
                .unwrap(),
            "one\ntwo"
        );
        assert!(text_input(&json!([{"type":"image","url":"file:///foreign"}])).is_err());
        assert!(text_input(&json!([])).is_err());
        assert!(text_input(&json!([{"type":"text","text":" "}])).is_err());
    }
    fn native_round_rotation(provider: ProviderKind) {
        // Short socket paths are required by Unix-domain socket platforms.
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let binary = directory.path().join("native-fixture");
        std::fs::write(&binary, include_str!("native/test_backend.py")).unwrap();
        std::fs::set_permissions(&binary, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let config =
            crate::config::Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
        let mut config = config;
        config.profiles.insert(
            "fixture".into(),
            crate::config::Profile {
                provider,
                binary: Some(binary.clone()),
                args: vec![],
                command: None,
                model: None,
                effort: crate::config::Effort::Low,
            },
        );
        config
            .providers
            .entry(provider)
            .or_insert_with(|| crate::config::ProviderConfig {
                binary: binary.clone(),
                args: vec![],
                command: None,
            });
        let core = Core::open(&directory.path().join(".antislop"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(directory.path().to_str().unwrap())
            .unwrap();
        let mut task = core
            .store
            .add_plan(&conversation.id, &[json!({"id":"task","title":"Task"})])
            .unwrap()
            .remove(0);
        let mut spec = agent::Spec {
            provider,
            binary,
            args: vec![],
            model: None,
            effort: provider
                .native_effort(crate::config::Effort::Low)
                .map(str::to_owned),
            cwd: directory.path().into(),
            add_dirs: vec![],
            access: agent::Access::ReadOnly,
            resume: None,
        };
        let mut previous: HashMap<String, Arc<Session>> = HashMap::new();
        for round in 1..=2 {
            task.round = round;
            core.store.update_task(&task).unwrap();
            for role in [
                format!("worker:{}", task.id),
                format!("verifier:{}", task.id),
            ] {
                let turn = core
                    .store
                    .start_turn(
                        &conversation.id,
                        &role,
                        agent::name(provider),
                        0,
                        Some(&task.plan_id),
                    )
                    .unwrap();
                spec.resume = None;
                let fingerprint = crate::lean::rounds::spec_signature(
                    "fixture",
                    crate::config::Effort::Low,
                    &spec,
                )
                .unwrap();
                let scope = format!(
                    "{:?}:fixture:{fingerprint}",
                    crate::lean::SessionPolicy::TaskRound {
                        task: task.id.clone(),
                        round
                    }
                );
                let current = core
                    .native
                    .prepare_scoped_with(&core, &conversation.id, &role, &spec, &scope, |_| true)
                    .unwrap()
                    .unwrap();
                spec.resume = Some(current.identity.clone());
                let continued = core
                    .native
                    .prepare_scoped_with(&core, &conversation.id, &role, &spec, &scope, |_| {
                        panic!("same-round session must not be recreated")
                    })
                    .unwrap()
                    .unwrap();
                assert!(Arc::ptr_eq(&current, &continued));
                core.store
                    .record_run_context(&store::RunContext {
                        turn_id: turn.id.clone(),
                        conversation: conversation.id.clone(),
                        role: role.clone(),
                        provider: agent::name(provider).into(),
                        profile: "fixture".into(),
                        cwd: spec.cwd.clone(),
                        add_dirs: vec![],
                        access: store::RunAccess::ReadOnly,
                        backend: current.backend_kind(),
                        provider_thread_id: (provider == ProviderKind::Codex)
                            .then(|| current.identity.clone()),
                        provider_session_id: (provider == ProviderKind::Opencode)
                            .then(|| current.identity.clone()),
                        provider_turn_id: None,
                    })
                    .unwrap();
                core.store.set_session(&turn.id, &current.identity).unwrap();
                core.store
                    .append_event(
                        &conversation.id,
                        Some(&turn.id),
                        Some(&task.plan_id),
                        "turn_started",
                        &json!({"execution_signature":fingerprint}),
                    )
                    .unwrap();
                core.store
                    .finish_turn(&turn.id, store::TurnStatus::Done, None)
                    .unwrap();
                let restored = core.native.restore(&core, &conversation.id, &role).unwrap();
                assert!(Arc::ptr_eq(&restored, &current));
                let mut updated = (*core.config_snapshot()).clone();
                updated.profiles.get_mut("fixture").unwrap().model = Some("changed-model".into());
                *core.config.write().unwrap() = Arc::new(updated.clone());
                let rejected = core
                    .native
                    .restore(&core, &conversation.id, &role)
                    .err()
                    .expect("changed profile must not resume an old native thread");
                assert!(
                    rejected
                        .to_string()
                        .contains("native execution profile changed")
                );
                updated.profiles.get_mut("fixture").unwrap().model = None;
                *core.config.write().unwrap() = Arc::new(updated);
                if round == 1 {
                    // The fixture has no model rollout; mark its synthetic first turn ready for attachment.
                    current.started.store(true, Ordering::SeqCst);
                    core.terminal_open(&conversation.id, &role, 40, 8, false)
                        .unwrap();
                    assert!(current.terminal.lock().unwrap().is_some());
                    let until = Instant::now() + Duration::from_secs(3);
                    loop {
                        let calls =
                            std::fs::read_to_string(directory.path().join("native-calls.jsonl"))
                                .unwrap();
                        if calls
                            .lines()
                            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                            .any(|call| {
                                call["method"] == "terminal_attached"
                                    && call["params"]["identity"]
                                        .as_array()
                                        .unwrap()
                                        .iter()
                                        .any(|arg| arg == &current.identity)
                            })
                        {
                            break;
                        }
                        assert!(Instant::now() < until, "mock terminal did not attach");
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
                if let Some(old) = previous.insert(role.clone(), current.clone()) {
                    assert_ne!(old.identity, current.identity);
                    assert!(old.stop.load(Ordering::SeqCst));
                    assert!(old.terminal.lock().unwrap().is_none());
                    let calls =
                        std::fs::read_to_string(directory.path().join("native-calls.jsonl"))
                            .unwrap();
                    for call in calls
                        .lines()
                        .map(|line| serde_json::from_str::<Value>(line).unwrap())
                    {
                        if call["method"] == "terminal_attached"
                            && call["params"]["identity"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .any(|arg| arg == &old.identity)
                        {
                            let pid = call["pid"].as_i64().unwrap() as i32;
                            assert_eq!(
                                unsafe { libc::kill(pid, 0) },
                                -1,
                                "old terminal process stayed alive"
                            );
                            assert_eq!(
                                std::io::Error::last_os_error().raw_os_error(),
                                Some(libc::ESRCH)
                            );
                        }
                    }
                    assert!(
                        old.process
                            .child
                            .lock()
                            .unwrap()
                            .try_wait()
                            .unwrap()
                            .is_some()
                    );
                    let until = Instant::now() + Duration::from_secs(3);
                    loop {
                        let open = match provider {
                            ProviderKind::Codex => std::os::unix::net::UnixStream::connect(
                                old.endpoint.strip_prefix("unix://").unwrap(),
                            )
                            .is_ok(),
                            ProviderKind::Opencode => std::net::TcpStream::connect(
                                old.gateway_address
                                    .lock()
                                    .unwrap()
                                    .as_ref()
                                    .unwrap()
                                    .strip_prefix("http://")
                                    .unwrap(),
                            )
                            .is_ok(),
                            _ => unreachable!(),
                        };
                        if !open {
                            break;
                        }
                        assert!(Instant::now() < until, "old gateway stayed available");
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
            }
        }
        let calls = std::fs::read_to_string(directory.path().join("native-calls.jsonl")).unwrap();
        let calls: Vec<Value> = calls
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let create = if provider == ProviderKind::Codex {
            "thread/start"
        } else {
            "/session"
        };
        assert_eq!(
            calls.iter().filter(|call| call["method"] == create).count(),
            4
        );
        assert!(!calls.iter().any(|call| call["method"] == "thread/resume"
            || call["method"].as_str().unwrap().starts_with("/session/")));
        assert_eq!(core.store.sessions(&conversation.id).unwrap().len(), 4);
        core.native.shutdown().unwrap();
    }

    #[test]
    fn codex_rounds_create_new_threads_and_retire_previous_backends() {
        native_round_rotation(ProviderKind::Codex);
    }
    #[test]
    fn opencode_rounds_create_new_sessions_and_retire_previous_backends() {
        native_round_rotation(ProviderKind::Opencode);
    }

    fn attach_smoke(provider: ProviderKind, binary: &str) {
        let directory = tempfile::tempdir_in("/private/tmp").unwrap();
        let config =
            crate::config::Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
        let core = Core::open(&directory.path().join(".antislop"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(directory.path().to_str().unwrap())
            .unwrap();
        let spec = agent::Spec {
            provider,
            binary: binary.into(),
            args: vec![],
            model: None,
            effort: None,
            cwd: directory.path().into(),
            add_dirs: vec![],
            access: agent::Access::ReadOnly,
            resume: None,
        };
        let native = core
            .native
            .prepare(&core, &conversation.id, "orchestrator", &spec)
            .unwrap()
            .expect("native adapter");
        let turn = core
            .store
            .start_turn(
                &conversation.id,
                "orchestrator",
                agent::name(provider),
                0,
                None,
            )
            .unwrap();
        let live_run = {
            core.store
                .record_run_context(&store::RunContext {
                    turn_id: turn.id.clone(),
                    conversation: conversation.id.clone(),
                    role: "orchestrator".into(),
                    provider: agent::name(provider).into(),
                    profile: "smoke".into(),
                    cwd: spec.cwd.clone(),
                    add_dirs: vec![],
                    access: store::RunAccess::ReadOnly,
                    backend: native.backend_kind(),
                    provider_thread_id: (provider == ProviderKind::Codex)
                        .then(|| native.identity.clone()),
                    provider_session_id: (provider == ProviderKind::Opencode)
                        .then(|| native.identity.clone()),
                    provider_turn_id: None,
                })
                .unwrap();
            let cancelled = Arc::new(AtomicBool::new(false));
            let timeout = cancelled.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(60));
                timeout.store(true, Ordering::SeqCst);
            });
            let running = native.clone();
            let turn_id = turn.id.clone();
            let run = std::thread::spawn(move || {
                running.run(
                    &turn_id,
                    "Reply with exactly ANTISLOP_SMOKE. Do not call tools.",
                    &[],
                    &cancelled,
                    &mut |event| {
                        if std::env::var_os("ANTISLOP_NATIVE_DEBUG").is_some() {
                            eprintln!("Native smoke event: {event:?}");
                        }
                    },
                )
            });
            let deadline = Instant::now() + Duration::from_secs(10);
            while !(if provider == ProviderKind::Codex {
                native.started.load(Ordering::SeqCst)
            } else {
                native.executing.load(Ordering::SeqCst)
            }) {
                assert!(
                    Instant::now() < deadline && !run.is_finished(),
                    "native turn did not start"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(
                native.executing.load(Ordering::SeqCst),
                "attach must begin during execution"
            );
            Some(run)
        };
        let screen = core
            .terminal_open(&conversation.id, "orchestrator", 80, 24, false)
            .unwrap();
        let until = Instant::now() + Duration::from_secs(20);
        let mut visible = None;
        let mut text = String::new();
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(100));
            if let Some(screen) = core
                .terminal_read(&conversation.id, "orchestrator", &screen.handle, 0)
                .unwrap()
            {
                text = screen.cells.iter().map(|cell| cell.text.as_str()).collect();
                assert!(screen.error.is_none(), "{text}: {:?}", screen.error);
                let ready = text.contains("ANTISLOP_SMOKE");
                if ready {
                    let first = visible.get_or_insert_with(Instant::now);
                    if first.elapsed() > Duration::from_secs(1) {
                        break;
                    }
                }
            }
        }
        assert!(!text.trim().is_empty(), "native CLI produced no screen");
        assert!(
            !text.contains("not supported in an antislop") && !text.contains("ManagedSessionError"),
            "{text}"
        );
        assert!(!native.stop.load(Ordering::SeqCst));
        if let Some(run) = live_run {
            let result = run.join().unwrap();
            assert!(result.is_ok(), "{result:?}; native screen: {text}");
            assert!(result.unwrap().text.contains("ANTISLOP_SMOKE"));
        }
        {
            // Exercise the real TUI's submit route while holding the role
            // lock. The accepted message is durable, but no model is invoked.
            let lock = core.turn_lock(&conversation.id, "orchestrator");
            let guard = lock.lock().unwrap();
            struct CancelOnDrop<'a>(&'a Core, &'a str);
            impl Drop for CancelOnDrop<'_> {
                fn drop(&mut self) {
                    self.0.cancel(self.1);
                }
            }
            // Cancel even if an assertion fails, before releasing the role lock.
            let cancel = CancelOnDrop(&core, &conversation.id);
            core.terminal_input(
                &conversation.id,
                "orchestrator",
                &screen.handle,
                1,
                super::super::terminal::Input::Paste {
                    text: "ANTISLOP_QUEUE_SMOKE".into(),
                },
            )
            .unwrap();
            std::thread::sleep(Duration::from_millis(100));
            core.terminal_input(
                &conversation.id,
                "orchestrator",
                &screen.handle,
                2,
                super::super::terminal::Input::Key {
                    event: crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::Enter,
                        crossterm::event::KeyModifiers::NONE,
                    ),
                },
            )
            .unwrap();
            let until = Instant::now() + Duration::from_secs(5);
            loop {
                let mail = core.store.mail(&conversation.id).unwrap();
                if mail
                    .iter()
                    .any(|mail| mail.sender == "user" && mail.text.contains("ANTISLOP_QUEUE_SMOKE"))
                {
                    break;
                }
                assert!(
                    Instant::now() < until,
                    "native CLI did not enqueue its message"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
            drop(cancel);
            drop(guard);
        }
        core.store
            .finish_turn(&turn.id, store::TurnStatus::Done, None)
            .unwrap();
    }
    #[test]
    #[ignore = "uses the locally installed Codex, account configuration and one short model turn"]
    fn installed_codex_attach_smoke() {
        attach_smoke(ProviderKind::Codex, "codex");
    }
    #[test]
    #[ignore = "uses the locally installed OpenCode, account configuration and one short model turn"]
    fn installed_opencode_attach_smoke() {
        attach_smoke(ProviderKind::Opencode, "opencode");
    }
}
