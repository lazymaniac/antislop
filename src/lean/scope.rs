//! Immutable plan scope and original sources; acceptance checks live on tasks.
use crate::lean::{Core, Store, store};
use crate::{Result, require};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scope {
    pub id: String,
    pub conversation_id: String,
    pub summary: String,
    pub upto_message: u64,
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub hash: String,
    pub reference: String,
}

impl Store {
    pub fn create_scope(&self, conversation: &str, summary: &str) -> Result<Scope> {
        require(!summary.trim().is_empty(), "scope must not be empty")?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let upto = tx.query_row(
            "SELECT coalesce(max(seq),0) FROM messages WHERE conversation_id=?1",
            [conversation],
            |r| r.get::<_, i64>(0),
        )?;
        let scope = Scope {
            id: crate::id(),
            conversation_id: conversation.into(),
            summary: summary.into(),
            upto_message: upto as u64,
            source_ids: Vec::new(),
        };
        tx.execute(
            "INSERT INTO scopes VALUES(?1,?2,?3,?4,?5)",
            params![scope.id, conversation, summary, upto, store::now_ms()],
        )?;
        tx.execute("INSERT INTO scope_documents SELECT ?1,id FROM conversation_sources WHERE conversation_id=?2", params![scope.id,conversation])?;
        tx.commit()?;
        drop(db);
        self.scope(conversation, &scope.id)
    }

