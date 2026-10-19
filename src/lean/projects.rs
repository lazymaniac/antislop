//! The projects the user has opened with antislop, most recent first, so that
//! the screen can offer them again. Each project keeps its own daemon, state
//! directory and configuration; this is only a list of where they are.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Longest list kept.
const KEEP: usize = 20;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    projects: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Projects {
    file: PathBuf,
}

impl Projects {
    /// Beside the list of trusted directories.
    pub fn from_env() -> Result<Self> {
        let registry = super::trust::Registry::from_env()?;
        Ok(Self::at(registry.path().parent().unwrap_or(Path::new("."))))
    }

    pub fn at(dir: &Path) -> Self {
        Self {
            file: dir.join("projects.json"),
        }
    }

    fn read(&self) -> Stored {
        std::fs::read_to_string(&self.file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// The remembered projects that still exist, most recent first.
    pub fn list(&self) -> Vec<PathBuf> {
        self.read()
            .projects
            .into_iter()
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .collect()
    }

    /// Puts `project` first. A list that cannot be written is only a missing
    /// convenience, so the error is returned for the caller to ignore.
    pub fn remember(&self, project: &Path) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let key = project
            .canonicalize()
            .unwrap_or_else(|_| project.to_path_buf())
            .to_string_lossy()
            .into_owned();
        let mut stored = self.read();
        stored.projects.retain(|entry| *entry != key);
        stored.projects.insert(0, key);
        stored.projects.truncate(KEEP);
        let dir = self.file.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        let temporary = self.file.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_string_pretty(&stored)?)
            .map_err(|error| Error::Invalid(error.to_string()))?;
        std::fs::rename(&temporary, &self.file)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_project_comes_first_and_gone_ones_are_left_out() {
        let config = tempfile::tempdir().unwrap();
        let (a, b, c) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let projects = Projects::at(config.path());
        assert!(projects.list().is_empty());
        for dir in [&a, &b, &c, &a] {
            projects.remember(dir.path()).unwrap();
        }
        let canonical = |d: &tempfile::TempDir| d.path().canonicalize().unwrap();
        assert_eq!(
            projects.list(),
            [canonical(&a), canonical(&c), canonical(&b)]
        );
        let gone = canonical(&b);
        drop(b);
        assert!(!projects.list().contains(&gone));
        std::fs::write(config.path().join("projects.json"), "garbage").unwrap();
        assert!(projects.list().is_empty(), "a damaged list is just empty");
    }
}
