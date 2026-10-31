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
        revising: Mutex::new(std::collections::BTreeSet::new()),
        revision_jobs: Mutex::new(HashMap::new()),
        revision_views: Mutex::new(HashMap::new()),
        agent_reviews: Mutex::new(HashMap::new()),
        task_cancels: Mutex::new(HashMap::new()),
        task_restarts: Mutex::new(HashMap::new()),
        orchestrator_restarts: Mutex::new(std::collections::HashSet::new()),
        opened_ms: store::now_ms(),
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
    core.store.interrupt_revision_jobs()?;
    if let Err(error) =
        crate::lean::core_path_helpers::release_settled_views(&core.store, &core.state)
    {
        eprintln!("antislop: settled task checkouts were not removed: {error}");
    }
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
    if core.unanswered(conversation)?.is_none() {
        core.store.queue_requirements_format_repair(conversation)?;
    }
    wake_pending_work(core, conversation)
}

fn wake_pending_work(core: &Arc<Core>, conversation: &str) -> Result<()> {
    if core.store.has_pending_requirements_approval(conversation)? {
        core.wake(conversation);
    }
    if core.store.workflow(conversation)?.planning() && core.unanswered(conversation)?.is_some() {
        core.wake(conversation);
    }
    Ok(())
}

fn resume_cut_off(core: &Arc<Core>, conversations: &[String]) {
    for conversation in conversations {
        core.wake(conversation);
    }
}

/// Continues the plans whose work the restart interrupted. Only a
/// conversation's current plan runs again, and only the work that was cut
/// off: tasks waiting for an answer or handed to the orchestrator or the
/// user stay as they were.
fn resume_interrupted(core: &Arc<Core>, plans: &[(String, String)]) {
    for (conversation, plan) in plans {
        let resumed = core.store.accepted_plan(conversation).and_then(|current| {
            if current.as_deref() != Some(plan.as_str()) {
                return Ok(());
            }
            core.store.requeue_after_restart(conversation, plan)?;
            core.maybe_continue_plan(conversation, plan)
        });
        if let Err(error) = resumed {
            core.notice(
                conversation,
                &format!("could not resume plan {plan}: {error}"),
            );
        }
    }
}

fn answer_pending_mail(core: &Arc<Core>) -> Result<()> {
    for conversation in core.store.pending_mail_conversations()? {
        core.wake(&conversation);
        core.resume_agent_reviews(&conversation);
    }
    Ok(())
}
