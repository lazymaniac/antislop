use crate::lean::core::task::feedback_helpers::{event_files, land, note};
use crate::lean::core::worktree::paths::worktree_branch;
use crate::lean::*;

mod actions;
mod events;
mod integrate;
mod manual;
mod orchestrator_retry;
mod prepare;
mod response;
#[cfg(test)]
mod test_support;

pub(in crate::lean) struct RecoveryWork {
    pub(in crate::lean) req: TaskRecoveryRequest,
    pub(in crate::lean) actor: String,
    pub(in crate::lean) project: PathBuf,
    pub(in crate::lean) config: Arc<Config>,
    pub(in crate::lean) spec: TaskContract,
    pub(in crate::lean) task: Task,
    pub(in crate::lean) token: String,
    pub(in crate::lean) integrated_files: Vec<String>,
    pub(in crate::lean) preserved_branch: Option<String>,
    pub(in crate::lean) notice: Option<String>,
    pub(in crate::lean) cancel: Arc<AtomicBool>,
}

impl Core {
    pub(in crate::lean) fn recover_task_as(
        self: &Arc<Self>,
        req: TaskRecoveryRequest,
        actor: &str,
    ) -> Result<TaskRecoveryResponse> {
        let operation = self.operation(&req.conversation)?;
        if matches!(
            req.action,
            RecoveryAction::CancelTask | RecoveryAction::RestartAgent
        ) {
            // Validate the caller's token before signalling, without waiting on
            // an integration/manual-review source scope.
            let mut preview = prepare::load(self, req.clone(), actor)?;
            let _graph = self.locks.graph();
            self.store.assert_task_inputs(&preview.task)?;
            if let Some(response) = prepare::interrupt_if_active(self, &mut preview) {
                self.cancel_task_operations(&req.conversation, &preview.task.id);
                return Ok(response);
            }
            let operations = self.activity.operations.lock().expect("operation leases");
            let matching = operations
                .values()
                .filter(|op| {
                    op.conversation == req.conversation
                        && op.task.as_deref() == Some(&preview.task.id)
                })
                .collect::<Vec<_>>();
            if !matching.is_empty() {
                require(
                    req.action == RecoveryAction::CancelTask,
                    "manual recovery is active; cancel the task before restarting its agent",
                )?;
                for op in matching {
                    op.cancel.store(true, Ordering::SeqCst);
                }
                preview.task = self
                    .store
                    .task_by_id(&preview.task.id)?
                    .ok_or_else(|| Error::Invalid("recovery task disappeared".into()))?;
                self.stop(
                    &req.conversation,
                    &mut preview.task,
                    status::CANCELLED,
                    Some("manual recovery cancellation requested"),
                )?;
                let mut response = response::interrupted(&preview);
                response.notice = "cancelling the active recovery operation".into();
                return Ok(response);
            }
        }
        let source = self.locks.source(&req.conversation);
        let _source = mail::lock_cancellable(&source, &operation.cancel)
            .ok_or_else(|| Error::Invalid("recovery cancelled".into()))?;
        require(
            !operation.cancel.load(Ordering::SeqCst),
            "recovery cancelled",
        )?;
        let mut work = prepare::load(self, req, actor)?;
        work.cancel = operation.cancel.clone();
        operation.bind_task(&work.task.id);
        {
            let _graph = self.locks.graph();
            let _integration = self.locks.integration();
            self.store.assert_task_inputs(&work.task)?;
            if let Some(response) = prepare::interrupt_if_active(self, &mut work) {
                return Ok(response);
            }
        }
        let primary = (work.req.action == RecoveryAction::ReviewManualSnapshot)
            .then(|| self.locks.primary(&work.project))
            .transpose()?;
        let _primary = if let Some(lock) = primary.as_ref() {
            Some(
                mail::lock_cancellable(lock, &operation.cancel)
                    .ok_or_else(|| Error::Invalid("recovery cancelled".into()))?,
            )
        } else {
            None
        };
        if let Err(error) = actions::apply(self, &mut work) {
            if work.cancel.load(Ordering::SeqCst) {
                if let Some(mut current) = self.store.task_by_id(&work.task.id)?
                    && current.status != status::CANCELLED
                {
                    self.stop(
                        &work.req.conversation,
                        &mut current,
                        status::CANCELLED,
                        Some(&error.to_string()),
                    )?;
                }
                return Err(error);
            }
            if !matches!(
                work.req.action,
                RecoveryAction::RestartAgent | RecoveryAction::RetryWithGuidance
            ) {
                return Err(error);
            }
            let reason = error.to_string();
            // Reload: an error may have happened after a partial state change.
            let mut task = self.task(&work.req.plan, &work.task.id)?;
            self.stop(
                &work.req.conversation,
                &mut task,
                status::FAILED,
                Some(&reason),
            )?;
            self.store.append_event(
                &work.req.conversation,
                None,
                Some(&work.req.plan),
                "task_recovery_failed",
                &json!({"task": task.id, "action": work.req.action, "actor": actor, "reason": reason}),
            )?;
            return Err(error);
        }
        response::build(self, work)
    }
}

