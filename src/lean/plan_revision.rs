//! One graph transformation, preserving the approved requirements and immutable history.
use super::*;
use crate::lean::tasks::{TaskContract, order_graph};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRevision {
    pub plan: String,
    pub base_revision: i64,
    pub reason: String,
    pub tasks: Vec<TaskContract>,
    pub retire: Vec<String>,
}

impl PlanRevision {
    pub fn merge(&self, baseline: &[Task]) -> Result<Vec<TaskContract>> {
        crate::require(
            !self.reason.trim().is_empty(),
            "revision.reason is required",
        )?;
        let mut specs: Vec<TaskContract> = baseline
            .iter()
            .map(|t| serde_json::from_value(t.spec.clone()))
            .collect::<std::result::Result<_, _>>()?;
        for id in &self.retire {
            crate::require(
                specs.iter().any(|s| &s.id == id),
                "retired task does not exist",
            )?;
        }
        specs.retain(|s| {
            !self.retire.contains(&s.id)
                && !s
                    .options_for
                    .as_ref()
                    .is_some_and(|id| self.retire.contains(id))
        });
        let mut seen = std::collections::BTreeSet::new();
        for task in &self.tasks {
            crate::require(
                !task.quality_gate
                    && task.options_for.is_none()
                    && !baseline.iter().any(|old| {
                        old.spec["id"] == task.id
                            && (old.spec["quality_gate"] == true
                                || old.spec["options_for"].as_str().is_some())
                    }),
                "generated gates and local options are maintained by the runtime; revise the owning task; its options and gates follow it",
            )?;
            crate::require(
                seen.insert(&task.id) && !self.retire.contains(&task.id),
                "revision repeats or retires an updated task",
            )?;
            if let Some(old) = specs.iter_mut().find(|old| old.id == task.id) {
                *old = task.clone();
            } else {
                specs.push(task.clone());
            }
        }
        order_graph(specs)
    }
}

pub(in crate::lean) struct RevisionLease<'a> {
    pub requirement_revision: i64,
    pub request: &'a str,
}

impl Store {
    pub(in crate::lean) fn plan_revision(&self, plan: &str) -> Result<i64> {
        self.db()
            .query_row(
                "SELECT revision FROM plan_heads WHERE plan_id=?1",
                [plan],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::Invalid("plan has no revision head; it was replaced".into()))
    }

