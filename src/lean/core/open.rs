use super::*;

type InterruptedWork = (Vec<String>, Vec<(String, String)>);

impl Core {
    pub fn open(state: &Path, config: Config) -> Result<Arc<Self>> {
        let lock = state_lock(state)?;
        let store = Store::open(&state.join("core.db"))?;
        let interrupted = recover_agents_and_turns(&store, state)?;
        let core = make_core(state, config, store, lock)?;
        restore_startup(&core, interrupted)?;
        Ok(core)
    }
}

fn state_lock(state: &Path) -> Result<std::fs::File> {
    daemon::private_directory(state)?;
    write_state_gitignore(state)?;
    let lock = open_lock_file(state)?;
    acquire_lock(&lock, state)?;
    Ok(lock)
}

fn write_state_gitignore(state: &Path) -> Result<()> {
    let ignore = state.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(ignore, "*\n")?;
    }
    Ok(())
}

fn open_lock_file(state: &Path) -> Result<std::fs::File> {
    Ok(std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(state.join("core.lock"))?)
}

fn acquire_lock(lock: &std::fs::File, state: &Path) -> Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !try_lock(lock) {
        require(
            std::time::Instant::now() < deadline,
            &format!("another antislop process owns {}", state.display()),
        )?;
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    Ok(())
}

fn try_lock(lock: &std::fs::File) -> bool {
    // SAFETY: the descriptor remains owned by Core for the lock's lifetime.
    unsafe {
        libc::flock(
            std::os::fd::AsRawFd::as_raw_fd(lock),
            libc::LOCK_EX | libc::LOCK_NB,
        ) == 0
    }
}

fn recover_agents_and_turns(store: &Store, state: &Path) -> Result<InterruptedWork> {
    native::recover(store, state)?;
    reap_orphaned_agents(store)?;
    let cut_off = store.interrupt_running_turns()?;
    let interrupted = store.interrupt_unfinished_tasks()?;
    Ok((cut_off, interrupted))
}

fn reap_orphaned_agents(store: &Store) -> Result<()> {
    for (pid, started_ms, provider) in store.running_processes()? {
        agent::reap(pid, started_ms, &provider);
    }
    Ok(())
}

fn make_core(state: &Path, config: Config, store: Store, lock: std::fs::File) -> Result<Arc<Core>> {
    let core = Arc::new(Core {
        store,
        native: native::Registry::default(),
        weak: std::sync::OnceLock::new(),
        _lock: lock,
        config: RwLock::new(Arc::new(config)),
        config_update: Mutex::new(()),
        config_modified: std::sync::atomic::AtomicU64::new(0),
        state: state.canonicalize()?,
        integrating: Mutex::new(()),
        graph_updates: Mutex::new(()),
        in_place: Mutex::new(()),
        mail_jobs: Mutex::new(HashMap::new()),
        task_cancels: Mutex::new(HashMap::new()),
        task_restarts: Mutex::new(HashMap::new()),
        orchestrator_restarts: Mutex::new(std::collections::HashSet::new()),
        mail_failures: Mutex::new(HashMap::new()),
        turn_locks: Mutex::new(HashMap::new()),
        running: Mutex::new(HashMap::new()),
        plans: Mutex::new(HashMap::new()),
        reviews: Mutex::new(HashMap::new()),
        blocked_seen: Mutex::new(HashMap::new()),
    });
    core.weak.set(Arc::downgrade(&core)).ok();
    spawn_native_reaper(&core);
    Ok(core)
}

fn spawn_native_reaper(core: &Arc<Core>) {
    let weak = Arc::downgrade(core);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let Some(core) = weak.upgrade() else { break };
            let _ = core.native.reap_idle();
        }
    });
}

fn restore_startup(core: &Arc<Core>, interrupted: InterruptedWork) -> Result<()> {
    core.store.recover_approved_requirements()?;
    restore_conversations(core)?;
    resume_cut_off(core, &interrupted.0);
    resume_interrupted(core, &interrupted.1);
    answer_pending_mail(core)?;
    Ok(())
}

fn restore_conversations(core: &Arc<Core>) -> Result<()> {
    for conversation in core.store.conversations()? {
        restore_conversation(core, &conversation.id)?;
    }
    Ok(())
}

fn restore_conversation(core: &Arc<Core>, conversation: &str) -> Result<()> {
    if core.autonomous(conversation)? {
        core.store.finish_interview(conversation)?;
    }
    queue_format_repair(core, conversation)?;
    wake_pending_work(core, conversation)
}

fn queue_format_repair(core: &Arc<Core>, conversation: &str) -> Result<()> {
    let workflow = core.store.workflow(conversation)?;
    if workflow.is_some_and(|workflow| workflow.phase == "interview") {
        return Ok(());
    }
    let Some(turn) = core.store.last_orchestration_turn(conversation)? else {
        return Ok(());
    };
    if turn.status != TurnStatus::Done {
        return Ok(());
    }
    let Ok(Some(answer)) = core.store.turn_final_answer(conversation, &turn.id) else {
        return Ok(());
    };
    if let (_, Some(Err(error))) = tasks::parse_plan(&answer) {
        core.store
            .queue_plan_format_repair(conversation, &turn.id, &error.to_string())?;
    }
    Ok(())
}

fn wake_pending_work(core: &Arc<Core>, conversation: &str) -> Result<()> {
    if core.store.has_pending_requirements_approval(conversation)? {
        core.wake(conversation);
    }
    let planning = core
        .store
        .workflow(conversation)?
        .is_some_and(|workflow| workflow.phase == "planning");
    if planning && core.unanswered(conversation)?.is_some() {
        core.wake(conversation);
    }
    Ok(())
}

fn resume_cut_off(core: &Arc<Core>, conversations: &[String]) {
    for conversation in conversations {
        core.wake(conversation);
    }
}

fn resume_interrupted(core: &Arc<Core>, plans: &[(String, String)]) {
    for (conversation, plan) in plans {
        if let Err(error) = core.resume_plan(conversation, plan) {
            core.notice(
                conversation,
                &format!("could not resume plan {plan}: {error}"),
            );
        }
    }
}

fn answer_pending_mail(core: &Arc<Core>) -> Result<()> {
    for conversation in core.store.pending_mail_conversations()? {
        core.answer_in_background(&conversation, "");
    }
    Ok(())
}