fn ensure_current(core: &Core, work: &RecoveryWork) -> Result<()> {
    core.store.assert_task_inputs(&work.task)?;
    require(
        !work.cancel.load(Ordering::SeqCst)
            && core.store.task_by_id(&work.task.id)?.is_some_and(|task| {
                !matches!(
                    task.status.as_str(),
                    status::CANCELLED | status::STALE | "superseded"
                )
            }),
        "recovery cancelled or superseded",
    )
}

#[cfg(test)]
mod artifact_recovery_tests {
    use super::*;

    #[test]
    fn consented_artifact_recovery_prepares_without_recursively_locking_integration() {
        let (directory, core, conversation, mut task, _turn) = test_support::recovery_fixture();
        let project = directory.path();
        git::ok(project, &["init", "-q", "-b", "main"]).unwrap();
        std::fs::write(project.join("source.txt"), "base").unwrap();
        let base = git::commit_all(project, git::CommitPurpose::TestFixture).unwrap();
        let writer_root = tempfile::tempdir().unwrap();
        let writer = writer_root.path().join("writer");
        git::add_worktree(project, &writer, "artifact-recovery-writer", &base).unwrap();
        std::fs::write(writer.join("result.md"), "reviewed artifact").unwrap();
        let change = git::commit_artifacts(
            &writer,
            &["result.md".into()],
            git::CommitPurpose::TestFixture,
            true,
        )
        .unwrap();
        task.spec["change_kind"] = json!("artifact");
        task.spec["delivery"] = json!("untracked");
        task.spec["outputs"] = json!(["result.md"]);
        task.base = Some(base);
        task.result = Some(change);
        task.status = status::NEEDS_USER.into();
        task.verdict = Some(json!({"approved":true,"findings":[]}));
        core.store
            .db()
            .execute(
                "UPDATE tasks SET spec=?2 WHERE id=?1",
                rusqlite::params![task.id, task.spec.to_string()],
            )
            .unwrap();
        core.store.update_task(&task).unwrap();
        let token = core
            .task_freshness_token(&conversation, &task.plan_id, &task.id)
            .unwrap();
        let req = TaskRecoveryRequest {
            conversation: conversation.clone(),
            plan: task.plan_id.clone(),
            task: task.id.clone(),
            action: RecoveryAction::IntegrateWithConsent,
            consent_files: Some(vec!["result.md".into()]),
            freshness_token: token,
            message: None,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = core.clone();
        std::thread::spawn(move || {
            let _ = tx.send(worker.recover_task(req));
        });
        let response = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("artifact recovery blocked on a shared scope lock")
            .unwrap();
        assert_eq!(response.status, status::APPROVED);
        assert_eq!(
            std::fs::read_to_string(project.join("result.md")).unwrap(),
            "reviewed artifact"
        );
    }
}
