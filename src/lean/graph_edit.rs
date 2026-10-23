//! Atomic additive edits to a running DAG. Executed tasks are immutable.
use super::{Store, Task, now_ms};
use crate::lean::{
    ledger::{Disposition, LedgerKind},
    tasks::{TaskSpec, status},
};
use crate::{Result, require};
use rusqlite::params;

impl Store {
    pub fn extend_running_graph(
        &self,
        conversation: &str,
        baseline: &[Task],
        specs: &[TaskSpec],
    ) -> Result<()> {
        require(!baseline.is_empty(), "no graph to edit")?;
        crate::lean::tasks::validate(specs)?;
        let plan = &baseline[0].plan_id;
        let run = self
            .plan_run(plan)?
            .ok_or_else(|| crate::Error::Invalid("graph has no requirements run".into()))?;
        require(
            self.extraction_current(conversation, &run)?,
            "requirements need independent verification before editing the graph",
        )?;
        let entries = self.ledger_entries(conversation, &run, None)?;
        for task in baseline {
            self.assert_task_inputs(task)?;
        }
        let mut db = self.db();
        let tx = db.transaction()?;
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM tasks WHERE conversation_id=?1 AND plan_id=?2",
            params![conversation, plan],
            |r| r.get(0),
        )?;
        require(
            count == baseline.len() as i64,
            "graph changed during review; retry against its latest version",
        )?;
        let unfinished: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE plan_id=?1 AND status IN ('queued','working','verifying'))",[plan],|r|r.get(0))?;
        require(
            unfinished,
            "the graph finished during review; start a new interview for further work",
        )?;
        let stale: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM tasks t JOIN task_inputs i ON i.task_id=t.id JOIN task_ledger_inputs l ON l.task_id=t.id WHERE t.plan_id=?1 AND l.version<>(SELECT max(v.version) FROM ledger_versions v WHERE v.run_id=i.run_id AND v.entry_id=l.entry_id))",[plan],|r|r.get(0))?;
        require(
            !stale,
            "graph requirements changed during review; replan first",
        )?;
        for old in baseline {
            let stored: String = tx.query_row(
                "SELECT spec FROM tasks WHERE id=?1 AND plan_id=?2 AND conversation_id=?3",
                params![old.id, plan, conversation],
                |r| r.get(0),
            )?;
            require(
                serde_json::from_str::<serde_json::Value>(&stored)? == old.spec,
                "graph changed during review; retry against its latest version",
            )?;
            require(
                specs.iter().any(|s| old.spec["id"].as_str() == Some(&s.id)),
                "graph edits cannot remove existing tasks",
            )?;
        }
        for (position, spec) in specs.iter().enumerate() {
            let value = serde_json::to_value(spec)?;
            let (uuid, task_status, round) = if let Some(old) = baseline
                .iter()
                .find(|old| old.spec["id"].as_str() == Some(&spec.id))
            {
                if serde_json::from_value::<TaskSpec>(old.spec.clone())? == *spec {
                    tx.execute(
                        "UPDATE tasks SET position=?2 WHERE id=?1",
                        params![old.id, position as i64],
                    )?;
                    continue;
                }
                let mut before: TaskSpec = serde_json::from_value(old.spec.clone())?;
                before.depends_on = spec.depends_on.clone();
                require(
                    before == *spec,
                    "running graph edits may change only dependencies of an existing unstarted task; add a new task for other changes",
                )?;
                let (state, round): (String, u32) = tx.query_row(
                    "SELECT status,round FROM tasks WHERE id=?1",
                    [&old.id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                require(
                    matches!(state.as_str(), status::QUEUED | status::PROPOSED) && round == 0,
                    &format!(
                        "task {} has already started; its dependencies and history cannot be rewritten",
                        spec.id
                    ),
                )?;
                tx.execute(
                    "UPDATE tasks SET spec=?2,position=?3 WHERE id=?1",
                    params![old.id, value.to_string(), position as i64],
                )?;
                (old.id.clone(), state, round)
            } else {
                require(
                    !spec.requirement_ids.is_empty() || !spec.plan_item_ids.is_empty(),
                    "added task needs a link to an active requirement or plan item",
                )?;
                for (ids, kind) in [
                    (&spec.requirement_ids, LedgerKind::Requirement),
                    (&spec.plan_item_ids, LedgerKind::PlanItem),
                ] {
                    for id in ids {
                        require(
                            entries.iter().any(|e| {
                                e.spec.id == *id
                                    && e.spec.kind == kind
                                    && e.spec.disposition == Disposition::Active
                            }),
                            "added task refers to an unknown or inactive requirement/plan item",
                        )?;
                    }
                }
                for criterion in &spec.criterion_ids {
                    require(
                        entries.iter().any(|e| {
                            spec.requirement_ids.contains(&e.spec.id)
                                && e.spec.criteria.iter().any(|c| c.id == *criterion)
                        }),
                        "added task criterion is not owned by a linked requirement",
                    )?;
                }
                let uuid = crate::id();
                tx.execute("INSERT INTO tasks(id,conversation_id,plan_id,position,spec,status,created_ms) VALUES(?1,?2,?3,?4,?5,'queued',?6)",params![uuid,conversation,plan,position as i64,value.to_string(),now_ms()])?;
                tx.execute(
                    "INSERT INTO task_inputs(task_id,run_id) VALUES(?1,?2)",
                    params![uuid, run],
                )?;
                let ids: std::collections::BTreeSet<_> = spec
                    .requirement_ids
                    .iter()
                    .chain(&spec.plan_item_ids)
                    .collect();
                for id in ids {
                    let entry = entries
                        .iter()
                        .find(|e| e.spec.id == *id)
                        .expect("validated entry");
                    let latest: i64 = tx.query_row(
                        "SELECT max(version) FROM ledger_versions WHERE run_id=?1 AND entry_id=?2",
                        params![run, id],
                        |r| r.get(0),
                    )?;
                    require(
                        latest == entry.version,
                        "requirements changed during graph review; retry",
                    )?;
                    tx.execute(
                        "INSERT INTO task_ledger_inputs VALUES(?1,?2,?3)",
                        params![uuid, id, entry.version],
                    )?;
                }
                (uuid, "queued".into(), 0)
            };
            tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'graph_task',?3,?4)",params![conversation,plan,serde_json::json!({"plan":plan,"id":uuid,"spec":spec,"status":task_status,"round":round,"old_dependencies":baseline.iter().find(|old| old.spec["id"].as_str()==Some(&spec.id)).map(|old| &old.spec["depends_on"])}).to_string(),now_ms()])?;
        }
        tx.commit()?;
        Ok(())
    }
}
