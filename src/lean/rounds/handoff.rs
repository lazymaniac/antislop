use super::*;

struct HandoffSeed {
    report: Option<(u32, String)>,
    primary: PathBuf,
}

impl Core {
    pub(super) fn new_handoff(
        &self,
        id: &str,
        round: u32,
        cwd: &Path,
        task: &Task,
    ) -> Result<RoundHandoff> {
        let seed = self.handoff_seed(id, round, task)?;
        Ok(base_handoff(task, round, cwd, seed))
    }

    fn handoff_seed(&self, id: &str, round: u32, task: &Task) -> Result<HandoffSeed> {
        let report = self.store.completed_worker_report(id, round)?;
        let primary = self.primary_project(&task.plan_id)?;
        Ok(HandoffSeed { report, primary })
    }

    fn primary_project(&self, plan: &str) -> Result<PathBuf> {
        let conversation = self.store.plan_conversation(plan)?;
        Ok(PathBuf::from(
            self.store.conversation(&conversation)?.project,
        ))
    }

    pub(super) fn add_handoff_requirements(
        &self,
        task: &Task,
        spec: &tasks::TaskContract,
        handoff: &mut RoundHandoff,
    ) -> Result<()> {
        let conversation = self.store.plan_conversation(&task.plan_id)?;
        if let Some(run) = self.store.plan_run(&task.plan_id)? {
            handoff.requirements = self
                .store
                .ledger_entries(&conversation, &run, None)?
                .into_iter()
                .filter(|entry| spec.requirement_ids.contains(&entry.spec.id))
                .map(|mut entry| {
                    entry
                        .spec
                        .criteria
                        .retain(|criterion| spec.criterion_ids.contains(&criterion.id));
                    entry
                })
                .collect();
        }
        Ok(())
    }

    pub(super) fn add_handoff_comments(
        &self,
        task: &Task,
        handoff: &mut RoundHandoff,
    ) -> Result<()> {
        let conversation = self.store.plan_conversation(&task.plan_id)?;
        handoff.comments = self
            .store
            .comments(&conversation, "open")?
            .into_iter()
            .filter(|comment| {
                (comment.target == spec_target(task)
                    || comment
                        .target
                        .starts_with(&format!("{}/", spec_target(task))))
                    && comment
                        .plan_id
                        .as_deref()
                        .is_none_or(|plan| plan == task.plan_id)
            })
            .collect();
        Ok(())
    }

    pub(super) fn add_protected_paths(
        &self,
        _task: &Task,
        spec: &tasks::TaskContract,
        handoff: &mut RoundHandoff,
    ) -> Result<()> {
        if !spec.writes {
            return Ok(());
        }
        handoff.protected_paths = git::uncommitted(&handoff.primary)?
            .into_iter()
            .filter(|path| {
                spec.scope.is_empty()
                    || spec
                        .scope
                        .iter()
                        .any(|scope| scope == "." || Path::new(path).starts_with(scope))
            })
            .collect();
        handoff.protected_paths.sort();
        handoff.protected_paths.dedup();
        Ok(())
    }

    pub(super) fn round_interruptions(
        &self,
        id: &str,
        round: u32,
    ) -> Result<Vec<RoundInterruption>> {
        let db = self.store.db();
        let mut query = db.prepare(
            "SELECT s.round,t.role,t.status,t.error FROM turns t JOIN turn_scopes s ON s.turn_id=t.id
             WHERE s.task_id=?1 AND s.round=(SELECT max(round) FROM turn_scopes WHERE task_id=?1 AND round<?2)
             AND t.role IN ('worker:'||?1,'verifier:'||?1) AND t.status IN ('failed','interrupted','cancelled')
             AND t.rowid=(SELECT max(n.rowid) FROM turns n JOIN turn_scopes ns ON ns.turn_id=n.id WHERE ns.task_id=s.task_id AND ns.round=s.round AND n.role=t.role)
             ORDER BY t.role",
        )?;
        Ok(query
            .query_map(params![id, round], |row| {
                Ok(RoundInterruption {
                    round: row.get(0)?,
                    role: row.get(1)?,
                    status: row.get(2)?,
                    reason: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?)
    }

    pub(super) fn recover_previous_rounds(
        &self,
        id: &str,
        round: u32,
        cwd: &Path,
        handoff: &mut RoundHandoff,
    ) -> Result<()> {
        let mut files = std::collections::BTreeSet::new();
        for (source, record) in self.store.handoff_rounds(id, round)? {
            self.merge_round_result(source, round, cwd, &record, handoff, &mut files)?;
            self.merge_round_report(source, round, &record, handoff)?;
        }
        handoff.files = files.into_iter().collect();
        Ok(())
    }

    fn merge_round_result(
        &self,
        source: u32,
        round: u32,
        cwd: &Path,
        record: &serde_json::Value,
        handoff: &mut RoundHandoff,
        files: &mut std::collections::BTreeSet<String>,
    ) -> Result<()> {
        if handoff.result.is_none() {
            handoff.result = record["result"].as_str().map(str::to_owned);
        }
        if source < round {
            collect_previous_changes(self, source, cwd, record, handoff, files)?;
        }
        Ok(())
    }

    fn merge_round_report(
        &self,
        source: u32,
        round: u32,
        record: &serde_json::Value,
        handoff: &mut RoundHandoff,
    ) -> Result<()> {
        let incomplete = record["worker_started"] == true && record["worker_completed"] != true;
        if should_mark_interrupted(source, round, incomplete, record, handoff) {
            handoff.interrupted = true;
        }
        if !incomplete && handoff.report.is_none() && !record["report"].is_null() {
            let report = self.store.handoff_field(&record["report"], false)?;
            if let Some(report) = report.as_str() {
                handoff.report = Some((source, report.to_owned()));
            }
        }
        Ok(())
    }
}

fn base_handoff(task: &Task, round: u32, cwd: &Path, seed: HandoffSeed) -> RoundHandoff {
    RoundHandoff {
        round,
        writes: task.spec["writes"].as_bool().unwrap_or(false),
        worktree: cwd.into(),
        report: seed.report,
        verdict: task.verdict.clone(),
        files: vec![],
        base: task.base.clone(),
        result: task.result.clone(),
        interrupted: false,
        interruptions: vec![],
        requirements: vec![],
        comments: vec![],
        protected_paths: vec![],
        primary: seed.primary,
    }
}

fn collect_previous_changes(
    core: &Core,
    source: u32,
    cwd: &Path,
    record: &serde_json::Value,
    handoff: &mut RoundHandoff,
    files: &mut std::collections::BTreeSet<String>,
) -> Result<()> {
    if let (Some(base), Some(result)) = (record["base"].as_str(), record["result"].as_str()) {
        files.extend(git::changed_files(cwd, base, result)?);
    }
    if handoff.verdict.is_none() && !record["verdict"].is_null() {
        handoff.verdict = Some(core.store.handoff_field(&record["verdict"], true)?);
    }
    let _ = source;
    Ok(())
}

fn should_mark_interrupted(
    source: u32,
    round: u32,
    incomplete_worker: bool,
    record: &serde_json::Value,
    handoff: &RoundHandoff,
) -> bool {
    (incomplete_worker
        && source < round
        && handoff
            .report
            .as_ref()
            .is_none_or(|(reported, _)| source > *reported))
        || (source < round
            && record["report"].is_null()
            && handoff
                .report
                .as_ref()
                .is_none_or(|(reported, _)| source > *reported))
}

fn spec_target(task: &Task) -> &str {
    task.spec["id"].as_str().unwrap_or(&task.id)
}
