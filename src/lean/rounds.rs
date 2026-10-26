//! Durable round boundaries and the explicit context carried across them.
use super::*;
use rusqlite::{OptionalExtension, params};

#[path = "rounds/handoff.rs"]
mod handoff;

#[path = "rounds/session.rs"]
mod session;

#[cfg(test)]
#[path = "rounds/tests.rs"]
mod tests;

#[derive(Debug)]
pub(crate) struct RoundHandoff {
    pub task: tasks::TaskContract,
    pub round: u32,
    pub worktree: PathBuf,
    pub report: Option<(u32, String)>,
    pub verdict: Option<serde_json::Value>,
    pub files: Vec<String>,
    pub base: Option<String>,
    pub result: Option<String>,
    pub interrupted: bool,
    pub interruptions: Vec<RoundInterruption>,
    pub dependencies: Vec<tasks::DependencyReport>,
    pub requirements: Vec<ledger::LedgerEntry>,
    pub comments: Vec<Comment>,
    pub protected_paths: Vec<String>,
    pub primary: PathBuf,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct RoundInterruption {
    pub round: u32,
    pub role: String,
    pub status: String,
    pub reason: Option<String>,
}

/// Hash only provider execution settings, never repository contents or secrets in logs.
/// Keeping this in turn metadata avoids a second session archive or schema change.
pub(super) fn execution_signature(profile: &ResolvedProfile) -> Result<String> {
    execution_signature_fields(
        &profile.profile,
        profile.provider,
        &profile.binary,
        &profile.args,
        profile.model.as_deref(),
        profile.native_effort.as_deref(),
        profile.effort,
    )
}

pub(super) fn spec_signature(
    profile: &str,
    effort: crate::config::Effort,
    spec: &agent::Spec,
) -> Result<String> {
    execution_signature_fields(
        profile,
        spec.provider,
        &spec.binary,
        &spec.args,
        spec.model.as_deref(),
        spec.effort.as_deref(),
        effort,
    )
}

fn execution_signature_fields(
    profile: &str,
    provider: crate::config::ProviderKind,
    binary: &Path,
    args: &[String],
    model: Option<&str>,
    effort: Option<&str>,
    configured_effort: crate::config::Effort,
) -> Result<String> {
    use sha2::Digest;
    let encoded = serde_json::to_vec(&json!({"profile":profile,"provider":provider,
        "binary":binary,"args":args,"model":model,"effort":effort,"configured_effort":configured_effort}))?;
    Ok(sha2::Sha256::digest(encoded)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

impl Core {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn session_for_turn(
        &self,
        conversation: &str,
        role: &str,
        provider: &str,
        policy: &SessionPolicy,
        profile: Option<&ResolvedProfile>,
        cwd: &Path,
        add_dirs: &[PathBuf],
        access: agent::Access,
    ) -> Result<Option<String>> {
        let SessionPolicy::TaskRound { task, round } = policy else {
            return self.store.last_session(conversation, role, provider);
        };
        self.matching_round_session(
            conversation,
            role,
            provider,
            task,
            *round,
            profile,
            cwd,
            add_dirs,
            access,
        )
    }

    pub(super) fn round_handoff(&self, id: &str, round: u32, cwd: &Path) -> Result<RoundHandoff> {
        let task = self
            .store
            .task_by_id(id)?
            .ok_or_else(|| Error::Invalid(format!("task {id} does not exist")))?;
        let spec: tasks::TaskContract = serde_json::from_value(task.spec.clone())?;
        let mut handoff = self.new_handoff(id, round, cwd, &task, &spec)?;
        self.add_handoff_requirements(&task, &spec, &mut handoff)?;
        self.add_handoff_comments(&task, &mut handoff)?;
        self.add_protected_paths(&task, &spec, &mut handoff)?;
        handoff.interruptions = self.round_interruptions(id, round)?;
        self.recover_previous_rounds(id, round, cwd, &mut handoff)?;
        Ok(handoff)
    }
}
