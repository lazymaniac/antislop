//! Which directories the user let antislop work in. Like other agent tools,
//! the answer is asked once per directory and remembered for the user, not
//! per project: deleting a project's `.antislop/` does not ask again, and a
//! cloned repository cannot bring its own answer.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Serialize, Deserialize)]
struct Stored {
    trusted: Vec<String>,
}

/// The user's list of trusted directories.
#[derive(Debug, Clone)]
pub struct Registry {
    file: PathBuf,
}

impl Registry {
    /// `ANTISLOP_CONFIG_DIR`, else `$XDG_CONFIG_HOME/antislop`, else
    /// `~/.config/antislop`.
    pub fn from_env() -> Result<Self> {
        let nonempty = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        let dir = if let Some(dir) = nonempty("ANTISLOP_CONFIG_DIR") {
            PathBuf::from(dir)
        } else if let Some(dir) = nonempty("XDG_CONFIG_HOME") {
            PathBuf::from(dir).join("antislop")
        } else if let Some(home) = nonempty("HOME") {
            PathBuf::from(home).join(".config").join("antislop")
        } else {
            return Err(Error::Invalid(
                "no place for the list of trusted directories: set HOME or ANTISLOP_CONFIG_DIR"
                    .into(),
            ));
        };
        Ok(Self::at(&dir))
    }

    pub fn at(dir: &Path) -> Self {
        Self {
            file: dir.join("trusted.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.file
    }

    fn key(project: &Path) -> String {
        project
            .canonicalize()
            .unwrap_or_else(|_| project.to_path_buf())
            .to_string_lossy()
            .into_owned()
    }

    fn read(&self) -> Result<Stored> {
        match std::fs::read_to_string(&self.file) {
            Ok(text) => serde_json::from_str(&text).map_err(|error| {
                Error::Invalid(format!(
                    "the list of trusted directories {} is not valid: {error}",
                    self.file.display()
                ))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Stored::default()),
            Err(error) => Err(error.into()),
        }
    }

    /// An unreadable list trusts nothing.
    pub fn is_trusted(&self, project: &Path) -> bool {
        self.read()
            .is_ok_and(|stored| stored.trusted.contains(&Self::key(project)))
    }

    pub fn list(&self) -> Result<Vec<String>> {
        Ok(self.read()?.trusted)
    }

    fn write(&self, stored: &Stored) -> Result<()> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let dir = self.file.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        let temporary = self.file.with_extension("json.tmp");
        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(serde_json::to_string_pretty(stored)?.as_bytes())?;
            file.sync_all()?;
        }
        std::fs::rename(&temporary, &self.file)?;
        Ok(())
    }

    pub fn trust(&self, project: &Path) -> Result<()> {
        let mut stored = self.read()?;
        let key = Self::key(project);
        if !stored.trusted.contains(&key) {
            stored.trusted.push(key);
            self.write(&stored)?;
        }
        Ok(())
    }

    pub fn untrust(&self, project: &Path) -> Result<()> {
        let mut stored = self.read()?;
        let key = Self::key(project);
        let before = stored.trusted.len();
        stored.trusted.retain(|entry| *entry != key);
        if stored.trusted.len() != before {
            self.write(&stored)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_is_remembered_per_directory_and_private() {
        use std::os::unix::fs::PermissionsExt;
        let config = tempfile::tempdir().unwrap();
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let registry = Registry::at(&config.path().join("antislop"));
        assert!(!registry.is_trusted(a.path()));
        registry.trust(a.path()).unwrap();
        registry.trust(a.path()).unwrap();
        assert!(registry.is_trusted(a.path()));
        assert!(!registry.is_trusted(b.path()));
        assert_eq!(registry.list().unwrap().len(), 1);
        // A path through a symlink is the same directory.
        let link = config.path().join("link");
        std::os::unix::fs::symlink(a.path(), &link).unwrap();
        assert!(registry.is_trusted(&link));
        registry.untrust(a.path()).unwrap();
        assert!(!registry.is_trusted(a.path()));
        registry.trust(b.path()).unwrap();
        let mode = std::fs::metadata(registry.path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "only the user can read or write it");
    }

    #[test]
    fn a_damaged_list_trusts_nothing_and_is_not_overwritten() {
        let config = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let registry = Registry::at(config.path());
        std::fs::write(registry.path(), "not json").unwrap();
        assert!(!registry.is_trusted(dir.path()));
        let error = registry.trust(dir.path()).unwrap_err().to_string();
        assert!(error.contains("is not valid"), "{error}");
        assert_eq!(
            std::fs::read_to_string(registry.path()).unwrap(),
            "not json"
        );
    }
}
