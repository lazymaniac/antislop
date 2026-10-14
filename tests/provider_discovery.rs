use architect::{config::*, domain::Run, id, providers::discovery::*};
use std::os::unix::fs::PermissionsExt;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
fn run(root: &Path, cli: &Path) -> Run {
    let config = architect::runtime::intake::bootstrap(cli.into()).unwrap();
    let mut spec: architect::domain::RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.to_string_lossy().into();
    spec.config = Some(config);
    Run::new(id(), spec).unwrap()
}
fn executable(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}
#[test]
fn catalogue_finds_executables_without_replacing_configured_or_missing_choices() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    executable(
        &bin.join("codex"),
        "#!/bin/sh\nprintf 'codex-cli 0.162.1\\n'\n",
    );
    executable(
        &bin.join("claude"),
        "#!/bin/sh\nprintf 'public version only\\n'\n",
    );
    std::fs::write(bin.join("gemini"), "not executable").unwrap();
    let absent = root.path().join("explicit-codex-missing");
    let config = architect::runtime::intake::bootstrap(absent.clone())
        .unwrap()
        .config;
    let candidates = candidates(&config, bin.as_os_str());
    assert_eq!(candidates[&ProviderKind::Codex].binary, absent);
    assert_eq!(
        candidates[&ProviderKind::Claude].binary,
        bin.join("claude").canonicalize().unwrap()
    );
    let missing = probe_entry(
        &run(root.path(), &absent),
        ProviderKind::Gemini,
        candidates[&ProviderKind::Gemini].clone(),
        Arc::new(AtomicBool::new(false)),
        Duration::from_secs(1),
        bin.as_os_str(),
    )
    .unwrap();
    assert!(matches!(missing.state, State::Missing));
    let r = run(root.path(), &absent);
    let e = probe_entry(
        &r,
        ProviderKind::Codex,
        candidates[&ProviderKind::Codex].clone(),
        Arc::new(AtomicBool::new(false)),
        Duration::from_secs(1),
        bin.as_os_str(),
    )
    .unwrap();
    assert!(matches!(e.state, State::Missing));
    assert!(e.resolved_binary.is_none());
}
#[test]
fn compatible_probe_records_actual_version_without_claiming_login_or_model_availability() {
    let root = tempfile::tempdir().unwrap();
    let cli = root.path().join("codex");
    executable(&cli, "#!/bin/sh\nprintf 'codex-cli 0.162.1\\n'\n");
    let r = run(root.path(), &cli);
    let def = r.spec.config.as_ref().unwrap().config.providers[&ProviderKind::Codex].clone();
    let e = probe_entry(
        &r,
        ProviderKind::Codex,
        def,
        Arc::new(AtomicBool::new(false)),
        Duration::from_secs(1),
        root.path().as_os_str(),
    )
    .unwrap();
    assert!(matches!(e.state, State::Compatible));
    assert_eq!(
        e.capabilities.as_ref().unwrap().version,
        "codex-cli 0.162.1"
    );
    assert_eq!(e.authentication, architect::providers::Support::Unverified);
    assert_eq!(e.model_catalog, architect::providers::Support::Unverified);
    assert_eq!(
        e.resolved_binary.as_ref(),
        Some(&cli.canonicalize().unwrap())
    );
}
#[test]
fn incompatible_version_is_retained_as_a_rejection_not_an_automatic_replacement() {
    let root = tempfile::tempdir().unwrap();
    let cli = root.path().join("codex");
    executable(&cli, "#!/bin/sh\nprintf 'codex-cli 99.0.0\\n'\n");
    let r = run(root.path(), &cli);
    let def = r.spec.config.as_ref().unwrap().config.providers[&ProviderKind::Codex].clone();
    let e = probe_entry(
        &r,
        ProviderKind::Codex,
        def,
        Arc::new(AtomicBool::new(false)),
        Duration::from_secs(1),
        root.path().as_os_str(),
    )
    .unwrap();
    assert!(matches!(e.state, State::Rejected));
    assert!(e.capabilities.is_none());
    assert_eq!(e.configuration.binary, cli);
}
#[test]
fn bounded_probe_stops_its_owned_process_and_user_cancellation_remains_distinct() {
    let root = tempfile::tempdir().unwrap();
    let cli = root.path().join("codex");
    executable(
        &cli,
        "#!/usr/bin/env python3\nimport time\ntime.sleep(60)\n",
    );
    let r = run(root.path(), &cli);
    let def = r.spec.config.as_ref().unwrap().config.providers[&ProviderKind::Codex].clone();
    let start = Instant::now();
    let e = probe_entry(
        &r,
        ProviderKind::Codex,
        def.clone(),
        Arc::new(AtomicBool::new(false)),
        Duration::from_millis(120),
        root.path().as_os_str(),
    )
    .unwrap();
    assert!(matches!(e.state, State::TimedOut));
    assert!(start.elapsed() < Duration::from_secs(5));
    let token = Arc::new(AtomicBool::new(false));
    let cancel = token.clone();
    let trigger = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        cancel.store(true, Ordering::SeqCst);
    });
    let result = probe_entry(
        &r,
        ProviderKind::Codex,
        def,
        token,
        Duration::from_secs(2),
        root.path().as_os_str(),
    );
    trigger.join().unwrap();
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("discovery cancelled")
    );
}

#[test]
fn compatibility_probe_does_not_assign_a_cli_to_a_user_restricted_role() {
    let root = tempfile::tempdir().unwrap();
    let cli = root.path().join("codex");
    executable(&cli, "#!/bin/sh\nprintf 'codex-cli 0.162.1\\n'\n");
    let mut r = run(root.path(), &cli);
    let mut config = r.spec.config.as_ref().unwrap().config.clone();
    let mut claude = config.providers[&ProviderKind::Codex].clone();
    claude.native_effort = false;
    claude.effort_map.clear();
    claude.isolation = IsolationProfile::ToolFree;
    config.providers.insert(ProviderKind::Claude, claude);
    config.profiles.get_mut("orchestrator").unwrap().provider = ProviderKind::Claude;
    config.role_providers = [
        (RoleKind::Orchestrator, [ProviderKind::Claude].into()),
        (RoleKind::Executor, [ProviderKind::Codex].into()),
        (RoleKind::Verifier, [ProviderKind::Codex].into()),
    ]
    .into();
    r.spec.config = Some(config.pin().unwrap());
    let before = r.spec.config.clone();
    let def = r.spec.config.as_ref().unwrap().config.providers[&ProviderKind::Codex].clone();
    let e = probe_entry(
        &r,
        ProviderKind::Codex,
        def,
        Arc::new(AtomicBool::new(false)),
        Duration::from_secs(1),
        root.path().as_os_str(),
    )
    .unwrap();
    assert!(matches!(e.state, State::Compatible), "{:?}", e.diagnostic);
    assert_eq!(r.spec.config, before);
    assert_eq!(
        r.spec
            .config
            .as_ref()
            .unwrap()
            .config
            .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
            .unwrap()
            .provider,
        ProviderKind::Claude
    );
}
