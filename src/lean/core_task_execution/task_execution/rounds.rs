use super::super::super::*;
use super::{Execution, RoundState};
use super::{review, worker};

impl Execution<'_> {
    pub(super) fn run_round(&mut self, round: u32, state: &mut RoundState) -> Result<bool> {
        self.task.round = round;
        self.task.verdict = None;
        let recheck = self.prepare_round(state)?;
        let Some(report) = self.worker_or_stop(round, state, recheck)? else {
            return Ok(false);
        };
        let Some((report, summary, changes)) = self.finish_worker_report(round, report)? else {
            return Ok(false);
        };
        let Some(verdict) = self.review_round(round, state, &report, changes.as_deref())? else {
            return Ok(false);
        };
        self.settle_round(state, verdict, &report, changes.as_deref(), summary)
    }

    fn finish_worker_report(
        &mut self,
        round: u32,
        raw: String,
    ) -> Result<Option<(String, (String, bool), Option<String>)>> {
        let Some((report, summary)) = worker::complete_report(
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
            &report,
            &summary.0,
        )?;
        Ok(Some((report, summary, changes)))
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
            &self.verifier,
        )?;
        if !recheck && state.resumed && !state.dependents_invalidated {
            self.core
                .invalidate_task_dependents(self.conversation, self.task)?;
            state.dependents_invalidated = true;
        }
        if !recheck {
            self.task.report = None;
        }
        self.task.feedback = None;
        self.core.set_status(
            self.conversation,
            self.task,
            if recheck {
                status::VERIFYING
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
            project: &self.project,
            workdir: &self.workdir,
            add_dirs: &self.add_dirs,
            upstream: &self.upstream,
            verifier: &self.verifier,
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
        report: &str,
        changes: Option<&str>,
        summary: (String, bool),
    ) -> Result<bool> {
        let obligations = tasks::obligations(&self.spec);
        let incomplete = !tasks::unaccounted(&verdict, &obligations).is_empty();
        tasks::settle(&mut verdict, &obligations, !self.spec.writes);
        self.save_round_verdict(&verdict, report, changes, summary)?;
        if incomplete {
            return self.incomplete();
        }
        if verdict.approved {
            self.approve_round()?;
            return Ok(false);
        }
        self.update_progress(state, verdict)
    }

    fn save_round_verdict(
        &mut self,
        verdict: &Verdict,
        report: &str,
        changes: Option<&str>,
        summary: (String, bool),
    ) -> Result<()> {
        review::record_checks(self.core, self.conversation, self.task, &self.spec, verdict)?;
        review::save_verdict(
            self.core,
            self.conversation,
            self.task,
            &self.spec,
            verdict.clone(),
            report,
            changes,
            Some(summary),
            self.integration_repairs,
        )
    }

    fn incomplete(&mut self) -> Result<bool> {
        self.core.stop(
            self.conversation,
            self.task,
            status::NEEDS_USER,
            Some("the verifier's verdict still leaves obligations unaccounted for"),
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
            return self.stalled();
        }
        state.findings = verdict.findings;
        Ok(true)
    }

    fn stalled(&mut self) -> Result<bool> {
        let why = "no previously open obligation was resolved and the verifier confirmed none of its earlier findings fixed; the orchestrator must supply a new basis or revise the contract";
        let report = self.task.report.take().unwrap_or_default();
        self.task.report = Some(format!("{report}\n\n[runtime] {why}"));
        self.core.stop(
            self.conversation,
            self.task,
            status::COORDINATING,
            Some(why),
        )?;
        Ok(false)
    }
}
