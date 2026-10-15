//! Admission of new effects respects durable pause/cancel state. Already
//! admitted work may finish; a paused continuation retains its workspace lease.
use super::Runtime;
use crate::{
    Result,
    domain::{Run, Status},
    require,
    storage::Store,
};
use std::sync::MutexGuard;

impl Runtime {
    /// Waiting releases the store mutex, so control commands and other runs
    /// remain responsive. The same mutex makes resume/cancel wakeups lossless.
    pub(super) fn active_store(&self, captured: &Run) -> Result<MutexGuard<'_, Store>> {
        let mut store = self.store.lock().expect("store mutex");
        let mut waiting_recorded = false;
        loop {
            require(
                !self
                    .workers
                    .lock()
                    .expect("workers mutex")
                    .get(&captured.id)
                    .is_some_and(|token| token.load(std::sync::atomic::Ordering::SeqCst)),
                "attempt is stopping after cancellation or checkpoint failure",
            )?;
            let current = store.get(&captured.id)?;
            require(
                current.spec_revision == captured.spec_revision
                    && current.attempt == captured.attempt,
                "attempt is no longer active",
            )?;
            if current.status == Status::Paused && current.paused_from == Some(Status::Running) {
                if !waiting_recorded {
                    store.save(&current, "work_waiting_paused", None)?;
                    waiting_recorded = true;
                }
                store = self.work_changed.wait(store).expect("store mutex");
                continue;
            }
            require(
                current.status == Status::Running,
                "attempt cancelled or no longer running",
            )?;
            return Ok(store);
        }
    }
    pub(super) fn await_action(&self, captured: &Run) -> Result<()> {
        drop(self.active_store(captured)?);
        require(
            crate::workspace::inputs_current(captured)?,
            "workspace inputs changed while waiting; reconcile before continuing",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::*,
        hash, id,
        providers::{fake::FakeProvider, *},
        runtime::{Command, ObservedWork},
        verification,
    };
    use std::{
        sync::{Arc, atomic::AtomicBool, mpsc},
        time::{Duration, Instant},
    };

    struct PauseAfterExecutor(Arc<Runtime>);
    impl Provider for PauseAfterExecutor {
        fn probe(&self) -> Result<Capabilities> {
            FakeProvider.probe()
        }
        fn execute_observed(
            &self,
            invocation: &ObservedInvocation<'_>,
            token: Arc<AtomicBool>,
            events: &mut dyn FnMut(&ProviderEvent) -> Result<()>,
        ) -> Result<TurnReceipt> {
            let receipt = FakeProvider.execute_observed(invocation, token, events)?;
            // The current native turn completes after Pause; its successor
            // must not be admitted until an explicit Resume.
            self.0.dispatch(
                &id(),
                Command::Pause {
                    run_id: invocation.run.id.clone(),
                },
            )?;
            Ok(receipt)
        }
        fn execute(
            &self,
            run: &Run,
            token: Arc<AtomicBool>,
            session: &mut dyn FnMut(&str) -> Result<()>,
        ) -> Result<(String, String)> {
            FakeProvider.execute(run, token, session)
        }
        fn verify(
            &self,
            run: &Run,
            artifact: &Artifact,
            evidence: &Evidence,
            token: Arc<AtomicBool>,
            session: &mut dyn FnMut(&str) -> Result<()>,
        ) -> Result<Review> {
            FakeProvider.verify(run, artifact, evidence, token, session)
        }
    }
    struct CancelOnDrop(Arc<Runtime>, String);
    impl Drop for CancelOnDrop {
        fn drop(&mut self) {
            let _ = self.0.dispatch(
                &id(),
                Command::Cancel {
                    run_id: self.1.clone(),
                },
            );
        }
    }
    #[test]
    fn admitted_executor_completion_survives_pause_while_verifier_reservation_waits() {
        let root = tempfile::tempdir().unwrap();
        let runtime =
            Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
        let mut spec: RunSpec =
            serde_json::from_str(include_str!("../../examples/fake-spec.json")).unwrap();
        spec.workspace = root.path().to_string_lossy().into();
        let mut run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();
        run.status = Status::Running;
        run.attempt = 1;
        runtime
            .store
            .lock()
            .unwrap()
            .save(&run, "test_admitted_attempt", None)
            .unwrap();
        let _cleanup = CancelOnDrop(runtime.clone(), run.id.clone());
        let run_id = run.id.clone();
        let worker_runtime = runtime.clone();
        let (sent, received) = mpsc::channel();
        let worker = std::thread::spawn(move || -> Result<Run> {
            let view = run.clone();
            let token = Arc::new(AtomicBool::new(false));
            let (logical, receipt) = worker_runtime.observed_turn(
                &mut run,
                &view,
                &PauseAfterExecutor(worker_runtime.clone()),
                ObservedWork::Execute,
                token.clone(),
            )?;
            let artifact = Artifact {
                id: id(),
                stage: run.stage,
                attempt: run.attempt,
                spec_revision: run.spec_revision,
                session_id: logical,
                hash: hash(&receipt.output),
                content: receipt.output,
            };
            let evidence = verification::observe(&artifact);
            run.artifacts.push(artifact.clone());
            run.evidence.push(evidence.clone());
            worker_runtime.persist_progress(&run)?;
            sent.send(artifact.content.clone()).unwrap();
            let view = run.clone();
            worker_runtime.observed_turn(
                &mut run,
                &view,
                &FakeProvider,
                ObservedWork::Verify {
                    artifact: &artifact,
                    evidence: &evidence,
                },
                token,
            )?;
            Ok(run)
        });
        let executor_output = received.recv_timeout(Duration::from_secs(10)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let waiting = runtime
                .store
                .lock()
                .unwrap()
                .events(0)
                .unwrap()
                .iter()
                .any(|e| e.kind == "work_waiting_paused");
            if waiting {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let paused = runtime.store.lock().unwrap().get(&run_id).unwrap();
        assert_eq!(paused.status, Status::Paused);
        assert_eq!(paused.session_ledger.sessions.len(), 1);
        assert!(
            paused
                .session_ledger
                .turns
                .values()
                .all(|t| t.state == crate::sessions::TurnState::Completed)
        );
        assert_eq!(paused.artifacts[0].content, executor_output);
        runtime
            .dispatch(
                &id(),
                Command::Resume {
                    run_id: run_id.clone(),
                },
            )
            .unwrap();
        let finished = worker.join().unwrap().unwrap();
        assert_eq!(finished.attempt, 1);
        assert_eq!(finished.session_ledger.sessions.len(), 2);
        assert_eq!(finished.session_ledger.turn_order.len(), 2);
        assert_eq!(finished.artifacts[0].content, executor_output);
        assert_eq!(
            finished
                .session_ledger
                .sessions
                .values()
                .filter(|s| s.scope.role == crate::config::RoleKind::Verifier)
                .count(),
            1
        );
    }
}
