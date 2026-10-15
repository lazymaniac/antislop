//! Explicit mode transitions preserve the configuration that authored old work.
//! Only mode and its revision may differ; permissions and routing stay pinned.
use crate::{Result, config::*, domain::Run, hash, require};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModeChange {
    pub spec_revision: u64,
    pub previous: PinnedConfig,
    pub mode: Mode,
    pub actor: String,
}

impl Run {
    pub(crate) fn mode_configurations(&self) -> Result<Vec<&PinnedConfig>> {
        require(
            self.mode_changes.iter().all(|change| {
                change.spec_revision > 0 && change.spec_revision <= self.spec_revision
            }),
            "mode history has a foreign specification revision",
        )?;
        let Some(current) = self.spec.config.as_ref() else {
            require(
                self.mode_changes
                    .iter()
                    .all(|change| change.spec_revision != self.spec_revision),
                "mode history requires a configuration",
            )?;
            return Ok(vec![]);
        };
        current.validate()?;
        let mut configurations = vec![current];
        let mut expected = None;
        for change in &self.mode_changes {
            if change.spec_revision != self.spec_revision {
                continue;
            }
            change.previous.validate()?;
            require(
                change.actor == "local_user"
                    && change.mode != change.previous.config.policy.mode
                    && expected
                        .as_ref()
                        .is_none_or(|hash| *hash == change.previous.content_hash),
                "mode history has an unauthenticated or broken transition",
            )?;
            let mut next = change.previous.config.clone();
            next.policy.mode = change.mode;
            next.policy.revision = next
                .policy
                .revision
                .checked_add(1)
                .ok_or_else(|| crate::Error::Invalid("policy revision overflow".into()))?;
            expected = Some(next.pin()?.content_hash);
            configurations.push(&change.previous);
        }
        require(
            expected.is_none_or(|hash| hash == current.content_hash),
            "mode history does not end at the current pinned configuration",
        )?;
        Ok(configurations)
    }

