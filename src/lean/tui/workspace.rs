//! The projects open in one screen. Each has its own daemon, state directory
//! and configuration, as when antislop is started in it alone; the screen
//! switches between them and shows what the others are doing.

use super::super::daemon::{Client, Request};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// One project the screen is connected to.
#[derive(Clone)]
pub struct ProjectConn {
    pub path: PathBuf,
    pub name: String,
    pub client: Client,
    /// Repository branch when this connection was opened, for sessions without start metadata.
    pub branch: Option<String>,
    /// The conversations as the daemon last listed them.
    pub conversations: Vec<Value>,
}

impl ProjectConn {
    pub fn new(path: PathBuf, client: Client) -> Self {
        Self {
            name: name_of(&path),
            branch: super::super::git::session_branch(&path).ok(),
            path,
            client,
            conversations: Vec::new(),
        }
    }

    /// How many conversations work, and how many wait for the user or have
    /// gone wrong.
    pub fn badges(&self) -> (usize, usize) {
        badges(&self.conversations)
    }
}

pub fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

pub fn badges(conversations: &[Value]) -> (usize, usize) {
    let count = |states: &[&str]| {
        conversations
            .iter()
            .filter(|c| states.contains(&c["status"].as_str().unwrap_or_default()))
            .count()
    };
    (
        count(&["working"]),
        count(&["needs_user", "waiting", "failed", "proposed"]),
    )
}

/// Opens a project: its own configuration and state directory, and the
/// daemon for them. Invalid configuration creates no runtime state.
pub fn connect(path: &Path, executable: &Path) -> Result<ProjectConn, String> {
    let fail = |error: &dyn std::fmt::Display| error.to_string();
    let path = path
        .canonicalize()
        .map_err(|e| fail(&format!("{}: {e}", path.display())))?;
    if !path.is_dir() {
        return Err(fail(&format!("{} is not a directory", path.display())));
    }
    let config = super::super::cli::resolve_config(&path, None).map_err(|e| fail(&e))?;
    crate::config::Config::load(&config).map_err(|e| fail(&e))?;
    let state = super::super::paths::project_state(&path).map_err(|e| fail(&e))?;
    super::super::daemon::private_directory(&state).map_err(|e| fail(&e))?;
    let state = state.canonicalize().map_err(|e| fail(&e))?;
    let client = Client::connect(&path, &config, &state, executable).map_err(|e| fail(&e))?;
    Ok(ProjectConn::new(path, client))
}

/// The project's latest conversation, else a new one.
pub fn latest_or_new(conn: &ProjectConn) -> Result<String, String> {
    let project = conn.path.to_string_lossy();
    let list = conn
        .client
        .call(&Request::Control(
            crate::lean::daemon::ControlRequest::Conversations,
        ))
        .map_err(|e| e.to_string())?;
    let latest = list
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["project"] == project.as_ref())
        .max_by_key(|c| c["created_ms"].as_i64())
        .and_then(|c| c["id"].as_str().map(str::to_owned));
    match latest {
        Some(id) => Ok(id),
        None => conn
            .client
            .call(&Request::Control(
                crate::lean::daemon::ControlRequest::StartIn {
                    project: conn.path.to_string_lossy().into_owned(),
                },
            ))
            .map_err(|e| e.to_string())
            .map(|c| c["id"].as_str().unwrap_or_default().to_owned()),
    }
}

/// What the header says of the other projects: `2 more ●1 !1`.
pub fn others_summary(projects: &[ProjectConn], active: usize) -> Option<String> {
    if projects.len() < 2 {
        return None;
    }
    let (mut working, mut attention) = (0, 0);
    for (index, project) in projects.iter().enumerate() {
        if index != active {
            let (w, a) = project.badges();
            working += w;
            attention += a;
        }
    }
    let mut text = format!("{} more", projects.len() - 1);
    if working > 0 {
        text.push_str(&format!(" ●{working}"));
    }
    if attention > 0 {
        text.push_str(&format!(" !{attention}"));
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn badges_count_what_works_and_what_needs_the_user() {
        let list = vec![
            json!({"status": "working"}),
            json!({"status": "working"}),
            json!({"status": "needs_user"}),
            json!({"status": "failed"}),
            json!({"status": "idle"}),
            json!({"status": "done"}),
        ];
        assert_eq!(badges(&list), (2, 2));
        assert_eq!(badges(&[]), (0, 0));
    }
}
