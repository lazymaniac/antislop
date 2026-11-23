use super::super::agent::Spec;
use super::super::mcp::{Gateway, SERVER};
use super::Launch;
use crate::config::ProviderKind;
use crate::{Error, Result};
use portable_pty::CommandBuilder;
use serde_json::{Value, json};
mod agy;
pub(crate) use agy::agy_ready;
mod claude;
mod codex;
#[path = "../common.rs"]
mod common;
pub(crate) mod gemini;
mod opencode;
pub(crate) mod pi;
use common::runtime_args;

const TOKEN_ENV: &str = "ANTISLOP_MCP_TOKEN";
const PARENT_SESSION_ENV: [&str; 7] = [
    "CLAUDECODE",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_BRIDGE_SESSION_ID",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_SESSION_ATTENDED",
];

pub(crate) fn command(spec: &Spec, gateway: &Gateway) -> Result<Launch> {
    let mut command = base_command(spec);
    if spec.provider == ProviderKind::Pi {
        let (session, pi) = pi::prepare(&mut command, spec, gateway)?;
        return Ok(Launch {
            command,
            session: Some(session),
            pi: Some(pi),
        });
    }
    let session = configure(&mut command, spec, gateway)?;
    Ok(Launch {
        command,
        session,
        pi: None,
    })
}

fn base_command(spec: &Spec) -> CommandBuilder {
    let mut command = CommandBuilder::new(&spec.binary);
    command.cwd(&spec.cwd);
    command.env("TERM", "xterm-256color");
    command
}

fn configure(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
) -> Result<Option<String>> {
    match spec.provider {
        ProviderKind::Claude => configure_claude(command, spec, gateway),
        ProviderKind::Codex => configure_codex(command, spec, gateway),
        ProviderKind::Opencode => configure_opencode(command, spec, gateway),
        ProviderKind::Agy => configure_agy(command, spec, gateway),
        ProviderKind::Gemini => configure_gemini(command, spec, gateway),
        ProviderKind::Pi => Err(Error::Invalid(
            "Pi must use its extension launch path".into(),
        )),
    }
}

fn configure_claude(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
) -> Result<Option<String>> {
    command.args(runtime_args(&spec.args, spec.provider));
    Ok(claude::claude(command, spec, gateway))
}

fn configure_codex(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
) -> Result<Option<String>> {
    command.args(runtime_args(&spec.args, spec.provider));
    codex::codex(command, spec, gateway);
    Ok(spec.resume.clone())
}

fn configure_opencode(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
) -> Result<Option<String>> {
    command.args(runtime_args(&spec.args, spec.provider));
    opencode::opencode(command, spec, gateway)?;
    Ok(spec.resume.clone())
}

fn configure_agy(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
) -> Result<Option<String>> {
    command.args(runtime_args(&spec.args, spec.provider));
    agy::agy(command, spec, gateway)?;
    Ok(spec.resume.clone())
}

fn configure_gemini(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
) -> Result<Option<String>> {
    command.args(runtime_args(&spec.args, spec.provider));
    gemini::gemini(command, spec, gateway)?;
    Ok(spec.resume.clone())
}
