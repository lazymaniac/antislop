use super::*;

pub(super) struct EntryRow {
    id: String,
    version: i64,
    revision: i64,
    source_id: String,
    start: usize,
    end: usize,
    interpretation: String,
    mandatory: bool,
    position: u32,
    disposition: String,
}

struct EntryRelations {
    qualifiers: Vec<String>,
    parent: Option<String>,
    dependencies: Vec<String>,
    tasks: Vec<String>,
    implementers: Vec<String>,
    verifiers: Vec<String>,
}

pub(super) fn entries(db: &Connection, run: &str, revision: i64) -> Result<Vec<LedgerEntry>> {
    let rows = entry_rows(db, run, revision)?;
    let mut sources = HashMap::<String, String>::new();
    rows.into_iter()
        .map(|row| entry_from_row(db, run, row, &mut sources))
        .collect()
}

fn entry_rows(db: &Connection, run: &str, revision: i64) -> Result<Vec<EntryRow>> {
    let mut stmt = db.prepare("SELECT v.entry_id,v.version,v.spec_revision,v.source_id,v.start_byte,v.end_byte,v.interpretation,v.mandatory,v.position,v.disposition
      FROM ledger_versions v JOIN ledger_entries e ON e.run_id=v.run_id AND e.id=v.entry_id
      WHERE v.run_id=?1 AND v.spec_revision<=?2 AND v.version=(SELECT max(w.version) FROM ledger_versions w WHERE w.run_id=v.run_id AND w.entry_id=v.entry_id AND w.spec_revision<=?2) ORDER BY v.position,v.entry_id")?;
    Ok(stmt
        .query_map(params![run, revision], |row| {
            Ok(EntryRow {
                id: row.get(0)?,
                version: row.get(1)?,
                revision: row.get(2)?,
                source_id: row.get(3)?,
                start: row.get::<_, i64>(4)? as usize,
                end: row.get::<_, i64>(5)? as usize,
                interpretation: row.get(6)?,
                mandatory: row.get(7)?,
                position: row.get(8)?,
                disposition: row.get(9)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?)
}

fn entry_from_row(
    db: &Connection,
    run: &str,
    row: EntryRow,
    sources: &mut HashMap<String, String>,
) -> Result<LedgerEntry> {
    let spec = entry_spec(db, run, &row, sources)?;
    let version = row.version;
    let revision = row.revision;
    let span = SourceSpan {
        source_id: row.source_id,
        start: row.start,
        end: row.end,
    };
    let original = source_text_for_span(db, run, &span, sources)?.to_owned();
    Ok(LedgerEntry {
        run_id: run.into(),
        version,
        spec_revision: revision,
        original,
        status: "identified".into(),
        spec,
    })
}

fn entry_spec(
    db: &Connection,
    run: &str,
    row: &EntryRow,
    sources: &mut HashMap<String, String>,
) -> Result<EntrySpec> {
    let source = SourceSpan {
        source_id: row.source_id.clone(),
        start: row.start,
        end: row.end,
    };
    source_text_for_span(db, run, &source, sources)?;
    let relations = entry_relations(db, run, &row.id, row.version)?;
    Ok(EntrySpec {
        source,
        interpretation: row.interpretation.clone(),
        mandatory: row.mandatory,
        position: row.position,
        disposition: serde_json::from_value(row.disposition.clone().into())?,
        criteria: entry_criteria(db, run, &row.id, row.version)?,
        qualifiers: relations.qualifiers,
        parent: relations.parent,
        depends_on: relations.dependencies,
        task_ids: relations.tasks,
        implementation_owners: relations.implementers,
        verification_owners: relations.verifiers,
        id: row.id.clone(),
    })
}

fn entry_relations(db: &Connection, run: &str, id: &str, version: i64) -> Result<EntryRelations> {
    Ok(EntryRelations {
        qualifiers: relations(db, run, id, version, "qualifier")?,
        parent: relations(db, run, id, version, "parent")?
            .into_iter()
            .next(),
        dependencies: relations(db, run, id, version, "dependency")?,
        tasks: relations(db, run, id, version, "task")?,
        implementers: relations(db, run, id, version, "implementer")?,
        verifiers: relations(db, run, id, version, "verifier")?,
    })
}

fn source_text_for_span<'a>(
    db: &Connection,
    run: &str,
    span: &SourceSpan,
    sources: &'a mut HashMap<String, String>,
) -> Result<&'a str> {
    if !sources.contains_key(&span.source_id) {
        let state = std::path::Path::new(db.path().unwrap_or(""))
            .parent()
            .ok_or_else(|| invalid("ledger database has no source directory"))?;
        sources.insert(
            span.source_id.clone(),
            source_text(db, run, &span.source_id, state)?,
        );
    }
    span.text(&sources[&span.source_id])
}

fn entry_criteria(db: &Connection, run: &str, id: &str, version: i64) -> Result<Vec<Criterion>> {
    Ok(db.prepare("SELECT id,text,mandatory,integration FROM ledger_criteria WHERE run_id=?1 AND entry_id=?2 AND version=?3 ORDER BY id")?
        .query_map(params![run,id,version], |row| Ok(Criterion { id: row.get(0)?, text: row.get(1)?, mandatory: row.get(2)?, integration: row.get(3)? }))?
        .collect::<std::result::Result<_, _>>()?)
}
