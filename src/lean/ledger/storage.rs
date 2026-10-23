use super::*;
use crate::{
    Error,
    lean::{
        store::{Store, now_ms},
        tasks::TaskSpec,
    },
};
use rusqlite::{OptionalExtension, params};
use std::collections::{HashMap, HashSet};

fn invalid(text: &str) -> Error {
    Error::Invalid(text.into())
}
fn run(db: &Connection, conversation: &str, id: &str) -> Result<Run> {
    db.query_row("SELECT id,conversation_id,goal,spec_revision,state,start_message FROM runs WHERE id=?1 AND conversation_id=?2", params![id,conversation], |r| Ok(Run { id:r.get(0)?, conversation_id:r.get(1)?, goal:r.get(2)?, spec_revision:r.get(3)?, state:r.get(4)?,start_message:r.get::<_,i64>(5)? as u64 })).optional()?.ok_or_else(|| invalid("run does not belong to conversation"))
}
fn relations(
    db: &Connection,
    run: &str,
    id: &str,
    version: i64,
    kind: &str,
) -> Result<Vec<String>> {
    Ok(db.prepare("SELECT target FROM ledger_relations WHERE run_id=?1 AND entry_id=?2 AND version=?3 AND kind=?4 ORDER BY target")?.query_map(params![run,id,version,kind], |r| r.get(0))?.collect::<std::result::Result<_,_>>()?)
}
fn entries(db: &Connection, run: &str, revision: i64) -> Result<Vec<LedgerEntry>> {
    let mut stmt = db.prepare("SELECT v.entry_id,v.version,v.spec_revision,v.source_id,v.start_byte,v.end_byte,v.original,v.interpretation,v.mandatory,v.position,v.disposition
      FROM ledger_versions v JOIN ledger_entries e ON e.run_id=v.run_id AND e.id=v.entry_id
      WHERE v.run_id=?1 AND v.spec_revision<=?2 AND v.version=(SELECT max(w.version) FROM ledger_versions w WHERE w.run_id=v.run_id AND w.entry_id=v.entry_id AND w.spec_revision<=?2) ORDER BY v.position,v.entry_id")?;
    let rows = stmt
        .query_map(params![run, revision], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)? as usize,
                r.get::<_, i64>(5)? as usize,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, bool>(8)?,
                r.get::<_, u32>(9)?,
                r.get::<_, String>(10)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut sources = HashMap::<String, String>::new();
    rows.into_iter().map(|(id,version,spec_revision,source_id,start,end,_original,interpretation,mandatory,position,disposition)| {
        let criteria = db.prepare("SELECT id,text,mandatory,integration FROM ledger_criteria WHERE run_id=?1 AND entry_id=?2 AND version=?3 ORDER BY id")?.query_map(params![run,id,version], |r| Ok(Criterion{id:r.get(0)?,text:r.get(1)?,mandatory:r.get(2)?,integration:r.get(3)?}))?.collect::<std::result::Result<_,_>>()?;
        let state=std::path::Path::new(db.path().unwrap_or("")).parent().ok_or_else(||invalid("ledger database has no source directory"))?;
        if !sources.contains_key(&source_id) {sources.insert(source_id.clone(),source_text(db,run,&source_id,state)?);}
        let original=SourceSpan{source_id:source_id.clone(),start,end}.text(&sources[&source_id])?.to_owned();
        Ok(LedgerEntry{run_id:run.into(),version,spec_revision,original,status:"identified".into(),spec:EntrySpec{
            source:SourceSpan{source_id,start,end}, interpretation,mandatory,position,
            disposition:serde_json::from_value(serde_json::Value::String(disposition))?, criteria,
            qualifiers:relations(db,run,&id,version,"qualifier")?,
            parent:relations(db,run,&id,version,"parent")?.into_iter().next(),
            depends_on:relations(db,run,&id,version,"dependency")?,
            task_ids:relations(db,run,&id,version,"task")?,
            implementation_owners:relations(db,run,&id,version,"implementer")?,
            verification_owners:relations(db,run,&id,version,"verifier")?, id,
        }})
    }).collect()
}
fn source_text(
    db: &Connection,
    run: &str,
    source: &str,
    state: &std::path::Path,
) -> Result<String> {
    let (message, locator, hash): (Option<i64>, Option<String>, String) = db.query_row(
        "SELECT message,locator,hash FROM ledger_sources WHERE id=?1 AND run_id=?2",
        params![source, run],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let text = if let Some(message) = message {
        let encoded: String =
            db.query_row("SELECT text FROM messages WHERE seq=?1", [message], |r| {
                r.get(0)
            })?;
        crate::lean::content::Content::new(&state.join("core.db")).decode(&encoded)?
    } else {
        let locator = locator.ok_or_else(|| invalid("source has no locator"))?;
        require(
            locator == format!("sources/{hash}.txt"),
            "invalid source artifact locator",
        )?;
        let path = state.join(locator);
        require(
            std::fs::symlink_metadata(&path)?.file_type().is_file(),
            "source artifact must be a regular immutable file",
        )?;
        std::fs::read_to_string(path)?
    };
    require(
        crate::hash(text.as_bytes()) == hash,
        "source artifact hash mismatch",
    )?;
    Ok(text)
}
fn checked_graph(all: &[EntrySpec]) -> Result<()> {
    let index: HashMap<_, _> = all.iter().map(|e| (e.id.as_str(), e)).collect();
    require(index.len() == all.len(), "duplicate ledger identity")?;
    let mut criterion_ids = HashSet::new();
    for e in all {
        require(
            !e.id.trim().is_empty() && !e.interpretation.trim().is_empty(),
            "ledger identity and interpretation are required",
        )?;
        require(
            e.criteria.iter().all(|c| {
                !c.id.trim().is_empty()
                    && !c.text.trim().is_empty()
                    && criterion_ids.insert(c.id.as_str())
            }),
            "criteria need unique identities and nonempty text",
        )?;
        if let Some(parent) = &e.parent {
            require(
                index.contains_key(parent.as_str()),
                &format!("entry {} has unknown parent {parent}", e.id),
            )?;
        }
        for id in e.depends_on.iter() {
            require(index.contains_key(id.as_str()), "unknown ledger dependency")?;
        }
        if all
            .iter()
            .any(|child| child.parent.as_deref() == Some(&e.id))
            && e.mandatory
        {
            require(
                e.criteria.iter().any(|c| c.mandatory && c.integration),
                "composite requirement needs its own integration criterion",
            )?;
        }
        let mut todo = vec![e.id.as_str()];
        let mut seen = HashSet::new();
        while let Some(id) = todo.pop() {
            if !seen.insert(id) {
                continue;
            }
            let node = index[id];
            for dep in node.depends_on.iter().chain(node.parent.iter()) {
                require(dep != &e.id, "cyclic ledger dependency or hierarchy")?;
                todo.push(dep);
            }
        }
    }
    Ok(())
}
fn impact(
    db: &Connection,
    run: &str,
    changed: &HashSet<String>,
    all: &[EntrySpec],
) -> Result<Vec<String>> {
    let mut ids = changed.clone();
    loop {
        let n = ids.len();
        for e in all {
            if e.depends_on
                .iter()
                .chain(e.parent.iter())
                .any(|id| ids.contains(id))
                || all
                    .iter()
                    .any(|child| child.parent.as_deref() == Some(&e.id) && ids.contains(&child.id))
            {
                ids.insert(e.id.clone());
            }
        }
        if n == ids.len() {
            break;
        }
    }
    let mut tasks = HashSet::new();
    let mut stmt=db.prepare("SELECT i.task_id,l.entry_id FROM task_inputs i JOIN task_ledger_inputs l ON i.task_id=l.task_id WHERE i.run_id=?1")?;
    for row in stmt.query_map([run], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (task, id) = row?;
        if ids.contains(&id) {
            tasks.insert(task);
        }
    }
    let rows=db.prepare("SELECT t.id,t.plan_id,t.spec FROM tasks t JOIN plan_runs p ON p.plan_id=t.plan_id WHERE p.run_id=?1")?.query_map([run], |r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
    loop {
        let n = tasks.len();
        for (id, plan, spec) in &rows {
            let spec: TaskSpec = serde_json::from_str(spec)?;
            for dep in &spec.depends_on {
                if rows.iter().any(|(up, p, s)| {
                    p == plan
                        && tasks.contains(up)
                        && serde_json::from_str::<TaskSpec>(s).is_ok_and(|s| s.id == *dep)
                }) {
                    tasks.insert(id.clone());
                }
            }
        }
        if n == tasks.len() {
            break;
        }
    }
    let mut out: Vec<_> = tasks.into_iter().collect();
    out.sort();
    Ok(out)
}
fn validate_change(
    db: &Connection,
    change: &LedgerChange,
    state: &std::path::Path,
    user: bool,
) -> Result<LedgerDiff> {
    let r = run(db, &change.conversation, &change.run_id)?;
    require(
        change.expected_spec_revision >= 0 && change.expected_spec_revision < i64::MAX,
        "invalid spec revision",
    )?;
    require(
        r.spec_revision == change.expected_spec_revision,
        "spec revision conflict; reload before saving; draft preserved",
    )?;
    require(
        !change.operation_id.trim().is_empty() && !change.entries.is_empty(),
        "operation identity and changes are required",
    )?;
    let before = entries(db, &r.id, r.spec_revision)?;
    let mut after: HashMap<String, EntrySpec> = before
        .iter()
        .map(|e| (e.spec.id.clone(), e.spec.clone()))
        .collect();
    let mut changed = HashSet::new();
    for e in &change.entries {
        require(changed.insert(e.id.clone()), "duplicate change identity")?;
        if after.contains_key(&e.id) {
            // A user decision is required for every semantic edit to an existing obligation.
            require(
                user,
                "agents may propose semantic changes but only the user may apply them",
            )?;
            require(
                !change.reason.trim().is_empty(),
                "scope edit requires a user rationale",
            )?;
        } else {
            require(
                e.disposition == Disposition::Active,
                "new obligation must be active",
            )?;
        }
        let source = source_text(db, &r.id, &e.source.source_id, state)?;
        e.source.text(&source)?;
        after.insert(e.id.clone(), e.clone());
    }
    let all: Vec<_> = after.into_values().collect();
    checked_graph(&all)?;
    let affected_tasks = impact(db, &r.id, &changed, &all)?;
    Ok(LedgerDiff {
        run_id: r.id,
        from_revision: r.spec_revision,
        to_revision: r.spec_revision + 1,
        before: before
            .into_iter()
            .filter(|e| changed.contains(&e.spec.id))
            .collect(),
        after: change.entries.clone(),
        affected_tasks,
    })
}

impl Store {
    pub fn create_run(&self, conversation: &str, goal: &str) -> Result<Run> {
        self.conversation(conversation)?;
        require(!goal.trim().is_empty(), "run goal is empty")?;
        let r = Run {
            id: crate::id(),
            conversation_id: conversation.into(),
            goal: goal.into(),
            spec_revision: 0,
            state: "active".into(),
            start_message: {
                let db = self.db();
                let exists: bool = db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM runs WHERE conversation_id=?1)",
                    [conversation],
                    |r| r.get(0),
                )?;
                db.query_row(if exists {"SELECT coalesce(max(seq),0) FROM messages WHERE conversation_id=?1 AND author='user'"} else {"SELECT coalesce(min(seq),0) FROM messages WHERE conversation_id=?1 AND author='user'"},[conversation],|r|r.get::<_,i64>(0))? as u64
            },
        };
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO runs(id,conversation_id,goal,created_ms,start_message) VALUES(?1,?2,?3,?4,?5)",
            params![r.id, conversation, goal, now_ms(),r.start_message as i64],
        )?;
        event(
            &tx,
            conversation,
            "run_created",
            &serde_json::json!({"run":r.id}),
        )?;
        tx.commit()?;
        Ok(r)
    }
    pub fn run(&self, conversation: &str, id: &str) -> Result<Run> {
        run(&self.db(), conversation, id)
    }
    pub fn planning_run(&self, conversation: &str) -> Result<Option<Run>> {
        let db = self.db();
        let id: Option<String> = db
            .query_row(
                "SELECT run_id FROM planning_runs WHERE conversation_id=?1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?;
        let id = match id {
            Some(id) => Some(id),
            None => db.query_row(
                "SELECT r.id FROM runs r WHERE r.conversation_id=?1 ORDER BY EXISTS(SELECT 1 FROM ledger_sources s WHERE s.run_id=r.id) DESC,r.created_ms DESC,r.rowid DESC LIMIT 1",
                [conversation], |row| row.get(0)
            ).optional()?,
        };
        id.map(|id| run(&db, conversation, &id)).transpose()
    }

    pub fn select_planning_run(&self, conversation: &str, run_id: &str) -> Result<()> {
        let db = self.db();
        run(&db, conversation, run_id)?;
        db.execute("INSERT INTO planning_runs VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET run_id=excluded.run_id", params![conversation,run_id])?;
        Ok(())
    }

    pub fn approved_requirements_producer(
        &self,
        conversation: &str,
        run_id: &str,
        revision: i64,
    ) -> Result<Option<String>> {
        self.run(conversation, run_id)?;
        Ok(self.db().query_row(
            "SELECT substr(p.author,6) FROM ledger_proposals p JOIN ledger_decisions d ON d.operation_id=json_extract(p.request,'$.operation_id') JOIN requirements_resumptions x ON x.operation_id=d.operation_id WHERE p.run_id=?1 AND d.spec_revision=?2 AND d.author='user' AND p.author LIKE 'turn:%' AND NOT EXISTS(SELECT 1 FROM ledger_reviews v WHERE v.run_id=p.run_id AND v.spec_revision=?2) ORDER BY d.created_ms DESC,d.rowid DESC LIMIT 1",
            params![run_id,revision],|row|row.get(0)
        ).optional()?)
    }

    pub fn has_pending_requirements_approval(&self, conversation: &str) -> Result<bool> {
        Ok(self.db().query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE conversation_id=?1 AND author IN ('requirements-approval','options-planner','options-plan-repair','plan-format-repair') AND seq>COALESCE((SELECT MAX(upto_message) FROM turns WHERE conversation_id=?1 AND role IN ('orchestrator','comment-orchestrator') AND status<>'interrupted'),0))",
            [conversation], |row| row.get(0)
        )?)
    }

    pub fn recover_approved_requirements(&self) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let changes = tx.prepare(
            "SELECT p.request FROM ledger_proposals p JOIN ledger_decisions d ON d.operation_id=json_extract(p.request,'$.operation_id') JOIN runs r ON r.id=p.run_id JOIN session_workflow w ON w.conversation_id=r.conversation_id WHERE p.author LIKE 'turn:%' AND d.author='user' AND d.spec_revision=r.spec_revision AND w.phase='planning' AND NOT EXISTS(SELECT 1 FROM requirements_resumptions x WHERE x.operation_id=d.operation_id) AND NOT EXISTS(SELECT 1 FROM ledger_reviews v WHERE v.run_id=r.id AND v.spec_revision=r.spec_revision) AND NOT EXISTS(SELECT 1 FROM runs newer WHERE newer.conversation_id=r.conversation_id AND (newer.created_ms>r.created_ms OR (newer.created_ms=r.created_ms AND newer.rowid>r.rowid)) AND EXISTS(SELECT 1 FROM ledger_sources s WHERE s.run_id=newer.id))"
        )?.query_map([], |row| row.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        for encoded in changes {
            let change: LedgerChange = serde_json::from_str(&encoded)?;
            enqueue_requirements_resume(&tx, &change)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn latest_run(&self, conversation: &str) -> Result<Option<Run>> {
        let id:Option<String>=self.db().query_row("SELECT id FROM runs WHERE conversation_id=?1 ORDER BY created_ms DESC,rowid DESC LIMIT 1",[conversation],|r|r.get(0)).optional()?;
        id.map(|id| self.run(conversation, &id)).transpose()
    }
    pub fn plan_run(&self, plan: &str) -> Result<Option<String>> {
        Ok(self
            .db()
            .query_row(
                "SELECT run_id FROM plan_runs WHERE plan_id=?1",
                [plan],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn link_plan_run(&self, conversation: &str, plan: &str, run_id: &str) -> Result<()> {
        self.run(conversation, run_id)?;
        require(
            self.plan(plan)?
                .iter()
                .all(|t| self.task_in_plan(&t.plan_id, &t.id).unwrap_or(false)),
            "unknown plan",
        )?;
        let db = self.db();
        let owner: String = db.query_row(
            "SELECT conversation_id FROM plans WHERE id=?1",
            [plan],
            |r| r.get(0),
        )?;
        require(
            owner == conversation,
            "plan belongs to another conversation",
        )?;
        db.execute(
            "INSERT INTO plan_runs(plan_id,run_id) VALUES(?1,?2)",
            params![plan, run_id],
        )?;
        Ok(())
    }
    pub fn register_message_source(
        &self,
        conversation: &str,
        run_id: &str,
        message: u64,
    ) -> Result<Source> {
        let mut db = self.db();
        let tx = db.transaction()?;
        run(&tx, conversation, run_id)?;
        let text: String = tx.query_row(
            "SELECT text FROM messages WHERE seq=?1 AND conversation_id=?2 AND author='user'",
            params![message as i64, conversation],
            |r| r.get(0),
        )?;
        let text = self.decode_content(&text)?;
        let id = format!("message:{message}");
        let source = Source {
            id: format!("{run_id}:{id}"),
            run_id: run_id.into(),
            message: Some(message),
            locator: None,
            hash: crate::hash(text.as_bytes()),
        };
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO ledger_sources(id,run_id,message,hash) VALUES(?1,?2,?3,?4)",
            params![source.id, run_id, message as i64, source.hash],
        )?;
        if inserted > 0 {
            event(
                &tx,
                conversation,
                "ledger_source_added",
                &serde_json::json!({"run":run_id,"source":source.id,"hash":source.hash}),
            )?;
        }
        tx.commit()?;
        Ok(source)
    }
    /// Import an immutable source artifact; callers pass the private state directory.
    pub fn register_document_source(
        &self,
        conversation: &str,
        run_id: &str,
        text: &str,
        state: &std::path::Path,
    ) -> Result<Source> {
        self.run(conversation, run_id)?;
        let hash = crate::hash(text.as_bytes());
        let locator = format!("sources/{hash}.txt");
        super::super::daemon::private_directory(&state.join("sources"))?;
        let path = state.join(&locator);
        use std::io::Write;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(text.as_bytes())?;
                file.sync_all()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                require(
                    crate::hash(std::fs::read(&path)?) == hash,
                    "source artifact hash mismatch",
                )?;
            }
            Err(error) => return Err(error.into()),
        }
        let source = Source {
            id: format!("{run_id}:{hash}"),
            run_id: run_id.into(),
            message: None,
            locator: Some(locator),
            hash,
        };
        let mut db = self.db();
        let tx = db.transaction()?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO ledger_sources(id,run_id,locator,hash) VALUES(?1,?2,?3,?4)",
            params![source.id, run_id, source.locator, source.hash],
        )?;
        if inserted > 0 {
            event(
                &tx,
                conversation,
                "ledger_source_added",
                &serde_json::json!({"run":run_id,"source":source.id,"hash":source.hash}),
            )?;
        }
        tx.commit()?;
        Ok(source)
    }
    pub fn ledger_sources(&self, conversation: &str, run_id: &str) -> Result<Vec<Source>> {
        let db = self.db();
        run(&db, conversation, run_id)?;
        Ok(db
            .prepare(
                "SELECT id,message,locator,hash FROM ledger_sources WHERE run_id=?1 ORDER BY id",
            )?
            .query_map([run_id], |r| {
                Ok(Source {
                    id: r.get(0)?,
                    run_id: run_id.into(),
                    message: r.get::<_, Option<i64>>(1)?.map(|n| n as u64),
                    locator: r.get(2)?,
                    hash: r.get(3)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?)
    }
    pub fn ledger_source_text(
        &self,
        conversation: &str,
        run_id: &str,
        source: &str,
        state: &std::path::Path,
    ) -> Result<String> {
        let db = self.db();
        run(&db, conversation, run_id)?;
        source_text(&db, run_id, source, state)
    }
    pub fn ledger_entries(
        &self,
        conversation: &str,
        run_id: &str,
        revision: Option<i64>,
    ) -> Result<Vec<LedgerEntry>> {
        let db = self.db();
        let r = run(&db, conversation, run_id)?;
        let revision = revision.unwrap_or(r.spec_revision);
        require(revision <= r.spec_revision, "unknown spec revision")?;
        entries(&db, run_id, revision)
    }
    pub fn preview_ledger_change(
        &self,
        change: &LedgerChange,
        state: &std::path::Path,
    ) -> Result<LedgerDiff> {
        validate_change(&self.db(), change, state, true)
    }
    pub fn propose_ledger_change(
        &self,
        change: &LedgerChange,
        state: &std::path::Path,
    ) -> Result<(String, LedgerDiff)> {
        self.propose_ledger_change_as(change, state, "user")
    }
    pub fn propose_agent_ledger_change(
        &self,
        change: &LedgerChange,
        state: &std::path::Path,
        turn: &str,
    ) -> Result<(String, LedgerDiff)> {
        self.check_analyst_author(change, turn)?;
        self.propose_ledger_change_as(change, state, &format!("turn:{turn}"))
    }
    fn propose_ledger_change_as(
        &self,
        change: &LedgerChange,
        state: &std::path::Path,
        author: &str,
    ) -> Result<(String, LedgerDiff)> {
        let mut db = self.db();
        let tx = db.transaction()?;
        if let Some((id,request))=tx.query_row("SELECT id,request FROM ledger_proposals WHERE json_extract(request,'$.operation_id')=?1",[&change.operation_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()? {
            require(serde_json::from_str::<LedgerChange>(&request)?==*change,"operation identity reused with a different proposal")?;
            return Ok((id.clone(),saved_proposal_diff(&tx,&id,change)?));
        }
        let diff = validate_change(&tx, change, state, true)?;
        let id = crate::id();
        let hash = super::diff_hash(&diff)?;
        tx.execute("INSERT INTO ledger_proposals(id,run_id,request,diff_hash,created_ms,author) VALUES(?1,?2,?3,?4,?5,?6)",params![id,change.run_id,serde_json::to_string(change)?,hash,now_ms(),author])?;
        for task in &diff.affected_tasks {
            tx.execute(
                "INSERT INTO ledger_proposal_tasks VALUES(?1,?2)",
                params![id, task],
            )?;
        }
        event(
            &tx,
            &change.conversation,
            "ledger_change_proposed",
            &serde_json::json!({"proposal":id,"run":change.run_id,"revision":change.expected_spec_revision,"author":author}),
        )?;
        tx.commit()?;
        Ok((id, diff))
    }
    pub fn apply_ledger_change(
        &self,
        change: &LedgerChange,
        state: &std::path::Path,
        user: bool,
    ) -> Result<ChangeResult> {
        require(
            user,
            "agent changes require a runtime-bound analyst session",
        )?;
        self.apply_ledger_change_as(change, state, true, "user")
    }
    pub fn apply_analyst_ledger_change(
        &self,
        change: &LedgerChange,
        state: &std::path::Path,
        turn: &str,
    ) -> Result<ChangeResult> {
        self.check_analyst_author(change, turn)?;
        self.apply_ledger_change_as(change, state, false, &format!("turn:{turn}"))
    }
    fn apply_ledger_change_as(
        &self,
        change: &LedgerChange,
        state: &std::path::Path,
        user: bool,
        author: &str,
    ) -> Result<ChangeResult> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let request_hash = crate::hash(serde_json::to_vec(change)?);
        if let Some((hash, result)) = tx
            .query_row(
                "SELECT request_hash,result FROM ledger_decisions WHERE operation_id=?1",
                [&change.operation_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        {
            require(
                hash == request_hash,
                "operation identity reused with a different payload",
            )?;
            return Ok(serde_json::from_str(&result)?);
        }
        let diff = validate_change(&tx, change, state, user)?;
        let mut changed_versions = Vec::new();
        for e in &change.entries {
            let text = source_text(&tx, &change.run_id, &e.source.source_id, state)?;
            e.source.text(&text)?;
            // The immutable artifact/message owns the source bytes. Version rows
            // retain the span, never duplicate a repository document.
            let original = "";
            tx.execute(
                "INSERT OR IGNORE INTO ledger_entries(run_id,id) VALUES(?1,?2)",
                params![change.run_id, e.id],
            )?;
            let version:i64=tx.query_row("SELECT coalesce(max(version),0)+1 FROM ledger_versions WHERE run_id=?1 AND entry_id=?2",params![change.run_id,e.id],|r|r.get(0))?;
            changed_versions.push(serde_json::json!({"id":e.id,"version":version}));
            let disposition = serde_json::to_value(e.disposition)?
                .as_str()
                .unwrap()
                .to_owned();
            tx.execute(
                "INSERT INTO ledger_versions VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    change.run_id,
                    e.id,
                    version,
                    diff.to_revision,
                    e.source.source_id,
                    e.source.start as i64,
                    e.source.end as i64,
                    original,
                    e.interpretation,
                    e.mandatory,
                    e.position,
                    disposition
                ],
            )?;
            for c in &e.criteria {
                tx.execute(
                    "INSERT INTO ledger_criteria VALUES(?1,?2,?3,?4,?5,?6,?7)",
                    params![
                        change.run_id,
                        e.id,
                        version,
                        c.id,
                        c.text,
                        c.mandatory,
                        c.integration
                    ],
                )?;
            }
            for (kind, values) in [
                ("parent", e.parent.iter().cloned().collect()),
                ("dependency", e.depends_on.clone()),
                ("task", e.task_ids.clone()),
                ("qualifier", e.qualifiers.clone()),
                ("implementer", e.implementation_owners.clone()),
                ("verifier", e.verification_owners.clone()),
            ] {
                for value in values {
                    tx.execute(
                        "INSERT INTO ledger_relations VALUES(?1,?2,?3,?4,?5)",
                        params![change.run_id, e.id, version, kind, value],
                    )?;
                }
            }
        }
        for task in &diff.affected_tasks {
            tx.execute(
                "UPDATE task_inputs SET generation=generation+1 WHERE task_id=?1",
                [task],
            )?;
            tx.execute("UPDATE tasks SET status='stale' WHERE id=?1", [task])?;
        }
        tx.execute(
            "UPDATE runs SET spec_revision=?2,state='active' WHERE id=?1",
            params![change.run_id, diff.to_revision],
        )?;
        let result = ChangeResult {
            run_id: change.run_id.clone(),
            spec_revision: diff.to_revision,
            affected_tasks: diff.affected_tasks,
        };
        tx.execute(
            "INSERT INTO ledger_decisions VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                change.operation_id,
                change.run_id,
                result.spec_revision,
                author,
                change.reason,
                request_hash,
                serde_json::to_string(&result)?,
                now_ms()
            ],
        )?;
        event(
            &tx,
            &change.conversation,
            "ledger_changed",
            &serde_json::json!({"run":result.run_id,"revision":result.spec_revision,"entries":changed_versions,"affected_tasks":result.affected_tasks}),
        )?;
        if user {
            let analyst_proposal: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM ledger_proposals WHERE run_id=?1 AND json_extract(request,'$.operation_id')=?2 AND author LIKE 'turn:%')",
                params![change.run_id,change.operation_id], |row| row.get(0)
            )?;
            if analyst_proposal {
                enqueue_requirements_resume(&tx, change)?;
            }
        }
        tx.commit()?;
        Ok(result)
    }
    pub fn ledger_diff(
        &self,
        conversation: &str,
        run_id: &str,
        from: i64,
        to: i64,
    ) -> Result<LedgerDiff> {
        let before = self.ledger_entries(conversation, run_id, Some(from))?;
        let after = self.ledger_entries(conversation, run_id, Some(to))?;
        let changed: HashSet<_> = after
            .iter()
            .filter(|e| {
                !before
                    .iter()
                    .any(|b| b.spec.id == e.spec.id && b.version == e.version)
            })
            .map(|e| e.spec.id.clone())
            .collect();
        let all: Vec<_> = after.iter().map(|e| e.spec.clone()).collect();
        Ok(LedgerDiff {
            run_id: run_id.into(),
            from_revision: from,
            to_revision: to,
            before: before
                .into_iter()
                .filter(|e| changed.contains(&e.spec.id))
                .collect(),
            after: after
                .into_iter()
                .filter(|e| changed.contains(&e.spec.id))
                .map(|e| e.spec)
                .collect(),
            affected_tasks: impact(&self.db(), run_id, &changed, &all)?,
        })
    }
}
fn event(
    db: &Connection,
    conversation: &str,
    kind: &str,
    payload: &serde_json::Value,
) -> Result<()> {
    db.execute(
        "INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,?2,?3,?4)",
        params![conversation, kind, payload.to_string(), now_ms()],
    )?;
    Ok(())
}

fn independent(db: &Connection, producer: &str, verifier: &str) -> Result<()> {
    let session = |id: &str| -> Result<(String, Option<String>, String, String)> {
        Ok(db.query_row(
            "SELECT role,session_id,status,conversation_id FROM turns WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?)
    };
    let (p, ps, pstatus, pc) = session(producer)?;
    let (v, vs, vstatus, vc) = session(verifier)?;
    require(
        producer != verifier && p != v && pc == vc && (ps.is_none() || vs.is_none() || ps != vs),
        "self verification is forbidden",
    )?;
    require(
        pstatus == "done" && vstatus == "done",
        "evidence requires completed sessions",
    )?;
    Ok(())
}
impl Store {
    pub fn task_generation(&self, task: &str) -> Result<i64> {
        Ok(self
            .db()
            .query_row(
                "SELECT generation FROM task_inputs WHERE task_id=?1",
                [task],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }
    pub fn assert_task_inputs(&self, task: &crate::lean::store::Task) -> Result<()> {
        require(
            self.task_generation(&task.id)? == task.input_generation,
            "stale task generation; result retained in turn history",
        )?;
        let db = self.db();
        let bound: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM task_inputs i JOIN task_ledger_inputs l ON l.task_id=i.task_id WHERE i.task_id=?1)", [&task.id], |r|r.get(0))?;
        require(bound, "integrity error: task has no requirement inputs")?;
        let stale:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM task_ledger_inputs l JOIN task_inputs i ON i.task_id=l.task_id WHERE l.task_id=?1 AND l.version<>(SELECT max(v.version) FROM ledger_versions v WHERE v.run_id=i.run_id AND v.entry_id=l.entry_id))",[&task.id],|r|r.get(0))?;
        require(!stale, "task ledger inputs changed; replan before resuming")
    }
    /// Bind actual per-entry versions. Global revision changes do not invalidate unrelated tasks.
    pub fn bind_task_ledger(&self, conversation: &str, run_id: &str, plan: &str) -> Result<()> {
        let all = self.ledger_entries(conversation, run_id, None)?;
        let tasks = self.plan(plan)?;
        let mut covered = HashSet::new();
        let prior = self.run_tasks(conversation, run_id)?;
        for task in prior
            .iter()
            .filter(|t| t.plan_id != plan && t.status == "approved")
        {
            if self.assert_task_inputs(task).is_ok() {
                let spec: TaskSpec = serde_json::from_value(task.spec.clone())?;
                covered.extend(spec.criterion_ids);
            }
        }
        let mut db = self.db();
        let tx = db.transaction()?;
        for t in &tasks {
            let spec: TaskSpec = serde_json::from_value(t.spec.clone())?;
            super::validate_task_links(&spec, &all)?;
            covered.extend(spec.criterion_ids.iter().cloned());
            let ids: HashSet<_> = spec.requirement_ids.iter().collect();
            tx.execute("INSERT INTO task_inputs(task_id,run_id) VALUES(?1,?2) ON CONFLICT(task_id) DO UPDATE SET run_id=excluded.run_id",params![t.id,run_id])?;
            tx.execute("DELETE FROM task_ledger_inputs WHERE task_id=?1", [&t.id])?;
            for id in ids {
                let e = all
                    .iter()
                    .find(|e| e.spec.id == *id)
                    .ok_or_else(|| invalid("unknown ledger task input"))?;
                tx.execute(
                    "INSERT INTO task_ledger_inputs VALUES(?1,?2,?3)",
                    params![t.id, id, e.version],
                )?;
            }
        }
        for e in &all {
            if e.spec.disposition != Disposition::Active {
                continue;
            }
            for assigned in &e.spec.task_ids {
                require(
                    tasks
                        .iter()
                        .chain(prior.iter().filter(|t| t.status == "approved"))
                        .any(|t| {
                            (t.id == *assigned || t.spec["id"].as_str() == Some(assigned))
                                && serde_json::from_value::<TaskSpec>(t.spec.clone())
                                    .is_ok_and(|s| s.requirement_ids.contains(&e.spec.id))
                        }),
                    &format!(
                        "ledger assignment {} -> {assigned} is not honored by the task plan",
                        e.spec.id
                    ),
                )?;
            }
        }
        super::validate_criterion_coverage(&all, &covered)?;
        tx.commit()?;
        Ok(())
    }
    #[allow(clippy::too_many_arguments)] // Session and input identities are validated together.
    pub fn record_ledger_review(
        &self,
        conversation: &str,
        run_id: &str,
        revision: i64,
        kind: &str,
        producer: &str,
        verifier: &str,
        fingerprint: &str,
        approved: bool,
        note: &str,
    ) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let r = run(&tx, conversation, run_id)?;
        require(
            r.spec_revision == revision,
            "review refers to an old spec revision",
        )?;
        require(
            matches!(kind, "extraction" | "audit" | "options"),
            "unknown ledger review kind",
        )?;
        require(!note.trim().is_empty(), "review needs an assessment")?;
        independent(&tx, producer, verifier)?;
        let bound: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM ledger_turns p JOIN ledger_turns v ON p.run_id=v.run_id WHERE p.turn_id=?1 AND v.turn_id=?2 AND p.run_id=?3 AND v.spec_revision=?4 AND p.phase=?5 AND v.phase=?5)",params![producer,verifier,run_id,revision,kind],|r|r.get(0))?;
        require(
            bound,
            "review sessions are not bound to this run revision and phase",
        )?;
        let uncovered: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM ledger_sources s WHERE s.run_id=?1 AND (NOT EXISTS(SELECT 1 FROM ledger_source_coverage c WHERE c.turn_id=?2 AND c.source_id=s.id) OR NOT EXISTS(SELECT 1 FROM ledger_source_coverage c WHERE c.turn_id=?3 AND c.source_id=s.id)))",params![run_id,producer,verifier],|r|r.get(0))?;
        require(
            !uncovered,
            "source set changed during review; repeat against all sources",
        )?;
        let (pc, pr): (String, String) = tx.query_row(
            "SELECT conversation_id,role FROM turns WHERE id=?1",
            [producer],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let vr: String = tx.query_row("SELECT role FROM turns WHERE id=?1", [verifier], |r| {
            r.get(0)
        })?;
        require(
            pc == conversation
                && pr == format!("requirements-{kind}-analyst")
                && vr == format!("requirements-{kind}-verifier"),
            "review author is not authorized for this run and phase",
        )?;
        tx.execute(
            "INSERT INTO ledger_reviews(run_id,spec_revision,kind,producer_turn,verifier_turn,fingerprint,approved,note,created_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                run_id,
                revision,
                kind,
                producer,
                verifier,
                fingerprint,
                approved,
                note,
                now_ms()
            ],
        )?;
        event(
            &tx,
            conversation,
            "ledger_review",
            &serde_json::json!({"run":run_id,"revision":revision,"kind":kind,"approved":approved}),
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn extraction_current(&self, conversation: &str, run_id: &str) -> Result<bool> {
        let db = self.db();
        let r = run(&db, conversation, run_id)?;
        Ok(db.query_row("SELECT approved FROM ledger_reviews WHERE run_id=?1 AND spec_revision=?2 AND kind='extraction' AND NOT EXISTS(SELECT 1 FROM ledger_sources s WHERE s.run_id=?1 AND NOT EXISTS(SELECT 1 FROM ledger_source_coverage c WHERE c.turn_id=ledger_reviews.verifier_turn AND c.source_id=s.id)) ORDER BY created_ms DESC,rowid DESC LIMIT 1",params![run_id,r.spec_revision],|r|r.get(0)).optional()?.unwrap_or(false))
    }
    pub fn add_ledger_evidence(
        &self,
        conversation: &str,
        run_id: &str,
        verifier_turn: &str,
        assessment: &Assessment,
    ) -> Result<Evidence> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let r = run(&tx, conversation, run_id)?;
        let p = &assessment.evidence;
        require(
            !p.result.trim().is_empty()
                && !p.integrated_fingerprint.trim().is_empty()
                && !assessment.note.trim().is_empty(),
            "empty evidence or assessment",
        )?;
        require(
            !matches!(
                p.result.trim().to_ascii_lowercase().as_str(),
                "done" | "completed" | "ok"
            ),
            "completion assertion is not evidence",
        )?;
        independent(&tx, &p.producer_turn, verifier_turn)?;
        let bound: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM ledger_turns p JOIN ledger_turns v ON p.run_id=v.run_id WHERE p.turn_id=?1 AND v.turn_id=?2 AND p.run_id=?3 AND p.spec_revision=?4 AND v.spec_revision=?4 AND p.phase='audit' AND v.phase='audit')",params![p.producer_turn,verifier_turn,run_id,r.spec_revision],|r|r.get(0))?;
        require(
            bound,
            "evidence sessions are not bound to the active run revision",
        )?;
        let role: String = tx.query_row(
            "SELECT role FROM turns WHERE id=?1 AND conversation_id=?2",
            params![verifier_turn, conversation],
            |r| r.get(0),
        )?;
        require(
            role == "requirements-audit-verifier",
            "only integrated-result verifier may assess completion evidence",
        )?;
        let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM task_inputs i JOIN tasks t ON t.id=i.task_id JOIN turns u ON u.id=?4 WHERE i.task_id=?1 AND i.run_id=?2 AND i.generation=?3 AND t.round=?5 AND t.status='approved' AND u.conversation_id=?6 AND u.role='requirements-audit-analyst')",params![p.task_id,run_id,p.generation,p.producer_turn,p.attempt,conversation],|r|r.get(0))?;
        require(
            valid,
            "evidence has stale task inputs, attempt, result, or unauthorized producer",
        )?;
        let all = entries(&tx, run_id, r.spec_revision)?;
        let e = all
            .iter()
            .find(|e| {
                e.spec.id == p.entry_id
                    && e.version == p.entry_version
                    && e.spec.disposition == Disposition::Active
            })
            .ok_or_else(|| invalid("evidence refers to an old or inactive entry"))?;
        require(
            e.spec.criteria.iter().any(|c| c.id == p.criterion_id),
            "unknown evidence criterion",
        )?;
        let linked:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM task_ledger_inputs WHERE task_id=?1 AND entry_id=?2 AND version=?3)",params![p.task_id,p.entry_id,p.entry_version],|r|r.get(0))?;
        require(linked, "evidence task does not implement this obligation")?;
        {
            let spec: String =
                tx.query_row("SELECT spec FROM tasks WHERE id=?1", [&p.task_id], |r| {
                    r.get(0)
                })?;
            require(
                serde_json::from_str::<TaskSpec>(&spec)?
                    .criterion_ids
                    .contains(&p.criterion_id),
                "evidence task does not implement this criterion",
            )?;
        }
        let evidence = Evidence {
            id: crate::id(),
            run_id: run_id.into(),
            proposal: p.clone(),
            verifier_turn: verifier_turn.into(),
            assessment: assessment.note.clone(),
            approved: assessment.approved,
        };
        tx.execute("INSERT INTO ledger_evidence VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",params![evidence.id,run_id,p.entry_id,p.entry_version,p.criterion_id,p.task_id,p.generation,p.attempt,p.producer_turn,verifier_turn,p.result,p.artifact,p.integrated_fingerprint,assessment.note,assessment.approved,now_ms()])?;
        event(
            &tx,
            conversation,
            "ledger_evidence",
            &serde_json::json!({"run":run_id,"entry":p.entry_id,"version":p.entry_version,"criterion":p.criterion_id,"evidence":evidence.id,"approved":assessment.approved}),
        )?;
        tx.commit()?;
        Ok(evidence)
    }
    pub fn ledger_evidence(
        &self,
        conversation: &str,
        run_id: &str,
        entry: Option<&str>,
    ) -> Result<Vec<Evidence>> {
        let db = self.db();
        run(&db, conversation, run_id)?;
        Ok(db.prepare("SELECT id,entry_id,entry_version,criterion_id,task_id,generation,attempt,producer_turn,verifier_turn,result,artifact,fingerprint,assessment,approved FROM ledger_evidence WHERE run_id=?1 AND (?2 IS NULL OR entry_id=?2) ORDER BY created_ms,rowid")?.query_map(params![run_id,entry],|r|Ok(Evidence{id:r.get(0)?,run_id:run_id.into(),proposal:EvidenceProposal{entry_id:r.get(1)?,entry_version:r.get(2)?,criterion_id:r.get(3)?,task_id:r.get(4)?,generation:r.get(5)?,attempt:r.get(6)?,producer_turn:r.get(7)?,result:r.get(9)?,artifact:r.get(10)?,integrated_fingerprint:r.get(11)?},verifier_turn:r.get(8)?,assessment:r.get(12)?,approved:r.get(13)?}))?.collect::<std::result::Result<_,_>>()?)
    }
    pub fn ledger_statuses(
        &self,
        conversation: &str,
        run_id: &str,
        revision: Option<i64>,
        fingerprint: &str,
    ) -> Result<Vec<LedgerEntry>> {
        let mut all = self.ledger_entries(conversation, run_id, revision)?;
        let evidence = self.ledger_evidence(conversation, run_id, None)?;
        let db = self.db();
        let task_rows=db.prepare("SELECT t.id,t.spec,t.status,t.round,i.generation,t.plan_id FROM tasks t JOIN task_inputs i ON i.task_id=t.id WHERE i.run_id=?1 ORDER BY t.created_ms DESC,t.rowid DESC")?.query_map([run_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,u32>(3)?,r.get::<_,i64>(4)?,r.get::<_,String>(5)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        for e in &mut all {
            if e.spec.disposition != Disposition::Active {
                e.status = serde_json::to_value(e.spec.disposition)?
                    .as_str()
                    .unwrap()
                    .into();
                continue;
            }
            let mut linked: Vec<_> = task_rows
                .iter()
                .filter(|(_, s, _, _, _, _)| {
                    serde_json::from_str::<TaskSpec>(s)
                        .is_ok_and(|s| s.requirement_ids.contains(&e.spec.id))
                })
                .collect();
            if let Some(latest_plan) = linked.first().map(|t| t.5.clone()) {
                linked.retain(|t| t.5 == latest_plan);
            }
            let verified = e.spec.criteria.iter().any(|c| c.mandatory)
                && e.spec.criteria.iter().filter(|c| c.mandatory).all(|c| {
                    let mut current = evidence
                        .iter()
                        .filter(|ev| {
                            let p = &ev.proposal;
                            p.entry_id == e.spec.id
                                && p.entry_version == e.version
                                && p.criterion_id == c.id
                                && p.integrated_fingerprint == fingerprint
                                && task_rows
                                    .iter()
                                    .any(|(id, _, status, round, generation, _)| {
                                        *id == p.task_id
                                            && *generation == p.generation
                                            && *round == p.attempt
                                            && status == "approved"
                                    })
                        })
                        .peekable();
                    current.peek().is_some() && current.all(|ev| ev.approved)
                });
            e.status = if verified {
                "verified"
            } else if linked.iter().any(|(_, _, s, _, _, _)| s == "stale") {
                "stale"
            } else if !linked.is_empty() && linked.iter().all(|(_, _, s, _, _, _)| s == "approved")
            {
                "implemented"
            } else if linked
                .iter()
                .any(|(_, _, s, _, _, _)| matches!(s.as_str(), "working" | "verifying"))
            {
                "in_progress"
            } else if linked.iter().any(|(_, _, s, _, _, _)| {
                matches!(
                    s.as_str(),
                    "failed" | "needs_user" | "cancelled" | "waiting"
                )
            }) {
                "blocked"
            } else if !linked.is_empty() {
                "planned"
            } else if e.spec.criteria.is_empty() {
                "clarification_needed"
            } else {
                "identified"
            }
            .into();
        }
        // Fixed point: neither children nor prerequisites can be bypassed by a parent verdict.
        loop {
            let invalid: Vec<_> = all
                .iter()
                .filter(|e| {
                    e.status == "verified"
                        && all.iter().any(|c| {
                            c.spec.disposition == Disposition::Active
                                && ((c.spec.mandatory
                                    && c.spec.parent.as_deref() == Some(&e.spec.id))
                                    || e.spec.depends_on.contains(&c.spec.id))
                                && c.status != "verified"
                        })
                })
                .map(|e| e.spec.id.clone())
                .collect();
            if invalid.is_empty() {
                break;
            }
            for e in &mut all {
                if invalid.contains(&e.spec.id) {
                    e.status = "implemented".into();
                }
            }
        }
        Ok(all)
    }
    pub fn list_ledger(&self, query: &LedgerQuery, fingerprint: &str) -> Result<LedgerPage> {
        let run = self.run(&query.conversation, &query.run_id)?;
        let linked_ids: Option<HashSet<String>> = if let Some(task) = &query.task {
            Some(self.db().prepare("SELECT l.entry_id FROM task_ledger_inputs l JOIN task_inputs i ON i.task_id=l.task_id JOIN tasks t ON t.id=i.task_id WHERE i.run_id=?1 AND (t.id=?2 OR json_extract(t.spec,'$.id')=?2)")?.query_map(params![query.run_id,task],|r|r.get(0))?.collect::<std::result::Result<_,_>>()?)
        } else {
            None
        };
        let entries: Vec<_> = self
            .ledger_statuses(
                &query.conversation,
                &query.run_id,
                query.revision,
                fingerprint,
            )?
            .into_iter()
            .filter(|e| {
                query.status.as_ref().is_none_or(|s| *s == e.status)
                    && (!query.missing_coverage
                        || !matches!(e.status.as_str(), "verified" | "superseded_by_user"))
                    && linked_ids
                        .as_ref()
                        .is_none_or(|ids| ids.contains(&e.spec.id))
            })
            .collect();
        let total = entries.len();
        let limit = query.limit.unwrap_or(50).clamp(1, 200);
        Ok(LedgerPage {
            run,
            entries: entries.into_iter().skip(query.offset).take(limit).collect(),
            total,
            offset: query.offset,
            has_more: query.offset.saturating_add(limit) < total,
        })
    }
    pub fn requirements_gaps(
        &self,
        conversation: &str,
        run_id: &str,
        fingerprint: &str,
    ) -> Result<Vec<String>> {
        let r = self.run(conversation, run_id)?;
        let mut gaps = Vec::new();
        if !self.extraction_current(conversation, run_id)? {
            gaps.push(
                "source coverage has not been independently verified for this revision".into(),
            );
        }
        let all = self.ledger_statuses(conversation, run_id, None, fingerprint)?;
        if all.is_empty() {
            gaps.push("ledger is empty".into());
        }
        for e in &all {
            if e.spec.disposition == Disposition::Active
                && e.spec.mandatory
                && e.status != "verified"
            {
                gaps.push(format!(
                    "{}: {} (current integrated evidence required)",
                    e.spec.id, e.status
                ));
            }
        }
        let db = self.db();
        let audit:bool=db.query_row("SELECT approved FROM ledger_reviews WHERE run_id=?1 AND spec_revision=?2 AND kind='audit' AND fingerprint=?3 ORDER BY created_ms DESC,rowid DESC LIMIT 1",params![run_id,r.spec_revision,fingerprint],|r|r.get(0)).optional()?.unwrap_or(false);
        if !audit {
            gaps.push(
                "integrated compliance audit and independent verification are missing or stale"
                    .into(),
            );
        }
        Ok(gaps)
    }
}
impl Store {
    pub fn check_ledger_proposal(&self, id: &str, hash: &str, change: &LedgerChange) -> Result<()> {
        let db = self.db();
        let (request, expected): (String, String) = db.query_row(
            "SELECT request,diff_hash FROM ledger_proposals WHERE id=?1 AND run_id=?2",
            params![id, change.run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        require(
            expected == hash && serde_json::from_str::<LedgerChange>(&request)? == *change,
            "approval must identify the exact proposed ledger diff",
        )
    }
}
impl Store {
    pub fn ledger_entry_history(
        &self,
        conversation: &str,
        run_id: &str,
        id: &str,
    ) -> Result<Vec<LedgerEntry>> {
        let db = self.db();
        run(&db, conversation, run_id)?;
        let revisions:Vec<i64>=db.prepare("SELECT spec_revision FROM ledger_versions WHERE run_id=?1 AND entry_id=?2 ORDER BY version")?.query_map(params![run_id,id],|r|r.get(0))?.collect::<std::result::Result<_,_>>()?;
        let mut out = Vec::new();
        for revision in revisions {
            if let Some(e) = entries(&db, run_id, revision)?
                .into_iter()
                .find(|e| e.spec.id == id)
            {
                out.push(e);
            }
        }
        Ok(out)
    }
}
impl Store {
    pub fn bind_ledger_turn(
        &self,
        conversation: &str,
        run_id: &str,
        turn: &str,
        revision: i64,
        phase: &str,
        sources: &[String],
    ) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let r = run(&tx, conversation, run_id)?;
        require(
            revision <= r.spec_revision && revision >= 0,
            "invalid ledger turn revision",
        )?;
        let owner: String = tx.query_row(
            "SELECT conversation_id FROM turns WHERE id=?1",
            [turn],
            |r| r.get(0),
        )?;
        require(
            owner == conversation,
            "turn belongs to another conversation",
        )?;
        tx.execute(
            "INSERT INTO ledger_turns VALUES(?1,?2,?3,?4)",
            params![turn, run_id, revision, phase],
        )?;
        for source in sources {
            let belongs: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM ledger_sources WHERE id=?1 AND run_id=?2)",
                params![source, run_id],
                |r| r.get(0),
            )?;
            require(belongs, "source belongs to another run")?;
            tx.execute(
                "INSERT INTO ledger_source_coverage VALUES(?1,?2)",
                params![turn, source],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
impl Store {
    pub fn task_has_ledger_inputs(&self, task: &str) -> Result<bool> {
        Ok(self.db().query_row(
            "SELECT EXISTS(SELECT 1 FROM task_inputs WHERE task_id=?1)",
            [task],
            |r| r.get(0),
        )?)
    }
    pub fn set_run_completion(
        &self,
        conversation: &str,
        run_id: &str,
        revision: i64,
        complete: bool,
    ) -> Result<()> {
        let db = self.db();
        let r = run(&db, conversation, run_id)?;
        require(
            r.spec_revision == revision,
            "scope changed before completion",
        )?;
        db.execute(
            "UPDATE runs SET state=?2 WHERE id=?1",
            params![run_id, if complete { "complete" } else { "active" }],
        )?;
        Ok(())
    }
}
impl Store {
    pub fn applied_ledger_change(&self, change: &LedgerChange) -> Result<Option<ChangeResult>> {
        let row: Option<(String, String)> = self
            .db()
            .query_row(
                "SELECT request_hash,result FROM ledger_decisions WHERE operation_id=?1",
                [&change.operation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(hash, result)| {
            require(
                hash == crate::hash(serde_json::to_vec(change)?),
                "operation identity reused with a different payload",
            )?;
            Ok(serde_json::from_str(&result)?)
        })
        .transpose()
    }
    pub fn list_runs(&self, conversation: &str) -> Result<Vec<Run>> {
        self.conversation(conversation)?;
        let db = self.db();
        let ids = db
            .prepare(
                "SELECT id FROM runs WHERE conversation_id=?1 ORDER BY created_ms DESC,rowid DESC",
            )?
            .query_map([conversation], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.iter().map(|id| run(&db, conversation, id)).collect()
    }
    pub fn latest_ledger_proposal(
        &self,
        conversation: &str,
        run_id: &str,
    ) -> Result<serde_json::Value> {
        let db = self.db();
        run(&db, conversation, run_id)?;
        let (id,request,hash):(String,String,String)=db.query_row("SELECT p.id,p.request,p.diff_hash FROM ledger_proposals p WHERE p.run_id=?1 AND NOT EXISTS(SELECT 1 FROM ledger_decisions d WHERE d.operation_id=json_extract(p.request,'$.operation_id')) ORDER BY p.created_ms DESC,p.rowid DESC LIMIT 1",[run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let change: LedgerChange = serde_json::from_str(&request)?;
        let diff = saved_proposal_diff(&db, &id, &change)?;
        require(
            super::diff_hash(&diff)? == hash,
            "proposal impact changed; prepare a new proposal from the preserved changes",
        )?;
        Ok(
            serde_json::json!({"change":change,"proposal":{"proposal_id":id,"diff_hash":hash,"diff":diff}}),
        )
    }
}
fn saved_proposal_diff(db: &Connection, id: &str, change: &LedgerChange) -> Result<LedgerDiff> {
    let before = entries(db, &change.run_id, change.expected_spec_revision)?
        .into_iter()
        .filter(|e| change.entries.iter().any(|new| new.id == e.spec.id))
        .collect();
    let affected_tasks = db
        .prepare("SELECT task_id FROM ledger_proposal_tasks WHERE proposal_id=?1 ORDER BY task_id")?
        .query_map([id], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(LedgerDiff {
        run_id: change.run_id.clone(),
        from_revision: change.expected_spec_revision,
        to_revision: change.expected_spec_revision + 1,
        before,
        after: change.entries.clone(),
        affected_tasks,
    })
}
impl Store {
    fn check_analyst_author(&self, change: &LedgerChange, turn: &str) -> Result<()> {
        let db = self.db();
        run(&db, &change.conversation, &change.run_id)?;
        let authorized:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM ledger_turns l JOIN turns t ON t.id=l.turn_id WHERE l.turn_id=?1 AND l.run_id=?2 AND l.spec_revision=?3 AND l.phase='extraction' AND t.role='requirements-extraction-analyst' AND t.conversation_id=?4 AND t.status='done')",params![turn,change.run_id,change.expected_spec_revision,change.conversation],|r|r.get(0))?;
        require(
            authorized,
            "ledger author is not the runtime-bound requirements analyst",
        )
    }
}
impl Store {
    pub fn run_tasks(
        &self,
        conversation: &str,
        run_id: &str,
    ) -> Result<Vec<crate::lean::store::Task>> {
        let plans = {
            let db = self.db();
            run(&db, conversation, run_id)?;
            db.prepare("SELECT plan_id FROM plan_runs WHERE run_id=?1 ORDER BY rowid")?
                .query_map([run_id], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut tasks = Vec::new();
        for plan in plans {
            tasks.extend(self.plan(&plan)?);
        }
        Ok(tasks)
    }
}
impl Store {
    pub fn ledger_entry_tasks(
        &self,
        conversation: &str,
        run_id: &str,
        entry: &str,
    ) -> Result<Vec<serde_json::Value>> {
        let db = self.db();
        run(&db, conversation, run_id)?;
        Ok(db.prepare("SELECT t.id,t.plan_id,json_extract(t.spec,'$.id'),json_extract(t.spec,'$.title'),t.status,i.generation,l.version FROM task_ledger_inputs l JOIN task_inputs i ON i.task_id=l.task_id JOIN tasks t ON t.id=i.task_id WHERE i.run_id=?1 AND l.entry_id=?2 ORDER BY t.created_ms DESC,t.rowid DESC")?.query_map(params![run_id,entry],|r|Ok(serde_json::json!({"id":r.get::<_,String>(0)?,"plan":r.get::<_,String>(1)?,"task":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?,"status":r.get::<_,String>(4)?,"generation":r.get::<_,i64>(5)?,"entry_version":r.get::<_,i64>(6)?})))?.collect::<std::result::Result<_,_>>()?)
    }
}

fn enqueue_requirements_resume(db: &Connection, change: &LedgerChange) -> Result<()> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM requirements_resumptions WHERE operation_id=?1)",
        [&change.operation_id],
        |row| row.get(0),
    )?;
    if exists {
        return Ok(());
    }
    db.execute("INSERT INTO planning_runs VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET run_id=excluded.run_id", params![change.conversation,change.run_id])?;
    let text = format!(
        "The user approved the exact requirements change for run {}. Continue independent requirements verification and prepare the plan for review using this run. This approves requirements only; implementation still requires separate plan approval.",
        change.run_id
    );
    db.execute("INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'requirements-approval',?2,?3)", params![change.conversation,text,now_ms()])?;
    let seq = db.last_insert_rowid();
    db.execute(
        "INSERT INTO requirements_resumptions VALUES(?1,?2)",
        params![change.operation_id, seq],
    )?;
    event(
        db,
        &change.conversation,
        "message",
        &serde_json::json!({"seq":seq,"author":"requirements-approval","text":text}),
    )?;
    event(
        db,
        &change.conversation,
        "planning_progress",
        &serde_json::json!({"text":"Requirements approved. Resuming verification and planning."}),
    )?;
    Ok(())
}
