//! Startup choices apply only to new conversations, never attached runs.
use crate::{Result, config::ProviderKind, require};
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub state: PathBuf,
    pub run_id: Option<String>,
    pub orchestrator: Option<ProviderKind>,
}
impl Options {
    pub fn parse(arguments: &[String]) -> Result<Self> {
        let mut positionals = Vec::new();
        let mut orchestrator = None;
        let mut arguments = arguments.iter();
        while let Some(argument) = arguments.next() {
            if argument == "--orchestrator" {
                require(orchestrator.is_none(), "duplicate --orchestrator")?;
                let value = arguments.next().ok_or_else(|| {
                    crate::Error::Invalid(
                        "--orchestrator requires codex, claude, gemini or agy".into(),
                    )
                })?;
                orchestrator = Some(match value.as_str() {
                    "codex" => ProviderKind::Codex,
                    "claude" => ProviderKind::Claude,
                    "gemini" => ProviderKind::Gemini,
                    "agy" => ProviderKind::Agy,
                    _ => {
                        return Err(crate::Error::Invalid(
                            "--orchestrator requires codex, claude, gemini or agy".into(),
                        ));
                    }
                });
            } else {
                require(!argument.starts_with('-'), "unknown UI option")?;
                require(!argument.is_empty(), "empty UI argument")?;
                positionals.push(argument);
            }
        }
        require(positionals.len() <= 2, "ui accepts [STATE_DIR] [RUN_ID]")?;
        let run_id = positionals.get(1).map(|s| (*s).clone());
        require(
            run_id.is_none() || orchestrator.is_none(),
            "--orchestrator selects a new conversation; it cannot change an attached run",
        )?;
        Ok(Self {
            state: positionals
                .first()
                .map(PathBuf::from)
                .unwrap_or_else(|| ".antislop".into()),
            run_id,
            orchestrator,
        })
    }
}