    /// Historical evidence is reusable only across recorded mode-only changes.
    /// A new effect always uses the current configuration and its new scope.
    pub(crate) fn mode_configuration_matches(&self, digest: &str) -> bool {
        if self.mode_changes.is_empty() {
            return self.spec.config.as_ref().map_or_else(
                || digest == hash(serde_json::to_vec(&self.spec).expect("specification")),
                |config| config.content_hash == digest,
            );
        }
        self.mode_configurations().is_ok_and(|configs| {
            if configs.is_empty() {
                digest == hash(serde_json::to_vec(&self.spec).expect("specification"))
            } else {
                configs.iter().any(|config| config.content_hash == digest)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::RunSpec,
        id,
        runtime::{Command, Runtime},
        sessions::SessionScope,
        storage::Store,
    };
    use std::sync::Arc;

    fn fixture(check: bool) -> (tempfile::TempDir, Arc<Runtime>, Run) {
        let root = tempfile::tempdir().unwrap();
        let mut spec: RunSpec =
            serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
        spec.workspace = root.path().to_string_lossy().into();
        spec.config = Some(
            Config::from_toml(include_str!("../examples/architect.toml"))
                .unwrap()
                .pin()
                .unwrap(),
        );
        if check {
            spec.check_command = vec!["true".into()];
        }
        let runtime =
            Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
        let run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();
        (root, runtime, run)
    }

    fn switch(runtime: &Arc<Runtime>, run: &Run, mode: Mode) -> Result<Run> {
        runtime.dispatch(
            &id(),
            Command::ChangeMode {
                run_id: run.id.clone(),
                mode,
                expected_policy_revision: run.spec.config.as_ref().unwrap().config.policy.revision,
            },
        )
    }

    #[test]
    fn recorded_mode_changes_keep_old_authorship_without_rewriting_new_scopes() {
        let (_root, runtime, original) = fixture(false);
        assert!(
            !serde_json::to_value(&original)
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("mode_changes")
        );
        let scope = SessionScope::for_run(
            &original,
            "worker".into(),
            Some("task".into()),
            RoleKind::Executor,
            "codex".into(),
            "fixture".into(),
        )
        .unwrap();
        let first = switch(&runtime, &original, Mode::Autonomous).unwrap();
        let current = switch(&runtime, &first, Mode::Supervised).unwrap();
        let new = SessionScope::for_run(
            &current,
            "worker".into(),
            Some("task".into()),
            RoleKind::Executor,
            "codex".into(),
            "fixture".into(),
        )
        .unwrap();
        assert_eq!(new.policy_revision, scope.policy_revision + 2);
        assert_ne!(new.configuration_hash, scope.configuration_hash);
        assert_eq!(original.source_hashes(), current.source_hashes());
        assert_eq!(original.spec_revision, current.spec_revision);
        assert!(
            scope
                .matches_run(
                    &current,
                    "worker".into(),
                    Some("task".into()),
                    RoleKind::Executor
                )
                .unwrap()
        );
        assert!(
            !scope
                .matches_run(
                    &current,
                    "foreign".into(),
                    Some("task".into()),
                    RoleKind::Executor
                )
                .unwrap()
        );
        assert!(
            !scope
                .matches_run(
                    &current,
                    "worker".into(),
                    Some("other".into()),
                    RoleKind::Executor
                )
                .unwrap()
        );
        let mut wrong = scope.clone();
        wrong.policy_revision += 1;
        assert!(
            !wrong
                .matches_run(
                    &current,
                    "worker".into(),
                    Some("task".into()),
                    RoleKind::Executor
                )
                .unwrap()
        );
        for kind in 0..4 {
            let mut forged = current.clone();
            match kind {
                0 => forged.mode_changes[0].actor = "model".into(),
                1 => {
                    forged.mode_changes[0].previous.config.policy.local_write = false;
                    forged.mode_changes[0].previous = forged.mode_changes[0]
                        .previous
                        .config
                        .clone()
                        .pin()
                        .unwrap();
                }
                2 => {
                    forged.mode_changes.remove(1);
                }
                _ => {
                    forged.mode_changes.reverse();
                }
            }
            assert!(forged.mode_configurations().is_err());
            assert!(!forged.mode_configuration_matches(&scope.configuration_hash));
            assert!(
                scope
                    .matches_run(
                        &forged,
                        "worker".into(),
                        Some("task".into()),
                        RoleKind::Executor
                    )
                    .is_err()
            );
        }
        assert!(
            runtime
                .store
                .lock()
                .unwrap()
                .events(0)
                .unwrap()
                .iter()
                .filter(|e| e.kind == "policy_mode_changed")
                .count()
                == 2
        );
        let reopened = Store::open(
            &runtime
                .store
                .lock()
                .unwrap()
                .data_directory()
                .unwrap()
                .join("state.db"),
        )
        .unwrap()
        .get(&current.id)
        .unwrap();
        assert!(
            scope
                .matches_run(
                    &reopened,
                    "worker".into(),
                    Some("task".into()),
                    RoleKind::Executor
                )
                .unwrap()
        );
        let mut revised = reopened;
        revised.spec_revision += 1;
        revised.spec.config = None;
        assert!(revised.mode_configurations().unwrap().is_empty());
        assert!(
            !scope
                .matches_run(
                    &revised,
                    "worker".into(),
                    Some("task".into()),
                    RoleKind::Executor
                )
                .unwrap()
        );
    }

    #[test]
    fn mode_switch_rebinds_a_preparatory_team_without_reassigning_tasks_or_permissions() {
        let (_root, runtime, mut run) = fixture(false);
        let mut team = crate::team::TeamPlan::bootstrap(run.spec.task_family, &run.spec).unwrap();
        team.validate_all(&run.spec, &run.spec.config.as_ref().unwrap().config)
            .unwrap();
        // Structural planning fixture only; no native acceptance or task result.
        run.team = Some(team);
        runtime
            .store
            .lock()
            .unwrap()
            .save(&run, "synthetic_preparatory_team", None)
            .unwrap();
        let tasks = serde_json::to_value(&run.team.as_ref().unwrap().tasks).unwrap();
        let agents = serde_json::to_value(&run.team.as_ref().unwrap().agents).unwrap();
        let scope = run.team_step_scope().unwrap();
        let changed = switch(&runtime, &run, Mode::Autonomous).unwrap();
        changed
            .team
            .as_ref()
            .unwrap()
            .validate(&changed.spec)
            .unwrap();
        assert_eq!(
            serde_json::to_value(&changed.team.as_ref().unwrap().tasks).unwrap(),
            tasks
        );
        assert_eq!(
            serde_json::to_value(&changed.team.as_ref().unwrap().agents).unwrap(),
            agents
        );
        assert_ne!(scope, changed.team_step_scope().unwrap());
        let mut normalized = changed.spec.config.as_ref().unwrap().config.clone();
        normalized.policy.mode = run.spec.config.as_ref().unwrap().config.policy.mode;
        normalized.policy.revision = run.spec.config.as_ref().unwrap().config.policy.revision;
        assert_eq!(normalized, run.spec.config.as_ref().unwrap().config);
    }

    #[test]
    fn mode_switch_refuses_unsupported_runner_and_failed_commit_without_mutating_state() {
        let (_root, runtime, original) = fixture(true);
        let error = switch(&runtime, &original, Mode::Autonomous)
            .unwrap_err()
            .to_string();
        assert!(error.contains("restricted runner"), "{error}");
        assert_eq!(
            serde_json::to_value(runtime.store.lock().unwrap().get(&original.id).unwrap()).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
        let (root, runtime, original) = fixture(false);
        let db = rusqlite::Connection::open(root.path().join("state.db")).unwrap();
        db.execute_batch("CREATE TRIGGER refuse_mode BEFORE INSERT ON events WHEN NEW.kind='policy_mode_changed' BEGIN SELECT RAISE(ABORT,'injected mode checkpoint failure'); END;").unwrap();
        assert!(switch(&runtime, &original, Mode::Autonomous).is_err());
        let reopened = Store::open(&root.path().join("state.db"))
            .unwrap()
            .get(&original.id)
            .unwrap();
        assert_eq!(
            serde_json::to_value(&reopened).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
        assert_eq!(
            serde_json::to_value(runtime.store.lock().unwrap().get(&original.id).unwrap()).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
    }
}