    pub fn scope(&self, conversation: &str, id: &str) -> Result<Scope> {
        let mut scope=self.db().query_row("SELECT id,conversation_id,summary,upto_message FROM scopes WHERE id=?1 AND conversation_id=?2",params![id,conversation],|r|Ok(Scope{id:r.get(0)?,conversation_id:r.get(1)?,summary:r.get(2)?,upto_message:r.get::<_,i64>(3)? as u64,source_ids:Vec::new()}))?;
        scope.source_ids = self
            .db()
            .prepare("SELECT source_id FROM scope_documents WHERE scope_id=?1 ORDER BY source_id")?
            .query_map([id], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        Ok(scope)
    }

    pub(in crate::lean) fn scope_documents_current(
        &self,
        conversation: &str,
        id: &str,
    ) -> Result<bool> {
        Ok(!self.db().query_row("SELECT EXISTS(SELECT 1 FROM conversation_sources s WHERE s.conversation_id=?1 AND NOT EXISTS(SELECT 1 FROM scope_documents d WHERE d.scope_id=?2 AND d.source_id=s.id))",params![conversation,id],|r|r.get::<_,bool>(0))?)
    }

    pub(in crate::lean) fn additional_source_context(
        &self,
        conversation: &str,
        scope: Option<&str>,
    ) -> Result<String> {
        let sources=self.db().prepare("SELECT id,reference FROM conversation_sources s WHERE conversation_id=?1 AND NOT EXISTS(SELECT 1 FROM scope_documents d WHERE d.scope_id=?2 AND d.source_id=s.id) ORDER BY created_ms,id")?.query_map(params![conversation,scope],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let mut text = String::new();
        for (id, reference) in sources {
            self.source_text(conversation, &id)?;
            let path = self.content.path(&serde_json::from_str(&reference)?)?;
            text.push_str(&format!("\nAdditional original document {id} at {}. Read it in full before planning. Its addition is accepted with the reviewed plan.\n",path.display()));
        }
        Ok(text)
    }

    pub fn plan_scope_id(&self, plan: &str) -> Result<Option<String>> {
        Ok(self
            .db()
            .query_row(
                "SELECT scope_id FROM plan_scopes WHERE plan_id=?1 ORDER BY revision DESC LIMIT 1",
                [plan],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn bind_plan_scope(&self, conversation: &str, plan: &str, scope: &str) -> Result<()> {
        self.scope(conversation, scope)?;
        require(
            self.plan_conversation(plan)? == conversation,
            "plan belongs to another conversation",
        )?;
        let revision = self.plan_revision(plan)?;
        self.db().execute(
            "INSERT INTO plan_scopes VALUES(?1,?2,?3)",
            params![plan, revision, scope],
        )?;
        Ok(())
    }

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

    pub fn assert_task_inputs(&self, task: &store::Task) -> Result<()> {
        require(
            self.db().query_row("SELECT i.generation FROM task_inputs i JOIN tasks t ON t.id=i.task_id WHERE t.id=?1 AND t.plan_id=?2",params![task.id,task.plan_id],|r|r.get::<_,i64>(0)).optional()? == Some(task.input_generation),
            "stale task generation; result retained in turn history",
        )
    }

    pub fn register_source(&self, conversation: &str, text: &str) -> Result<Source> {
        self.conversation(conversation)?;
        let _write = self.content_write.lock().expect("content write mutex");
        let hash = crate::hash(text.as_bytes());
        if let Some(source)=self.db().query_row("SELECT id,hash,reference FROM conversation_sources WHERE conversation_id=?1 AND hash=?2",params![conversation,hash],|r|Ok(Source{id:r.get(0)?,hash:r.get(1)?,reference:r.get(2)?})).optional()? {
            return Ok(source);
        }
        let source = Source {
            id: crate::id(),
            hash,
            reference: serde_json::to_string(&self.content.write(text)?)?,
        };
        self.db().execute(
            "INSERT INTO conversation_sources VALUES(?1,?2,?3,?4,?5)",
            params![
                source.id,
                conversation,
                source.reference,
                source.hash,
                store::now_ms()
            ],
        )?;
        Ok(source)
    }

    pub fn source_text(&self, conversation: &str, source: &str) -> Result<String> {
        let (reference, hash): (String, String) = self.db().query_row(
            "SELECT reference,hash FROM conversation_sources WHERE conversation_id=?1 AND id=?2",
            params![conversation, source],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let text = self.content.read(&serde_json::from_str(&reference)?)?;
        require(
            crate::hash(text.as_bytes()) == hash,
            "source artifact hash mismatch",
        )?;
        Ok(text)
    }

    pub fn scope_sources(&self, conversation: &str, scope: &str) -> Result<String> {
        let scope = self.scope(conversation, scope)?;
        let mut text = format!(
            "Plan scope (derived from original sources; acceptance belongs to the reviewed plan):\n{}\n\nOriginal conversation in chronological order; later explicit user corrections supersede earlier requests. Agent messages are context, not user authorization:\n",
            scope.summary
        );
        for message in self
            .messages_after(conversation, 0)?
            .into_iter()
            .filter(|m| {
                m.seq <= scope.upto_message
                    && (m.author == "user" || m.author == crate::lean::ORCHESTRATOR)
            })
        {
            text.push_str(&format!(
                "\n{} message {}:\n{}\n",
                message.author, message.seq, message.text
            ));
        }
        let sources=self.db().prepare("SELECT s.id,s.reference FROM conversation_sources s JOIN scope_documents d ON d.source_id=s.id WHERE d.scope_id=?1 ORDER BY s.created_ms,s.id")?.query_map([&scope.id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        for (id, reference) in sources {
            self.source_text(conversation, &id)?;
            let path = self.content.path(&serde_json::from_str(&reference)?)?;
            text.push_str(&format!(
                "\nRead the full original document {id} at {}. Do not rely on excerpts.\n",
                path.display()
            ));
        }
        Ok(text)
    }
}

impl Core {
    pub(in crate::lean) fn plan_scope_context(
        &self,
        conversation: &str,
        plan: &str,
    ) -> Result<String> {
        let id = self
            .store
            .plan_scope_id(plan)?
            .ok_or_else(|| crate::Error::Invalid("plan has no scope".into()))?;
        self.store.scope_sources(conversation, &id)
    }
}

impl Store {
    pub(in crate::lean) fn pending_scope_revision(&self, plan: &str) -> Result<Option<String>> {
        Ok(self.db().query_row("SELECT p.request FROM scope_revision_proposals p JOIN plan_revision_jobs j ON j.id=p.request WHERE j.plan_id=?1 AND p.state='awaiting_acceptance' AND j.status='awaiting_acceptance'",[plan],|r|r.get(0)).optional()?)
    }

    pub(in crate::lean) fn scope_revision_token(&self, request: &str) -> Result<String> {
        Ok(self.db().query_row("SELECT fingerprint FROM scope_revision_proposals WHERE request=?1 AND state='awaiting_acceptance'",[request],|r|r.get(0))?)
    }

    pub(in crate::lean) fn scope_holds_task(&self, task: &str) -> Result<bool> {
        Ok(self.db().query_row("SELECT EXISTS(SELECT 1 FROM scope_revision_proposals p JOIN json_each(p.affected) a WHERE p.state='awaiting_acceptance' AND a.value=?1)",[task],|r|r.get(0))?)
    }
}

#[cfg(test)]
#[path = "scope/tests.rs"]
mod tests;
