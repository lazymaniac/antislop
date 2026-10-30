//! A task's stall count outlives a restart: it is kept in the event log, one
//! record per rejected round, and a stop for coordination closes it.
use super::*;

impl Store {
    /// The run a task's latest progress record describes, unless a stop for coordination
    /// closed it since.
    pub(in crate::lean) fn task_progress(
        &self,
        conversation: &str,
        task: &str,
    ) -> Result<Option<Value>> {
        let payload: Option<String> = self
            .db()
            .query_row(
                "SELECT payload FROM events WHERE conversation_id=?1 AND kind='task_progress'
                 AND json_extract(payload,'$.task')=?2 ORDER BY seq DESC LIMIT 1",
                params![conversation, task],
                |row| row.get(0),
            )
            .optional()?;
        let record: Option<Value> = payload
            .map(|text| serde_json::from_str(&text))
            .transpose()?;
        Ok(record
            .map(|mut record| record["run"].take())
            .filter(|run| !run.is_null()))
    }

    pub(in crate::lean) fn record_task_progress(
        &self,
        conversation: &str,
        task: &Task,
        run: Option<&Value>,
    ) -> Result<()> {
        self.append_event(
            conversation,
            None,
            Some(&task.plan_id),
            "task_progress",
            &serde_json::json!({"task": task.id, "run": run}),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_progress_record_survives_reopening_until_a_stop_closes_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.db");
        let store = Store::open(&path).unwrap();
        let conversation = store.create_conversation("/p").unwrap().id;
        let task = store
            .add_plan(
                &conversation,
                &[serde_json::json!({"id":"a","title":"A","goal":"g","kind":"analysis"})],
            )
            .unwrap()
            .remove(0);
        assert_eq!(store.task_progress(&conversation, &task.id).unwrap(), None);
        let run = serde_json::json!({"since": 2});
        store
            .record_task_progress(&conversation, &task, Some(&run))
            .unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.task_progress(&conversation, &task.id).unwrap(),
            Some(run)
        );
        store
            .record_task_progress(&conversation, &task, None)
            .unwrap();
        assert_eq!(store.task_progress(&conversation, &task.id).unwrap(), None);
    }
}
