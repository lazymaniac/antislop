pub mod comments;
pub mod communication;
pub mod config;
pub mod context;
pub mod domain;
pub mod drive;
pub mod ipc;
pub mod lean;
pub mod orchestration;
pub mod policy;
pub mod process;
pub mod providers;
pub mod quality;
pub mod recovery;
pub mod runtime;
pub mod sessions;
pub mod storage;
pub mod team;
pub mod transport;
pub mod tui;
pub mod verification;
pub mod workspace;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Provider(#[from] providers::ProviderFailure),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
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
