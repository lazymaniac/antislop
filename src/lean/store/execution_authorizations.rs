//! Explicit user/client grants, never agent-inferred permission or ledger edits.
use super::*;
use crate::lean::ledger::{LedgerEntry, RuntimeConstraint};
use crate::lean::tasks::TaskContract;

type GrantBinding = (String, String, i64, String, i64, String);

/// A trusted client binds an already answered decision to one exact upcoming
/// contract. This control request is deliberately not exposed as an MCP tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionAuthorization {
    pub conversation: String,
    pub plan: String,
    pub base_revision: i64,
    pub decision: String,
    pub selected_option: String,
    pub requirement: String,
    pub contract: TaskContract,
}

impl Store {
    pub(in crate::lean) fn authorize_task_execution(
        &self,
        grant: &ExecutionAuthorization,
    ) -> Result<Value> {
        crate::require(
            self.accepted_plan(&grant.conversation)?.as_deref() == Some(&grant.plan),
            "authorization must name the accepted plan",
        )?;
        crate::require(
            self.plan_revision(&grant.plan)? == grant.base_revision,
            "stale authorization plan revision",
        )?;
        let task = self
            .plan(&grant.plan)?
            .into_iter()
            .find(|t| t.spec["id"] == grant.contract.id)
            .ok_or_else(|| Error::Invalid("authorization task is not in this plan".into()))?;
        crate::require(
            !grant.contract.deferred && grant.contract.writes && grant.contract.changes_code(),
            "authorize only the specific released code writer",
        )?;
        crate::require(
            task.spec != serde_json::to_value(&grant.contract)?,
            "authorization must bind the forthcoming changed contract",
        )?;
        let question = self.db().query_row("SELECT payload FROM events WHERE conversation_id=?1 AND kind='user_decision_requested' ORDER BY seq DESC LIMIT 1", [&grant.conversation], |r|r.get::<_,String>(0))?;
        let question: crate::lean::interview::Question = serde_json::from_str(&question)?;
        crate::require(
            question.id == grant.decision
                && question
                    .options
                    .iter()
                    .any(|o| o.label == grant.selected_option),
            "authorization must reference the latest decision and its exact selected option",
        )?;
        let prefix = format!("Interview answer ({})\n", grant.decision);
        let answer = self
            .messages_by(&grant.conversation, "user", 0)?
            .into_iter()
            .rev()
            .find(|m| m.text.starts_with(&prefix))
            .ok_or_else(|| Error::Invalid("execution decision has no user answer".into()))?;
        crate::require(
            answer.text
                == format!(
                    "{prefix}Question: {}\nAnswer: {}",
                    question.question, grant.selected_option
                ),
            "execution decision answer does not match the supplied option",
        )?;
        let run = self
            .plan_run(&grant.plan)?
            .ok_or_else(|| Error::Invalid("plan has no requirements".into()))?;
        let entry = self
            .ledger_entries(&grant.conversation, &run, None)?
            .into_iter()
            .find(|e| {
                e.spec.id == grant.requirement
                    && e.spec
                        .constraints
                        .contains(&RuntimeConstraint::NoNewExecutions)
            })
            .ok_or_else(|| Error::Invalid("authorization names no execution restriction".into()))?;
        crate::require(
            entry.spec.disposition == crate::lean::ledger::Disposition::Active,
            "authorization restriction is not active",
        )?;
        let (raw_hash, hash) = self.execution_contract_hashes(&grant.plan, &grant.contract)?;
        let generation = task.input_generation + 1;
        let mut db = self.db();
        let tx = db.transaction()?;
        crate::require(
            tx.query_row(
                "SELECT revision FROM plan_heads WHERE plan_id=?1",
                [&grant.plan],
                |r| r.get::<_, i64>(0),
            )? == grant.base_revision,
            "stale authorization plan revision",
        )?;
        let generation_now: i64 = tx.query_row(
            "SELECT coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0)",
            [&task.id],
            |r| r.get(0),
        )?;
        let stored: String =
            tx.query_row("SELECT spec FROM tasks WHERE id=?1", [&task.id], |r| {
                r.get(0)
            })?;
        let version_now: i64 = tx.query_row(
            "SELECT max(version) FROM ledger_versions WHERE run_id=?1 AND entry_id=?2",
            params![run, entry.spec.id],
            |r| r.get(0),
        )?;
        crate::require(
            generation_now == task.input_generation
                && serde_json::from_str::<Value>(&stored)? == task.spec
                && version_now == entry.version,
            "authorization inputs changed",
        )?;
        let old: Option<GrantBinding> = tx.query_row("SELECT conversation_id,task_id,generation,requirement_id,requirement_version,contract_hash FROM execution_authorizations WHERE decision_id=?1 AND conversation_id=?2",params![grant.decision,grant.conversation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
        if let Some(old) = old {
            crate::require(
                old == (
                    grant.conversation.clone(),
                    task.id.clone(),
                    generation,
                    grant.requirement.clone(),
                    entry.version,
                    old.5.clone(),
                ) && (old.5 == hash || old.5 == raw_hash),
                "decision is already bound to another contract; it cannot authorize broader work",
            )?;
        } else {
            tx.execute("INSERT INTO execution_authorizations(decision_id,conversation_id,plan_id,task_id,local_task,generation,requirement_id,requirement_version,contract_hash,answer_seq,base_revision,created_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![grant.decision,grant.conversation,grant.plan,task.id,grant.contract.id,generation,grant.requirement,entry.version,hash,answer.seq as i64,grant.base_revision,now_ms()])?;
            tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'task_execution_authorized',?3,?4)",params![grant.conversation,grant.plan,serde_json::json!({"actor":"user","decision":grant.decision,"selected_option":grant.selected_option,"answer":answer.seq,"task":task.id,"local_task":grant.contract.id,"generation":generation,"contract_hash":hash,"requirement":grant.requirement,"requirement_version":entry.version}).to_string(),now_ms()])?;
        }
        tx.commit()?;
        Ok(
            serde_json::json!({"authorized":grant.contract.id,"decision":grant.decision,"generation":generation,"contract_hash":hash}),
        )
    }

    /// Authoritative launch metadata, separate from code/qualification approval.
    pub(in crate::lean) fn task_execution_approval_context(&self, task: &Task) -> Result<Value> {
        let contract: TaskContract = serde_json::from_value(task.spec.clone())?;
        let (raw_hash, hash) = self.execution_contract_hashes(&task.plan_id, &contract)?;
        let db = self.db();
        let accepted: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM plan_authorizations WHERE plan_id=?1)",
            [&task.plan_id],
            |r| r.get(0),
        )?;
        let revision: Option<i64> = db
            .query_row(
                "SELECT revision FROM plan_heads WHERE plan_id=?1",
                [&task.plan_id],
                |r| r.get(0),
            )
            .optional()?;
        let applied:Option<String>=db.query_row("SELECT id FROM plan_revision_jobs WHERE plan_id=?1 AND status='applied' ORDER BY rowid DESC LIMIT 1",[&task.plan_id],|r|r.get(0)).optional()?;
        let mut query=db.prepare("SELECT decision_id,answer_seq,requirement_id,requirement_version FROM execution_authorizations WHERE plan_id=?1 AND task_id=?2 AND generation=?3 AND contract_hash IN (?4,?5) ORDER BY decision_id")?;
        let permissions=query.query_map(params![task.plan_id,task.id,task.input_generation,hash,raw_hash],|r|Ok(serde_json::json!({"decision":r.get::<_,String>(0)?,"answer_message":r.get::<_,i64>(1)?,"requirement":r.get::<_,String>(2)?,"requirement_version":r.get::<_,i64>(3)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(
            serde_json::json!({"plan":task.plan_id,"plan_accepted":accepted,"current_plan_revision":revision,"last_applied_reviewed_revision":applied,"task":task.id,"input_generation":task.input_generation,"contract_hash":hash,"scoped_execution_authorizations":permissions}),
        )
    }

    pub(in crate::lean) fn validate_task_execution_constraints(&self, task: &Task) -> Result<()> {
        self.refresh_unused_execution_authorizations(task)?;
        let Some(run) = self.plan_run(&task.plan_id)? else {
            return Ok(());
        };
        let conversation = self.plan_conversation(&task.plan_id)?;
        let entries = self.ledger_entries(&conversation, &run, None)?;
        let contract: TaskContract = serde_json::from_value(task.spec.clone())?;
        let entries = self.authorized_task_constraints(
            &conversation,
            task,
            &contract,
            task.input_generation,
            &entries,
        )?;
        crate::lean::ledger::validate_runtime_constraints(std::slice::from_ref(&contract), &entries)
    }

    /// Planned dependency work can refresh inputs before the first writer turn.
    /// Preserve the exact contract allowance, without renewing any used grant.
    fn refresh_unused_execution_authorizations(&self, task: &Task) -> Result<()> {
        let contract: TaskContract = serde_json::from_value(task.spec.clone())?;
        let (raw_hash, hash) = self.execution_contract_hashes(&task.plan_id, &contract)?;
        let mut db = self.db();
        let candidates: Vec<(String, i64, String)> = {
            let mut query = db.prepare("SELECT decision_id,generation,contract_hash FROM execution_authorizations WHERE plan_id=?1 AND task_id=?2 AND contract_hash IN (?3,?5) AND generation<?4")?;
            query
                .query_map(
                    params![task.plan_id, task.id, hash, task.input_generation, raw_hash],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        if candidates.is_empty() {
            return Ok(());
        }
        let tx = db.transaction()?;
        let current: i64 = tx.query_row(
            "SELECT coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0)",
            [&task.id],
            |r| r.get(0),
        )?;
        let stored: String =
            tx.query_row("SELECT spec FROM tasks WHERE id=?1", [&task.id], |r| {
                r.get(0)
            })?;
        crate::require(
            current == task.input_generation
                && serde_json::from_str::<Value>(&stored)? == task.spec,
            "authorization inputs changed before launch",
        )?;
        for (decision, generation, hash) in candidates {
            let started: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM turns t JOIN turn_scopes s ON s.turn_id=t.id JOIN turn_inputs x ON x.turn_id=t.id WHERE s.task_id=?1 AND t.role='worker:'||?1 AND x.generation>=?2)",params![task.id,generation],|r|r.get(0))?;
            if started {
                continue;
            }
            let changed = tx.execute("UPDATE execution_authorizations SET generation=?3 WHERE decision_id=?1 AND task_id=?2 AND generation=?4 AND contract_hash=?5",params![decision,task.id,current,generation,hash])?;
            if changed == 1 {
                tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) SELECT conversation_id,plan_id,'task_execution_authorization_rebound',?3,?4 FROM execution_authorizations WHERE decision_id=?1 AND task_id=?2",params![decision,task.id,serde_json::json!({"actor":"runtime","decision":decision,"task":task.id,"previous_generation":generation,"generation":current,"contract_hash":hash,"reason":"planned inputs refreshed before first authorized writer turn; no budget renewed"}).to_string(),now_ms()])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub(in crate::lean) fn authorized_task_constraints(
        &self,
        conversation: &str,
        task: &Task,
        contract: &TaskContract,
        generation: i64,
        entries: &[LedgerEntry],
    ) -> Result<Vec<LedgerEntry>> {
        let (raw_hash, hash) = self.execution_contract_hashes(&task.plan_id, contract)?;
        let db = self.db();
        let mut adjusted = entries.to_vec();
        for entry in &mut adjusted {
            let allowed:bool = db.query_row("SELECT EXISTS(SELECT 1 FROM execution_authorizations WHERE conversation_id=?1 AND plan_id=?2 AND task_id=?3 AND local_task=?4 AND generation=?5 AND requirement_id=?6 AND requirement_version=?7 AND contract_hash IN (?8,?9))",params![conversation,task.plan_id,task.id,contract.id,generation,entry.spec.id,entry.version,hash,raw_hash],|r|r.get(0))?;
            if allowed {
                entry
                    .spec
                    .constraints
                    .retain(|c| *c != RuntimeConstraint::NoNewExecutions);
            }
        }
        Ok(adjusted)
    }

    /// Finalization attaches an existing runtime-owned options prerequisite.
    /// It cannot change the user-authorized scope or any authored dependency.
    /// Keep the raw hash as a compatibility lookup for previously stored grants.
    fn execution_contract_hashes(
        &self,
        plan: &str,
        contract: &TaskContract,
    ) -> Result<(String, String)> {
        let raw = crate::hash(serde_json::to_vec(contract)?);
        let options = {
            let db = self.db();
            let mut query = db.prepare("SELECT json_extract(spec,'$.id') FROM tasks WHERE plan_id=?1 AND json_extract(spec,'$.options_for')=?2 AND status<>'superseded'")?;
            query
                .query_map(params![plan, contract.id], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut authored = contract.clone();
        authored.depends_on.retain(|id| !options.contains(id));
        Ok((raw, crate::hash(serde_json::to_vec(&authored)?)))
    }

    pub(in crate::lean) fn validate_authorized_plan_constraints(
        &self,
        conversation: &str,
        plan: &str,
        specs: &[TaskContract],
        baseline: &[Task],
        redo: &[String],
        entries: &[LedgerEntry],
    ) -> Result<()> {
        crate::require(
            self.plan_conversation(plan)? == conversation,
            "authorization plan belongs to another conversation",
        )?;
        for spec in specs {
            let entries = if let Some(old) = baseline.iter().find(|t| t.spec["id"] == spec.id) {
                let changed = old.spec != serde_json::to_value(spec)? || redo.contains(&spec.id);
                self.authorized_task_constraints(
                    conversation,
                    old,
                    spec,
                    old.input_generation + i64::from(changed),
                    entries,
                )?
            } else {
                entries.to_vec()
            };
            crate::lean::ledger::validate_plan_runtime_constraints(
                std::slice::from_ref(spec),
                baseline,
                redo,
                &entries,
            )?;
        }
        Ok(())
    }
}
