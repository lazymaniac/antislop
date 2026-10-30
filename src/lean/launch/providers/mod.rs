use super::super::agent::{Access, Spec};
use super::super::mcp::{Gateway, SERVER, TOOL};
use super::Launch;
use crate::config::ProviderKind;
use crate::{Error, Result};
use portable_pty::CommandBuilder;
use serde_json::{Value, json};
mod agy;
mod claude;
mod codex;
#[path = "../common.rs"]
mod common;
mod gemini;
mod opencode;
use common::strip_sandbox;

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
    let (session, settings) = configure(&mut command, spec, gateway)?;
    Ok(Launch {
        command,
        session,
        settings,
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
) -> Result<(Option<String>, Option<tempfile::NamedTempFile>)> {
    match spec.provider {
        ProviderKind::Claude => configure_claude(command, spec, gateway),
        ProviderKind::Codex => configure_codex(command, spec, gateway),
        ProviderKind::Opencode => configure_opencode(command, spec, gateway),
        ProviderKind::Agy => configure_agy(command, spec, gateway),
        ProviderKind::Gemini => configure_gemini(command, spec, gateway),
    }
}

fn configure_claude(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
) -> Result<(Option<String>, Option<tempfile::NamedTempFile>)> {
    command.args(&spec.args);
    Ok((
        claude::claude(command, spec, gateway, is_read_only(spec)),
        None,
    ))
}

fn configure_codex(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
) -> Result<(Option<String>, Option<tempfile::NamedTempFile>)> {
    command.args(strip_sandbox(&spec.args));
    codex::codex(command, spec, gateway, is_read_only(spec));
    Ok((spec.resume.clone(), None))
}

fn configure_opencode(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
) -> Result<(Option<String>, Option<tempfile::NamedTempFile>)> {
    command.args(&spec.args);
    opencode::opencode(command, spec, gateway, is_read_only(spec))?;
    Ok((spec.resume.clone(), None))
}

fn configure_agy(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
) -> Result<(Option<String>, Option<tempfile::NamedTempFile>)> {
    command.args(&spec.args);
    agy::agy(command, spec, gateway, is_read_only(spec))?;
    Ok((spec.resume.clone(), None))
}

fn configure_gemini(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
) -> Result<(Option<String>, Option<tempfile::NamedTempFile>)> {
    command.args(&spec.args);
    Ok((
        spec.resume.clone(),
        Some(gemini::gemini(command, spec, gateway, is_read_only(spec))?),
    ))
}

fn is_read_only(spec: &Spec) -> bool {
    spec.access == Access::ReadOnly
}
