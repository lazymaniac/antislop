use super::*;

impl Store {
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
        crate::lean::daemon::private_directory(&state.join("sources"))?;
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
}
