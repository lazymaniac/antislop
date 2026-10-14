//! Runtime-owned public CLI probes. Compatibility is not authentication or
//! substantive model acceptance; failed candidates never become silent routes.
use crate::{
    Result,
    config::*,
    domain::Run,
    providers::{Capabilities, Support},
    require,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Missing,
    Compatible,
    Rejected,
    TimedOut,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub configuration: ProviderConfig,
    pub resolved_binary: Option<PathBuf>,
    pub state: State,
    pub capabilities: Option<Capabilities>,
    pub authentication: Support,
    pub model_catalog: Support,
    pub diagnostic: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalogue {
    pub entries: BTreeMap<ProviderKind, Entry>,
}

pub(crate) fn default_configuration(kind: ProviderKind, binary: PathBuf) -> ProviderConfig {
    let native = matches!(kind, ProviderKind::Codex | ProviderKind::Agy);
    ProviderConfig {
        binary,
        managed_home: None,
        isolation: if kind == ProviderKind::Codex {
            IsolationProfile::ReadOnly
        } else {
            IsolationProfile::ToolFree
        },
        native_effort: native,
        effort_map: if native {
            BTreeMap::from([
                (Effort::Light, "low".into()),
                (Effort::Standard, "medium".into()),
                (Effort::Deep, "high".into()),
            ])
        } else {
            BTreeMap::new()
        },
    }
}

pub fn path_binary(binary: &Path, search_path: &std::ffi::OsStr) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let candidate = |path: PathBuf| {
        let m = path.metadata().ok()?;
        (m.is_file() && m.permissions().mode() & 0o111 != 0)
            .then(|| path.canonicalize().ok())
            .flatten()
    };
    if binary.components().count() > 1 || binary.is_absolute() {
        return candidate(binary.to_owned());
    }
    std::env::split_paths(search_path)
        .filter(|p| p.is_absolute())
        .find_map(|dir| candidate(dir.join(binary)))
}

pub fn candidates(
    config: &Config,
    search_path: &std::ffi::OsStr,
) -> BTreeMap<ProviderKind, ProviderConfig> {
    let mut candidates = config.providers.clone();
    for (kind, name) in [
        (ProviderKind::Codex, "codex"),
        (ProviderKind::Claude, "claude"),
        (ProviderKind::Gemini, "gemini"),
        (ProviderKind::Agy, "agy"),
    ] {
        if candidates.contains_key(&kind) {
            continue;
        }
        {
            let binary =
                path_binary(Path::new(name), search_path).unwrap_or_else(|| PathBuf::from(name));
            candidates.insert(kind, default_configuration(kind, binary));
        }
    }
    candidates
}

pub fn probe_entry(
    run: &Run,
    kind: ProviderKind,
    configuration: ProviderConfig,
    cancelled: Arc<AtomicBool>,
    timeout: Duration,
    search_path: &std::ffi::OsStr,
) -> Result<Entry> {
    require(!cancelled.load(Ordering::SeqCst), "discovery cancelled")?;
    let resolved_binary = path_binary(&configuration.binary, search_path);
    if resolved_binary.is_none() {
        return Ok(Entry {
            configuration,
            resolved_binary,
            state: State::Missing,
            capabilities: None,
            authentication: Support::Unverified,
            model_catalog: Support::Unverified,
            diagnostic: Some(
                "Configured executable is absent or not executable; it was not replaced.".into(),
            ),
        });
    }
    let local_cancel = Arc::new(AtomicBool::new(false));
    let complete = Arc::new(AtomicBool::new(false));
    let timed_out = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (local_cancel, complete, timed_out, cancelled) = (
            local_cancel.clone(),
            complete.clone(),
            timed_out.clone(),
            cancelled.clone(),
        );
        std::thread::spawn(move || {
            let deadline = Instant::now() + timeout;
            while !complete.load(Ordering::SeqCst) {
                if cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline {
                    timed_out.store(!cancelled.load(Ordering::SeqCst), Ordering::SeqCst);
                    local_cancel.store(true, Ordering::SeqCst);
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })
    };
    let result = (|| {
        let mut probe_run = run.clone();
        let mut config = run
            .spec
            .config
            .as_ref()
            .ok_or_else(|| crate::Error::Invalid("discovery configuration missing".into()))?
            .config
            .clone();
        // Compatibility inspection does not assign this CLI to a task role.
        // The synthetic probe profile must not turn a user routing restriction
        // into a false "unsupported CLI" result. The real run stays unchanged.
        config.role_providers.clear();
        let mut fixed = configuration.clone();
        fixed.binary = resolved_binary.clone().unwrap();
        config.providers.insert(kind, fixed);
        config.profiles.insert(
            "discovery".into(),
            Profile {
                provider: kind,
                model: None,
                effort: Effort::Deep,
                work_kinds: [WorkKind::Orchestration].into(),
                roles: [RoleKind::Orchestrator].into(),
            },
        );
        config.orchestrator.profile = "discovery".into();
        if let Some(route) = config.routing.get_mut(&WorkKind::Orchestration) {
            route.executor = "discovery".into();
        }
        probe_run.spec.config = Some(config.pin()?);
        crate::providers::build_for_work(
            &probe_run,
            WorkKind::Orchestration,
            RoleKind::Orchestrator,
        )?
        .probe_cancelled(&local_cancel)
    })();
    complete.store(true, Ordering::SeqCst);
    let _ = watcher.join();
    require(!cancelled.load(Ordering::SeqCst), "discovery cancelled")?;
    let timed_out = timed_out.load(Ordering::SeqCst);
    let (state, capabilities, diagnostic) = match result {
        Ok(c) if !timed_out => (State::Compatible, Some(c), None),
        _ if timed_out => (
            State::TimedOut,
            None,
            Some("Owned CLI compatibility probe exceeded its budget and was stopped.".into()),
        ),
        Err(error) => (
            State::Rejected,
            None,
            Some(crate::tui::sanitize::sanitize(&error.to_string())),
        ),
        Ok(_) => unreachable!(),
    };
    Ok(Entry {
        configuration,
        resolved_binary,
        state,
        capabilities,
        authentication: Support::Unverified,
        model_catalog: Support::Unverified,
        diagnostic,
    })
}

pub fn allowed(
    round: &crate::orchestration::planning::Round,
) -> Result<BTreeMap<ProviderKind, ProviderConfig>> {
    let Some(catalogue) = &round.provider_catalogue else {
        return Ok(BTreeMap::new());
    };
    let context: serde_json::Value = crate::transport::decode(round.request_context.as_bytes())?;
    require(
        context["provider_catalogue"] == serde_json::to_value(catalogue)?,
        "discovery catalogue differs from the actual orchestration context",
    )?;
    Ok(catalogue
        .entries
        .iter()
        .filter(|(_, e)| matches!(e.state, State::Compatible) && e.capabilities.is_some())
        .map(|(k, e)| (*k, e.configuration.clone()))
        .collect())
}
