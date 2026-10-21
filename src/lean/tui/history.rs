//! Project-local input history. Each submission adds one small SQLite row;
//! concurrent frontends append independently instead of overwriting a snapshot.

use crate::Result;
use rusqlite::{Connection, params};
use std::path::Path;

pub const LIMIT: usize = 1000;

fn open(path: &Path) -> Result<Connection> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(std::time::Duration::from_millis(100))?;
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS input_history(
             id INTEGER PRIMARY KEY, text TEXT NOT NULL);",
    )?;
    Ok(connection)
}

pub fn load(path: &Path) -> Result<Vec<String>> {
    let connection = open(path)?;
    let mut statement = connection.prepare(
        "SELECT text FROM (SELECT id,text FROM input_history ORDER BY id DESC LIMIT ?1)
         ORDER BY id",
    )?;
    let rows = statement.query_map([LIMIT as i64], |row| row.get(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub fn append(path: &Path, text: &str) -> Result<()> {
    let mut connection = open(path)?;
    let transaction = connection.transaction()?;
    transaction.execute("INSERT INTO input_history(text) VALUES(?1)", params![text])?;
    transaction.execute(
        "DELETE FROM input_history WHERE id <= (
             SELECT id FROM input_history ORDER BY id DESC LIMIT 1 OFFSET ?1)",
        [LIMIT as i64],
    )?;
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_latest_thousand_submissions_are_kept_in_order() {
        let state = tempfile::tempdir().unwrap();
        let path = state.path().join("history.db");
        for index in 0..LIMIT + 2 {
            append(&path, &format!("message {index}")).unwrap();
        }
        let entries = load(&path).unwrap();
        assert_eq!(entries.len(), LIMIT);
        assert_eq!(entries.first().unwrap(), "message 2");
        assert_eq!(entries.last().unwrap(), &format!("message {}", LIMIT + 1));
        let connection = open(&path).unwrap();
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM input_history", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, LIMIT as i64);
    }
}
