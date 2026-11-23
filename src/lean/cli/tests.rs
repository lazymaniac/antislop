use super::*;

fn test_tempdir() -> crate::test_resources::TestDir {
    crate::test_resources::tempdir()
}

#[test]
fn removed_trust_flag_is_rejected_without_a_compatibility_mode() {
    let error = ParsedArgs::parse(&["--trust".into()]).err().unwrap();
    assert!(error.to_string().contains("unknown option --trust"));
}

#[test]
fn parsing_the_options_creates_nothing() {
    let fixture = test_tempdir();
    let config = fixture.path().join("config.toml");
    let source = include_str!("../../../examples/antislop.toml");
    std::fs::write(&config, source).unwrap();
    let before = std::fs::read_dir(fixture.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    let state = fixture.path().join("state");
    assert!(!state.exists(), "test state path must start absent");
    let options = Options::parse(&[
        "--config".into(),
        config.to_string_lossy().into_owned(),
        "--state".into(),
        state.to_string_lossy().into_owned(),
        "hi".into(),
    ])
    .unwrap();
    assert_eq!(options.config, config.canonicalize().unwrap());
    assert_eq!(options.state, state);
    let after = std::fs::read_dir(fixture.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(after, before, "parsing must not change fixture entries");
    assert_eq!(std::fs::read_to_string(&config).unwrap(), source);
    assert!(!state.exists(), "parsing must not create the state path");
    assert_eq!(options.message.as_deref(), Some("hi"));
}

#[test]
fn explicit_config_wins_and_a_missing_explicit_path_does_not_fall_back() {
    let project = test_tempdir();
    let local = project.path().join("antislop.toml");
    let explicit = project.path().join("custom.toml");
    std::fs::write(&local, "local config").unwrap();
    std::fs::write(&explicit, "explicit config").unwrap();

    assert_eq!(
        resolve_config(project.path(), None).unwrap(),
        local.canonicalize().unwrap()
    );
    assert_eq!(
        resolve_config(project.path(), Some(Path::new("custom.toml"))).unwrap(),
        explicit.canonicalize().unwrap()
    );

    let missing = project.path().join("missing.toml");
    let error = resolve_config(project.path(), Some(Path::new("missing.toml"))).unwrap_err();
    assert!(
        error
            .to_string()
            .contains(missing.to_string_lossy().as_ref())
    );
    assert!(!missing.exists());
    assert!(!project.path().join(".antislop").exists());
}

#[test]
fn config_symlink_is_resolved_to_its_target() {
    let project = test_tempdir();
    let target = project.path().join("settings.toml");
    let link = project.path().join("selected.toml");
    std::fs::write(&target, "config").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&target, &link).unwrap();

    assert_eq!(
        resolve_config(project.path(), Some(Path::new("selected.toml"))).unwrap(),
        target.canonicalize().unwrap()
    );
}
