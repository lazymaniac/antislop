//! Compact provenance and frozen input versions for untracked documents.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::lean) struct ArtifactReceipt {
    pub task: String,
    pub generation: i64,
    pub commit: String,
}

impl Store {
    pub(in crate::lean) fn artifact_receipt(
        &self,
        conversation: &str,
        path: &str,
    ) -> Result<Option<ArtifactReceipt>> {
        Ok(self.db().query_row("SELECT task_id,generation,result_commit FROM artifact_deliveries WHERE conversation_id=?1 AND path=?2",params![conversation,path],|r| Ok(ArtifactReceipt{task:r.get(0)?,generation:r.get(1)?,commit:r.get(2)?})).optional()?)
    }

    pub(in crate::lean) fn record_artifact_delivery(
        &self,
        conversation: &str,
        path: &str,
        receipt: &ArtifactReceipt,
    ) -> Result<()> {
        crate::require(
            self.db().query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND conversation_id=?2)",
                params![receipt.task, conversation],
                |r| r.get::<_, bool>(0),
            )?,
            "artifact producer belongs to another conversation",
        )?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let changed=tx.execute("INSERT INTO artifact_deliveries VALUES(?1,?2,?3,?4,?5) ON CONFLICT(conversation_id,path) DO UPDATE SET task_id=excluded.task_id,generation=excluded.generation,result_commit=excluded.result_commit WHERE task_id<>excluded.task_id OR generation<>excluded.generation OR result_commit<>excluded.result_commit",params![conversation,path,receipt.task,receipt.generation,receipt.commit])?;
        if changed > 0 {
            tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) SELECT ?1,plan_id,'artifact_delivery_recorded',?3,?4 FROM tasks WHERE id=?2",params![conversation,receipt.task,serde_json::json!({"path":path,"producer":receipt.task,"generation":receipt.generation,"commit":receipt.commit}).to_string(),now_ms()])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(in crate::lean) fn artifact_input(
        &self,
        task: &Task,
        path: &str,
    ) -> Result<Option<Option<ArtifactReceipt>>> {
        Ok(self.db().query_row("SELECT source_task,source_generation,source_commit FROM task_artifact_inputs WHERE task_id=?1 AND generation=?2 AND path=?3",params![task.id,task.input_generation,path],|r| {
            let source: Option<String> = r.get(0)?;
            source.map(|source| Ok(ArtifactReceipt{task:source,generation:r.get(1)?,commit:r.get(2)?})).transpose()
        }).optional()?)
    }

    pub(in crate::lean) fn freeze_artifact_input(
        &self,
        task: &Task,
        path: &str,
        source: Option<&ArtifactReceipt>,
    ) -> Result<Option<ArtifactReceipt>> {
        self.db().execute(
            "INSERT OR IGNORE INTO task_artifact_inputs VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                task.id,
                task.input_generation,
                path,
                source.map(|s| &s.task),
                source.map(|s| s.generation),
                source.map(|s| &s.commit)
            ],
        )?;
        self.artifact_input(task, path)?
            .ok_or_else(|| Error::Invalid("artifact input was not recorded".into()))
    }
}
