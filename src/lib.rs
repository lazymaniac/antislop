pub mod config;
pub mod lean;
pub mod transport;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{}", sql_message(.0))]
    Sql(#[from] rusqlite::Error),
}

/// Agents and users see the cause, not a bare driver message.
fn sql_message(error: &rusqlite::Error) -> String {
    match error {
        rusqlite::Error::QueryReturnedNoRows => {
            "a record this step depends on no longer exists (the task or plan was replaced); \
             re-read the current state and retry"
                .into()
        }
        other => other.to_string(),
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error::Invalid(message.into()))
    }
}
pub fn hash(bytes: impl AsRef<[u8]>) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes.as_ref())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
