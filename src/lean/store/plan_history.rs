use super::*;

impl Store {
    /// Detailed history of one plan: its tasks, agents, turns, reports and verdicts.
    pub fn plan_history(&self, conversation: &str, plan_id: &str) -> Result<PlanHistory> {
        self.conversation(conversation)?;
        let tasks = self.plan(plan_id)?;
        crate::require(
            !tasks.is_empty(),
            &format!("no plan {plan_id} in conversation {conversation}"),
        )?;

        let db = self.db();
        let (created_ms, is_active) = plan_history_timing(&db, conversation, plan_id)?;
        let status = plan_history_status(&tasks);
        let (turns, agents) = plan_history_sessions(&db, conversation, plan_id, &tasks)?;
        let (reports, verdicts) = plan_history_outcomes(&tasks);
        let run_id = plan_history_run(&db, plan_id)?;
        Ok(PlanHistory {
            run_id,
            id: plan_id.to_string(),
            conversation_id: conversation.to_string(),
            created_ms,
            status,
            is_active,
            tasks,
            agents,
            turns,
            reports,
            verdicts,
        })
    }
}

fn plan_history_timing(
    db: &rusqlite::Connection,
    conversation: &str,
    plan_id: &str,
) -> Result<(i64, bool)> {
    let created_ms: i64 = db.query_row(
        "SELECT COALESCE(
                (SELECT created_ms FROM plans WHERE id=?1 AND conversation_id=?2),
                (SELECT MIN(created_ms) FROM tasks WHERE plan_id=?1 AND conversation_id=?2),
                0
            )",
        params![plan_id, conversation],
        |row| row.get(0),
    )?;

    let active_plan_id: Option<String> = db
            .query_row(
                "SELECT plan_id FROM tasks WHERE conversation_id=?1 ORDER BY created_ms DESC, rowid DESC LIMIT 1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?;
    Ok((created_ms, active_plan_id.as_deref() == Some(plan_id)))
}

fn plan_history_sessions(
    db: &rusqlite::Connection,
    conversation: &str,
    plan_id: &str,
    tasks: &[Task],
) -> Result<(Vec<SessionInfo>, Vec<PlanAgentInfo>)> {
    let task_ids = tasks
        .iter()
        .map(|task| task.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let all_turns = plan_history_turn_rows(db, conversation)?;
    let turns = all_turns
        .into_iter()
        .filter(|turn| turn_belongs_to_plan(turn, plan_id, &task_ids))
        .collect::<Vec<_>>();
    let agents = plan_history_agents(&turns, tasks);
    Ok((turns, agents))
}

fn plan_history_turn_rows(
    db: &rusqlite::Connection,
    conversation: &str,
) -> Result<Vec<SessionInfo>> {
    let mut turn_stmt = db.prepare(
            "SELECT id, plan_id, role, provider, session_id, status, error, upto_message, started_ms, finished_ms, pid, (SELECT backend FROM run_contexts r WHERE r.turn_id=turns.id)
             FROM turns WHERE conversation_id=?1 ORDER BY started_ms ASC, rowid ASC",
        )?;
    Ok(turn_stmt
        .query_map([conversation], |row| {
            Ok(SessionInfo {
                turn_id: row.get(0)?,
                plan_id: row.get(1)?,
                role: row.get(2)?,
                provider: row.get(3)?,
                session_id: row.get(4)?,
                status: row.get(5)?,
                error: row.get(6)?,
                upto_message: row.get::<_, i64>(7)? as u64,
                started_ms: row.get(8)?,
                finished_ms: row.get(9)?,
                pid: row.get(10)?,
                backend: row.get(11)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?)
}

fn plan_history_run(db: &rusqlite::Connection, plan_id: &str) -> Result<Option<String>> {
    Ok(db
        .query_row(
            "SELECT run_id FROM plan_runs WHERE plan_id=?1",
            [plan_id],
            |row| row.get(0),
        )
        .optional()?)
}

fn plan_history_status(tasks: &[Task]) -> String {
    if tasks.iter().all(|t| t.status == "approved") {
        "approved".to_string()
    } else if tasks
        .iter()
        .any(|t| t.status == "working" || t.status == "verifying" || t.status == "queued")
    {
        "active".to_string()
    } else if tasks.iter().all(|t| t.status == "proposed") {
        "proposed".to_string()
    } else if tasks.iter().any(|t| t.status == "failed") {
        "failed".to_string()
    } else if tasks.iter().any(|t| t.status == "cancelled") {
        "cancelled".to_string()
    } else if tasks.iter().any(|t| t.status == "needs_user") {
        "needs_user".to_string()
    } else if tasks.iter().any(|t| t.status == "revised") {
        "revised".to_string()
    } else {
        "done".to_string()
    }
}

fn turn_belongs_to_plan(
    turn: &SessionInfo,
    plan_id: &str,
    task_ids: &std::collections::HashSet<&str>,
) -> bool {
    turn.plan_id.as_deref() == Some(plan_id)
        || crate::lean::task_role(&turn.role).is_some_and(|(id, _)| task_ids.contains(id))
}

fn plan_history_agents(turns: &[SessionInfo], tasks: &[Task]) -> Vec<PlanAgentInfo> {
    let mut agents = std::collections::BTreeMap::<String, PlanAgentInfo>::new();
    for turn in turns {
        let agent = agents
            .entry(turn.role.clone())
            .or_insert_with(|| plan_agent_for_turn(turn, tasks));
        agent.turns_count += 1;
        agent.status.clone_from(&turn.status);
        agent.provider.clone_from(&turn.provider);
    }
    agents.into_values().collect()
}

fn plan_agent_for_turn(turn: &SessionInfo, tasks: &[Task]) -> PlanAgentInfo {
    let task_id = crate::lean::task_role(&turn.role).map(|(id, _)| id);
    let task_label = task_id.and_then(|id| {
        tasks
            .iter()
            .find(|task| task.id == id)
            .and_then(|task| task.spec["id"].as_str())
            .map(str::to_owned)
    });
    PlanAgentInfo {
        role: turn.role.clone(),
        provider: turn.provider.clone(),
        model: None,
        task_id: task_id.map(str::to_owned),
        task_label,
        turns_count: 0,
        status: turn.status.clone(),
    }
}

fn plan_history_outcomes(tasks: &[Task]) -> (Vec<TaskReportInfo>, Vec<TaskVerdictInfo>) {
    let mut reports = Vec::new();
    let mut verdicts = Vec::new();
    for task in tasks {
        let label = task.spec["id"].as_str().unwrap_or(&task.id).to_owned();
        add_report(task, &label, &mut reports);
        add_verdict(task, label, &mut verdicts);
    }
    (reports, verdicts)
}

fn add_report(task: &Task, label: &str, reports: &mut Vec<TaskReportInfo>) {
    if let Some(report) = &task.report {
        reports.push(TaskReportInfo {
            task_id: task.id.clone(),
            task_label: label.to_owned(),
            round: task.round,
            report: report.clone(),
        });
    }
}

fn add_verdict(task: &Task, label: String, verdicts: &mut Vec<TaskVerdictInfo>) {
    if let Some(verdict) = &task.verdict {
        verdicts.push(TaskVerdictInfo {
            task_id: task.id.clone(),
            task_label: label,
            round: task.round,
            verdict: verdict.clone(),
        });
    }
}
