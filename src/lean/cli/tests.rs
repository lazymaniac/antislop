use super::*;
use crate::lean::trust::Registry;
use std::io::Cursor;

fn test_tempdir() -> tempfile::TempDir {
    std::fs::create_dir_all(".antislop/tmp").unwrap();
    tempfile::tempdir_in(".antislop/tmp").unwrap()
}

fn ask(
    registry: &Registry,
    dir: &std::path::Path,
    flag: bool,
    tty: bool,
    input: &str,
) -> (Result<()>, String) {
    let mut shown = Vec::new();
    let result = ensure_trusted(
        registry,
        dir,
        flag,
        tty,
        Cursor::new(input.to_owned()),
        &mut shown,
    );
    (result, String::from_utf8(shown).unwrap())
}

#[test]
fn the_directory_is_asked_about_once_and_never_without_a_terminal() {
    let config = test_tempdir();
    let project = test_tempdir();
    let registry = Registry::at(config.path());

    // No terminal and no flag: nobody to ask, so no.
    let (result, shown) = ask(&registry, project.path(), false, false, "y\n");
    assert!(result.unwrap_err().to_string().contains("--trust"));
    assert!(shown.is_empty() && !registry.is_trusted(project.path()));

    // Declining (or an empty answer) leaves it untrusted.
    for answer in ["n\n", "\n", "maybe\n"] {
        let (result, shown) = ask(&registry, project.path(), false, true, answer);
        assert!(result.is_err());
        assert!(shown.contains("Do you trust the files in this directory?"));
        assert!(!registry.is_trusted(project.path()));
    }

    // Yes is remembered: the next start does not ask.
    let (result, _) = ask(&registry, project.path(), false, true, "Yes\n");
    result.unwrap();
    let (result, shown) = ask(&registry, project.path(), false, false, "");
    result.unwrap();
    assert!(shown.is_empty());

    // The flag answers for a directory not yet known.
    let other = test_tempdir();
    let (result, _) = ask(&registry, other.path(), true, false, "");
    result.unwrap();
    assert!(registry.is_trusted(other.path()));
}

#[test]
fn parsing_the_options_creates_nothing() {
    let temp = test_tempdir();
    let state = temp.path().join("state");
    assert!(!state.exists(), "test state path must start absent");
    let options = Options::parse(&[
        "--config".into(),
        "Cargo.toml".into(),
        "--state".into(),
        state.to_string_lossy().into_owned(),
        "hi".into(),
    ])
    .unwrap();
    assert_eq!(options.state, state);
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

    let error = resolve_config(project.path(), Some(Path::new("missing.toml"))).unwrap_err();
    assert!(error.to_string().contains("missing.toml"));
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
