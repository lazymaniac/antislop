use super::*;
use rusqlite::{Connection, OptionalExtension, params};

const COLUMNS: &str = "id,executable,build,project,config,launch_path,task,conversation,state,error,signature,generation,config_revision,log,diagnostic_tail";

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Activation> {
    let state: String = row.get(8)?;
    let state = serde_json::from_value(json!(state)).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(Activation {
        id: row.get(0)?,
        executable: PathBuf::from(row.get::<_, String>(1)?),
        build: row.get(2)?,
        project: PathBuf::from(row.get::<_, String>(3)?),
        config: PathBuf::from(row.get::<_, String>(4)?),
        path: OsString::from(row.get::<_, String>(5)?),
        task: row.get(6)?,
        conversation: row.get(7)?,
        state,
        error: row.get(9)?,
        signature: row.get(10)?,
        generation: row.get(11)?,
        config_revision: row.get(12)?,
        log: row.get(13)?,
        diagnostic_tail: row.get(14)?,
        resume: Vec::new(),
    })
}

pub(super) fn load(db: &Connection, id: Option<&str>, active: bool) -> Result<Option<Activation>> {
    let table: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='runtime_activations')",[],|r|r.get(0))?;
    if !table {
        return Ok(None);
    }
    let condition = if id.is_some() {
        "id=?1"
    } else if active {
        "state IN ('validating','waiting_for_idle','switching')"
    } else {
        "1=1"
    };
    let sql = format!(
        "SELECT {COLUMNS} FROM runtime_activations WHERE {condition} ORDER BY created_ms DESC,rowid DESC LIMIT 1"
    );
    let mut operation = if let Some(id) = id {
        db.query_row(&sql, [id], row).optional()?
    } else {
        db.query_row(&sql, [], row).optional()?
    };
    if let Some(operation) = &mut operation {
        operation.resume = db.prepare("SELECT conversation,plan,cancelled FROM runtime_activation_conversations WHERE operation=?1 ORDER BY rowid")?.query_map([&operation.id],|r|Ok(Resume{conversation:r.get(0)?,plan:r.get(1)?,cancelled:r.get(2)?}))?.collect::<std::result::Result<_,_>>()?;
    }
    Ok(operation)
}

pub(super) fn save(core: &Core, operation: &Activation) -> Result<()> {
    let mut db = core.store.db();
    let tx = db.transaction()?;
    tx.execute("INSERT INTO runtime_activations(id,executable,build,project,config,launch_path,task,conversation,state,error,signature,generation,config_revision,log,diagnostic_tail,created_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16) ON CONFLICT(id) DO UPDATE SET state=excluded.state,error=excluded.error,log=excluded.log,diagnostic_tail=excluded.diagnostic_tail",
        params![operation.id,operation.executable.to_string_lossy(),operation.build,operation.project.to_string_lossy(),operation.config.to_string_lossy(),operation.path.to_string_lossy(),operation.task,operation.conversation,operation.state.name(),operation.error,operation.signature,operation.generation,operation.config_revision,operation.log,operation.diagnostic_tail,crate::lean::store::now_ms()])?;
    for item in &operation.resume {
        tx.execute("INSERT INTO runtime_activation_conversations(operation,conversation,plan,cancelled) VALUES(?1,?2,?3,?4) ON CONFLICT(operation,conversation) DO UPDATE SET cancelled=excluded.cancelled",params![operation.id,item.conversation,item.plan,item.cancelled])?;
    }
    tx.execute("INSERT INTO runtime_activation_events(operation,state,error,created_ms) VALUES(?1,?2,?3,?4)",params![operation.id,operation.state.name(),operation.error,crate::lean::store::now_ms()])?;
    tx.commit()?;
    Ok(())
}

/// Import legacy checkpoints only during startup, never during a status read.
pub(super) fn import(core: &Core) -> Result<Vec<Activation>> {
    let mut imported = Vec::new();
    for name in [LAST, PENDING] {
        let Some(mut operation) = legacy(&core.state.join(name))? else {
            continue;
        };
        if load(&core.store.db(), Some(&operation.id), false)?.is_none() {
            if operation.state == ActivationState::Validating {
                operation.state = ActivationState::Interrupted;
                operation.error =
                    Some("legacy activation interrupted; request validation again".into());
            }
            operation.generation = operation
                .task
                .as_deref()
                .map(|id| {
                    core.store
                        .task_by_id(id)
                        .map(|t| t.map(|t| t.input_generation))
                })
                .transpose()?
                .flatten();
            save(core, &operation)?;
        }
        if let Some(operation) = load(&core.store.db(), Some(&operation.id), false)? {
            imported.push(operation);
        }
        // A checkpoint cannot reappear as an active operation on a later restart.
        std::fs::rename(
            core.state.join(name),
            core.state.join(format!("{name}.imported")),
        )?;
    }
    Ok(imported)
}

pub(super) fn legacy(path: &Path) -> Result<Option<Activation>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(super) fn for_task(core: &Core, task: &crate::lean::store::Task) -> Result<Option<Activation>> {
    let db = core.store.db();
    let id: Option<String>=db.query_row("SELECT id FROM runtime_activations WHERE task=?1 AND generation=?2 ORDER BY created_ms DESC,rowid DESC LIMIT 1",params![task.id,task.input_generation],|r|r.get(0)).optional()?;
    match id {
        Some(id) => load(&db, Some(&id), false),
        None => Ok(None),
    }
}
