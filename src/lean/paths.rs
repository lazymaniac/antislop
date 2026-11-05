//! Durable user-owned locations, independent of disposable worktrees.
use crate::{Error, Result};
use std::path::{Path, PathBuf};

fn environment_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Shared user configuration. Resolving a location creates no files.
pub fn config_dir() -> Result<PathBuf> {
    if let Some(path) = environment_path("ANTISLOP_CONFIG_DIR") {
        return Ok(path);
    }
    if let Some(path) = environment_path("XDG_CONFIG_HOME") {
        return Ok(path.join("antislop"));
    }
    environment_path("HOME")
        .map(|path| path.join(".config/antislop"))
        .ok_or_else(|| {
            Error::Invalid(
                "no user configuration directory: set HOME or ANTISLOP_CONFIG_DIR".into(),
            )
        })
}

/// The global runtime root; project state is always derived from stable identity.
pub fn state_root() -> Result<PathBuf> {
    resolve_state_root(
        environment_path("ANTISLOP_STATE_ROOT"),
        environment_path("XDG_STATE_HOME"),
        environment_path("HOME"),
    )
}

fn resolve_state_root(
    explicit: Option<PathBuf>,
    xdg: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Result<PathBuf> {
    let path = explicit
        .or_else(|| xdg.map(|path| path.join("antislop")))
        .or_else(|| home.map(|path| path.join(".local/state/antislop")))
        .ok_or_else(|| {
            Error::Invalid(
                "no durable runtime root: set ANTISLOP_STATE_ROOT, XDG_STATE_HOME or HOME".into(),
            )
        })?;
    crate::require(
        path.is_absolute(),
        "runtime root must be an absolute path, independent of the current worktree",
    )?;
    Ok(path)
}

pub fn project_state(project: &Path) -> Result<PathBuf> {
    state_in_root(&state_root()?, project)
}

fn state_in_root(root: &Path, project: &Path) -> Result<PathBuf> {
    Ok(root.join("projects").join(project_identity(project)?))
}

/// Stable repository identity; linked worktrees share their common Git directory.
pub fn project_identity(project: &Path) -> Result<String> {
    let project = project.canonicalize()?;
    let output = super::git::git(&project, &["rev-parse", "--git-common-dir"])?;
    let (kind, identity) = if output.status.success() {
        let common = String::from_utf8(output.stdout)
            .map_err(|error| Error::Invalid(format!("git common directory: {error}")))?;
        (
            "git",
            project.join(common.trim_end_matches('\n')).canonicalize()?,
        )
    } else {
        // A broken or inaccessible repository must not silently select a new database.
        let has_git = project
            .ancestors()
            .any(|path| path.join(".git").symlink_metadata().is_ok());
        if has_git || !String::from_utf8_lossy(&output.stderr).contains("not a git repository") {
            return Err(Error::Invalid(format!(
                "git common directory: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        ("directory", project)
    };
    let mut key = kind.as_bytes().to_vec();
    key.push(0);
    key.extend_from_slice(identity.as_os_str().as_encoded_bytes());
    Ok(crate::hash(&key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_root_precedence_is_explicit_then_xdg_then_home_without_writes() {
        let explicit = PathBuf::from("/fixture/explicit");
        let xdg = PathBuf::from("/fixture/xdg");
        let home = PathBuf::from("/fixture/home");
        assert_eq!(
            resolve_state_root(
                Some(explicit.clone()),
                Some(xdg.clone()),
                Some(home.clone())
            )
            .unwrap(),
            explicit
        );
        assert_eq!(
            resolve_state_root(None, Some(xdg), Some(home.clone())).unwrap(),
            PathBuf::from("/fixture/xdg/antislop")
        );
        assert_eq!(
            resolve_state_root(None, None, Some(home)).unwrap(),
            PathBuf::from("/fixture/home/.local/state/antislop")
        );
        assert!(resolve_state_root(None, None, None).is_err());
        assert!(resolve_state_root(Some("relative".into()), None, None).is_err());
    }

    #[test]
    fn linked_worktrees_share_state_and_cleanup_does_not_remove_it() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        super::super::git::ok(&project, &["init", "--quiet", "-b", "main"]).unwrap();
        super::super::git::ok(
            &project,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "fixture",
            ],
        )
        .unwrap();
        let checkout = temp.path().join("checkout");
        super::super::git::ok(
            &project,
            &[
                "worktree",
                "add",
                "--quiet",
                "--detach",
                checkout.to_str().unwrap(),
            ],
        )
        .unwrap();
        let root = temp.path().join("durable");
        let primary_state = state_in_root(&root, &project).unwrap();
        assert_eq!(primary_state, state_in_root(&root, &checkout).unwrap());
        assert!(!root.exists(), "resolving identity must create nothing");
        std::fs::create_dir_all(&primary_state).unwrap();
        let evidence = primary_state.join("checkpoint");
        std::fs::write(&evidence, "retained").unwrap();
        super::super::git::ok(
            &project,
            &["worktree", "remove", checkout.to_str().unwrap()],
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(evidence).unwrap(), "retained");
        assert_eq!(primary_state, state_in_root(&root, &project).unwrap());
    }

    #[test]
    fn canonical_non_git_directories_have_distinct_stable_state() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("one");
        let second = temp.path().join("two");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&first, &alias).unwrap();
        let root = temp.path().join("durable");
        assert_eq!(
            state_in_root(&root, &first).unwrap(),
            state_in_root(&root, &alias).unwrap()
        );
        assert_ne!(
            state_in_root(&root, &first).unwrap(),
            state_in_root(&root, &second).unwrap()
        );
        assert!(!root.exists());
    }
}