    pub(in crate::lean) fn apply_plan_revision(
        &self,
        conversation: &str,
        revision: &PlanRevision,
        baseline: &[Task],
        specs: &[TaskContract],
        affected: &std::collections::BTreeSet<String>,
        lease: &RevisionLease<'_>,
    ) -> Result<()> {
        let run = self
            .plan_run(&revision.plan)?
            .ok_or_else(|| Error::Invalid("plan has no requirements".into()))?;
        let entries = self.ledger_entries(conversation, &run, Some(lease.requirement_revision))?;
        let mut covered = std::collections::HashSet::new();
        for spec in specs {
            crate::lean::ledger::validate_task_links(spec, &entries)?;
            covered.extend(spec.criterion_ids.iter().cloned());
        }
        crate::lean::ledger::validate_criterion_coverage(&entries, &covered)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let current: i64 =
            tx.query_row("SELECT spec_revision FROM runs WHERE id=?1", [&run], |r| {
                r.get(0)
            })?;
        crate::require(
            current == lease.requirement_revision,
            "requirements changed during revision review",
        )?;
        let changed = tx.execute(
            "UPDATE plan_heads SET revision=revision+1 WHERE plan_id=?1 AND revision=?2",
            params![revision.plan, revision.base_revision],
        )?;
        crate::require(changed == 1, "stale plan revision")?;
        for old in baseline {
            let generation: i64 = tx.query_row(
                "SELECT coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0)",
                [&old.id],
                |r| r.get(0),
            )?;
            crate::require(
                generation == old.input_generation,
                "task inputs changed during review",
            )?;
            let stored: String =
                tx.query_row("SELECT spec FROM tasks WHERE id=?1", [&old.id], |r| {
                    r.get(0)
                })?;
            crate::require(
                serde_json::from_str::<Value>(&stored)? == old.spec,
                "contract changed during review",
            )?;
            if affected.contains(&old.id) {
                tx.execute("UPDATE agent_questions SET state='invalidated' WHERE turn_id IN (SELECT turn_id FROM turn_scopes WHERE task_id=?1) AND state IN ('pending','awaiting_user')",[&old.id])?;
            }
            if !specs.iter().any(|s| old.spec["id"] == s.id) {
                tx.execute(
                    "UPDATE task_inputs SET generation=generation+1 WHERE task_id=?1",
                    [&old.id],
                )?;
                tx.execute(
                    "UPDATE tasks SET status='superseded' WHERE id=?1",
                    [&old.id],
                )?;
            }
        }
        for (position, spec) in specs.iter().enumerate() {
            let encoded = serde_json::to_string(spec)?;
            let old = baseline.iter().find(|t| t.spec["id"] == spec.id);
            let id = old.map_or_else(crate::id, |t| t.id.clone());
            match old {
                Some(_) if affected.contains(&id) => {
                    tx.execute("UPDATE tasks SET spec=?2,position=?3,status='queued',verdict=NULL,feedback=NULL,base=NULL,result=NULL WHERE id=?1",params![id,encoded,position as i64])?;
                    tx.execute(
                        "UPDATE task_inputs SET generation=generation+1 WHERE task_id=?1",
                        [&id],
                    )?;
                }
                Some(_) => {
                    tx.execute(
                        "UPDATE tasks SET position=?2 WHERE id=?1",
                        params![id, position as i64],
                    )?;
                }
                None => {
                    tx.execute("INSERT INTO tasks(id,conversation_id,plan_id,position,spec,status,created_ms) VALUES(?1,?2,?3,?4,?5,'queued',?6)",params![id,conversation,revision.plan,position as i64,encoded,now_ms()])?;
                    tx.execute(
                        "INSERT INTO task_inputs(task_id,run_id) VALUES(?1,?2)",
                        params![id, run],
                    )?;
                }
            }
            if old.is_none() || affected.contains(&id) {
                tx.execute("DELETE FROM task_ledger_inputs WHERE task_id=?1", [&id])?;
                for entry in entries
                    .iter()
                    .filter(|e| spec.requirement_ids.contains(&e.spec.id))
                {
                    tx.execute(
                        "INSERT INTO task_ledger_inputs VALUES(?1,?2,?3)",
                        params![id, entry.spec.id, entry.version],
                    )?;
                }
                tx.execute(
                    "INSERT INTO task_contracts(task_id,version,spec) VALUES(?1,?2,?3)",
                    params![id, revision.base_revision + 1, encoded],
                )?;
                tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'task_contract_changed',?3,?4)",params![conversation,revision.plan,serde_json::json!({"task":id,"version":revision.base_revision+1}).to_string(),now_ms()])?;
            }
        }
        for spec in specs {
            if baseline
                .iter()
                .any(|old| old.spec["id"] == spec.id && !affected.contains(&old.id))
            {
                continue;
            }
            let (id,status,round):(String,String,i64)=tx.query_row("SELECT id,status,round FROM tasks WHERE plan_id=?1 AND json_extract(spec,'$.id')=?2 AND status<>'superseded'",params![revision.plan,spec.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
            tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'graph_task',?3,?4)",params![conversation,revision.plan,serde_json::json!({"plan":revision.plan,"id":id,"status":status,"round":round,"spec":spec}).to_string(),now_ms()])?;
        }
        tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'graph_updated',?3,?4)",params![conversation,revision.plan,serde_json::json!({"plan":revision.plan,"text":revision.reason,"order":specs.iter().map(|s|&s.id).collect::<Vec<_>>(),"revision":revision.base_revision+1}).to_string(),now_ms()])?;
        tx.execute("UPDATE runs SET state='active' WHERE id=?1", [run])?;
        let changed = tx.execute("UPDATE plan_revision_jobs SET status='applied',error=NULL WHERE id=?1 AND status='reviewing'",[lease.request])?;
        crate::require(changed == 1, "revision receipt is no longer active")?;
        tx.commit()?;
        Ok(())
    }
}
