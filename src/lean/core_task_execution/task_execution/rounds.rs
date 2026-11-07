use super::super::super::*;
use super::{Execution, RoundState};
use super::{review, worker};

struct FinishedWorker {
    report: String,
    summary: (String, bool),
    changes: Option<String>,
}

impl Execution<'_> {
    pub(super) fn run_round(&mut self, round: u32, state: &mut RoundState) -> Result<bool> {
        self.task.round = round;
        self.task.verdict = None;
        let recheck = self.prepare_round(state)?;
        let Some(report) = self.worker_or_stop(round, state, recheck)? else {
            return Ok(false);
        };
        let Some(finished) = self.finish_worker_report(round, report)? else {
            return Ok(false);
        };
        let Some(verdict) =
            self.review_round(round, state, &finished.report, finished.changes.as_deref())?
        else {
            return Ok(false);
        };
        self.settle_round(state, verdict, finished)
    }

    fn finish_worker_report(&mut self, round: u32, raw: String) -> Result<Option<FinishedWorker>> {
        let Some(completed) = worker::complete_report(
            self.core,
            self.conversation,
            self.task,
            &self.spec,
            self.worktree.as_deref(),
            raw,
        )?
        else {
            return Ok(None);
        };
        let changes = worker::commit_round(
            self.task,
            &self.spec,
            self.worktree.as_deref(),
            round,
            &completed.report,
            &completed.effect.0,
            &completed.commit_message,
        )?;
        if let Some(path) = self.worktree.as_deref() {
            self.core
                .validate_artifact_delta(self.task, &self.spec, path)?;
        }
        if let Some(result) = &self.task.result {
            self.core.store.db().execute("INSERT INTO task_source_revisions(task_id,generation,revision) VALUES(?1,?2,?3) ON CONFLICT(task_id,generation) DO UPDATE SET revision=excluded.revision",rusqlite::params![self.task.id,self.task.input_generation,result])?;
        }
        Ok(Some(FinishedWorker {
            report: completed.report,
            summary: completed.effect,
            changes,
        }))
    }

    fn worker_or_stop(
        &mut self,
        round: u32,
        state: &RoundState,
        recheck: bool,
    ) -> Result<Option<String>> {
        let report = self.worker_report(round, state, recheck)?;
        if report.is_none() {
            self.core
                .stop_task(self.conversation, self.task, &self.worker, self.cancel)?;
        }
        Ok(report)
    }

    fn prepare_round(&mut self, state: &mut RoundState) -> Result<bool> {
        let recheck = worker::is_recheck(
            self.core,
            self.conversation,
            self.task,
            &self.worker,
            &self.reviewer,
        )?;
        if !recheck && state.resumed && !state.dependents_invalidated {
            self.core
                .invalidate_task_dependents(self.conversation, self.task)?;
            state.dependents_invalidated = true;
        }
        if !recheck {
            self.task.report = None;
        }
        self.task.feedback = if recheck {
            self.task
                .feedback
                .as_ref()
                .and_then(|f| f["carried_from"].as_str())
                .map(|source| json!({"round": self.task.round,"carried_from":source}))
        } else {
            None
        };
        self.core.set_status(
            self.conversation,
            self.task,
            if recheck {
                status::REVIEWING
            } else {
                status::WORKING
            },
        )?;
        Ok(recheck)
    }

    fn worker_report(
        &self,
        round: u32,
        state: &RoundState,
        recheck: bool,
    ) -> Result<Option<String>> {
        worker::run(worker::WorkerRound {
            core: self.core,
            conversation: self.conversation,
            task: self.task,
            cancel: self.cancel,
            spec: &self.spec,
            workdir: &self.workdir,
            project_text: &self.project_text,
            add_dirs: &self.add_dirs,
            upstream: &self.upstream,
            worker: &self.worker,
            round,
            state,
            recheck,
        })
    }

    fn review_round(
        &mut self,
        round: u32,
        state: &RoundState,
        report: &str,
        changes: Option<&str>,
    ) -> Result<Option<Verdict>> {
        review::run(review::ReviewRound {
            core: self.core,
            conversation: self.conversation,
            task: self.task,
            cancel: self.cancel,
            spec: &self.spec,
            workdir: &self.workdir,
            add_dirs: &self.add_dirs,
            upstream: &self.upstream,
            reviewer: &self.reviewer,
            round,
            findings: &state.findings,
            report,
            changes,
        })
    }

    fn settle_round(
        &mut self,
        state: &mut RoundState,
        mut verdict: Verdict,
        finished: FinishedWorker,
    ) -> Result<bool> {
        let obligations = tasks::obligations(&self.spec);
        let incomplete = !tasks::unaccounted(&verdict, &obligations).is_empty();
        tasks::settle(
            &mut verdict,
            &obligations,
            !self.spec.change_kind.has_delivery(),
        );
        self.save_round_verdict(&verdict, finished)?;
        if incomplete {
            return self.incomplete();
        }
        if verdict.approved {
            self.approve_round()?;
            return Ok(false);
        }
        self.update_progress(state, verdict)
    }

    fn save_round_verdict(&mut self, verdict: &Verdict, finished: FinishedWorker) -> Result<()> {
        review::record_checks(self.core, self.conversation, self.task, &self.spec, verdict)?;
        review::save_verdict(review::VerdictSave {
            core: self.core,
            task: self.task,
            spec: &self.spec,
            verdict: verdict.clone(),
            report: &finished.report,
            changes: finished.changes.as_deref(),
            summary_effect: Some(finished.summary),
            repairs: self.integration_repairs,
        })
    }

    fn incomplete(&mut self) -> Result<bool> {
        self.core.stop(
            self.conversation,
            self.task,
            status::NEEDS_USER,
            Some("the reviewer's verdict still leaves obligations unaccounted for"),
        )?;
        Ok(false)
    }

    fn approve_round(&mut self) -> Result<()> {
        if let Some(path) = &self.worktree {
            return self.core.integrate(
                self.conversation,
                self.task,
                &self.spec,
                &self.project,
                path,
                &self.config,
                self.cancel,
            );
        }
        let _integration = self.core.integrating.lock().expect("integration mutex");
        if self.core.defer_agent_work(self.conversation, self.task)? {
            return Ok(());
        }
        super::super::core_feedback_helpers::land(
            self.task,
            Outcome::NotApplicable,
            "run",
            "read-only task; nothing needs to land",
            None,
            None,
            &[],
        );
        self.core
            .set_status(self.conversation, self.task, status::APPROVED)
    }

    fn update_progress(&mut self, state: &mut RoundState, verdict: Verdict) -> Result<bool> {
        let fixed = super::super::progress::fixed_earlier(&state.findings, &verdict);
        let stalled = state
            .run
            .progress
            .record_round(super::super::progress::open_obligations(&verdict), fixed);
        state.run.rejections += 1;
        state.run.findings = verdict.findings.clone();
        self.core.store.record_task_progress(
            self.conversation,
            self.task,
            Some(&serde_json::to_value(&state.run)?),
        )?;
        if stalled {
            self.core.notice(self.conversation, "Review found no resolved obligation in this round; the worker continues with its findings and must change its approach where needed.");
        }
        state.findings = verdict.findings;
        Ok(true)
    }
}
