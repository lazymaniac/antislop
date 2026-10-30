//! `antislop chat`: talk to the orchestrator. In a terminal without a message
//! it opens the TUI; otherwise it prints events line by line. Either way the
//! project's daemon does the work, so it goes on after the frontend exits.
//! Lines typed while agents work are accepted at once and answered in the
//! orchestrator's next turn. `/accept` starts the proposed plan, `/resume`
//! runs again what the last plan left unfinished, `/cancel` stops running
//! work.

use super::daemon::{Client, Request};
use crate::{Result, require};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

mod chat;
mod presentation;
pub use presentation::{describe, review_lines};

/// Select the configuration without creating files or directories.
/// An explicit path or an existing project file never falls back on error.
pub fn resolve_config(project: &Path, explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return checked_config(&project.join(path));
    }
    let local = project.join("antislop.toml");
    match std::fs::symlink_metadata(&local) {
        Ok(_) => return checked_config(&local),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let global = super::trust::config_dir()?.join("antislop.toml");
    match std::fs::symlink_metadata(&global) {
        Ok(_) => checked_config(&global),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(crate::Error::Invalid(format!(
                "no configuration at {} or {} (use --config FILE to select another file)",
                local.display(),
                global.display()
            )))
        }
        Err(error) => Err(error.into()),
    }
}

fn checked_config(path: &Path) -> Result<PathBuf> {
    path.canonicalize().map_err(|error| {
        crate::Error::Invalid(format!("configuration {}: {error}", path.display()))
    })
}

pub struct Options {
    pub project: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
    pub conversation: Option<String>,
    pub new: bool,
    pub trust: bool,
    pub message: Option<String>,
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Self> {
        let project = std::env::current_dir()?.canonicalize()?;
        let parsed = ParsedArgs::parse(args)?;
        let config = resolve_config(&project, parsed.config.as_deref())?;
        // Nothing is created yet: the directory may not be trusted.
        let state = parsed.state.unwrap_or_else(|| project.join(".antislop"));
        let state = if state.is_absolute() {
            state
        } else {
            project.join(state)
        };
        Ok(Self {
            project,
            config,
            state,
            conversation: parsed.conversation,
            new: parsed.new,
            trust: parsed.trust,
            message: (!parsed.words.is_empty()).then(|| parsed.words.join(" ")),
        })
    }

    /// Creates the private state directory. Called once the working
    /// directory is trusted, so that asking leaves nothing behind.
    pub fn prepare(&mut self) -> Result<()> {
        super::daemon::private_directory(&self.state)?;
        self.state = self.state.canonicalize()?;
        Ok(())
    }

    /// The conversation to show: the one asked for, else the latest for this
    /// project unless `--new`, else a new one.
    pub fn conversation(&self, client: &Client) -> Result<String> {
        if let Some(id) = &self.conversation {
            return Ok(id.clone());
        }
        if !self.new
            && let Some(id) = self.latest(client)?
        {
            return Ok(id);
        }
        Ok(client.call(&Request::Control(
            crate::lean::daemon::ControlRequest::Start,
        ))?["id"]
            .as_str()
            .unwrap_or_default()
            .to_owned())
    }
    /// This project's latest conversation.
    pub fn latest(&self, client: &Client) -> Result<Option<String>> {
        let project = self.project.to_string_lossy();
        Ok(client
            .call(&Request::Control(
                crate::lean::daemon::ControlRequest::Conversations,
            ))?
            .as_array()
            .into_iter()
            .flatten()
            .filter(|conversation| conversation["project"] == project.as_ref())
            .max_by_key(|conversation| conversation["created_ms"].as_i64())
            .and_then(|conversation| conversation["id"].as_str().map(str::to_owned)))
    }
}

#[derive(Default)]
struct ParsedArgs {
    config: Option<PathBuf>,
    state: Option<PathBuf>,
    conversation: Option<String>,
    new: bool,
    trust: bool,
    words: Vec<String>,
}

impl ParsedArgs {
    fn parse(args: &[String]) -> Result<Self> {
        let mut parsed = Self::default();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            parsed.apply(arg, &mut args)?;
        }
        Ok(parsed)
    }

    fn apply(&mut self, arg: &str, args: &mut std::slice::Iter<'_, String>) -> Result<()> {
        match arg {
            "--config" => self.config = Some(PathBuf::from(option_value(arg, args)?)),
            "--state" => self.state = Some(PathBuf::from(option_value(arg, args)?)),
            "--conversation" => self.conversation = Some(option_value(arg, args)?),
            "--new" => self.new = true,
            "--trust" => self.trust = true,
            flag if flag.starts_with("--") => {
                return Err(crate::Error::Invalid(format!("unknown option {flag}")));
            }
            word => self.words.push(word.to_owned()),
        }
        Ok(())
    }
}

fn option_value(flag: &str, args: &mut std::slice::Iter<'_, String>) -> Result<String> {
    args.next()
        .cloned()
        .ok_or_else(|| crate::Error::Invalid(format!("{flag} requires a value")))
}

/// Makes sure the user let antislop work in `project`: it is remembered, or
/// `--trust` says so, or the user answers now. Without a terminal and
/// without `--trust` there is nobody to ask, so the answer is no.
pub fn ensure_trusted<R: BufRead, W: Write>(
    registry: &super::trust::Registry,
    project: &std::path::Path,
    trust_flag: bool,
    interactive: bool,
    reader: R,
    writer: W,
) -> Result<()> {
    if registry.is_trusted(project) {
        return Ok(());
    }
    if trust_flag {
        return registry.trust(project);
    }
    confirm_trust(registry, project, interactive, reader, writer)
}

fn confirm_trust<R: BufRead, W: Write>(
    registry: &super::trust::Registry,
    project: &std::path::Path,
    interactive: bool,
    mut reader: R,
    mut writer: W,
) -> Result<()> {
    require(
        interactive,
        "this directory is not trusted yet: run antislop here in a terminal to answer, \
         or pass --trust",
    )?;
    writeln!(
        writer,
        "Do you trust the files in this directory?\n  {}\nAntislop agents can read files, run commands, and modify code in this directory.",
        project.display()
    )?;
    write!(writer, "Trust this directory? [y/N]: ")?;
    writer.flush()?;
    let answer = read_trust_answer(&mut reader)?;
    require(answer, "workspace directory is not trusted")?;
    registry.trust(project)
}

fn read_trust_answer(reader: &mut impl BufRead) -> Result<bool> {
    let mut answer = String::new();
    reader.read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// One-time setup that an agent CLI needs and that only the user should do.
/// `agy` registers the stdio MCP bridge in Agy's own MCP list: Agy has no
/// per-launch MCP option, so its list names `antislop mcp-bridge` once and each
/// session passes its gateway through the environment.
pub fn setup(args: &[String]) -> Result<()> {
    match args {
        [target] if target == "agy" => {
            let exe = std::env::current_exe()?;
            let status = std::process::Command::new("agy")
                .args(["mcp", "add", super::mcp::SERVER])
                .arg(&exe)
                .arg("mcp-bridge")
                .status()
                .map_err(|error| crate::Error::Invalid(format!("cannot run agy: {error}")))?;
            crate::require(status.success(), "agy could not register the MCP bridge")?;
            println!(
                "agy now starts `{} mcp-bridge` for its `{}` MCP server",
                exe.display(),
                super::mcp::SERVER
            );
            Ok(())
        }
        _ => Err(crate::Error::Invalid("usage: antislop setup agy".into())),
    }
}

/// Stops the existing daemon without starting one or loading configuration.
pub fn stop(args: &[String]) -> Result<()> {
    let mut state = None;
    let mut force = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--state" => {
                state = Some(PathBuf::from(args.next().ok_or_else(|| {
                    crate::Error::Invalid("--state requires a value".into())
                })?));
            }
            "--force" => force = true,
            _ => return Err(crate::Error::Invalid(format!("unknown stop option {arg}"))),
        }
    }
    let state = state.unwrap_or_else(|| PathBuf::from(".antislop"));
    if Client::stop(&state, force)? {
        println!("daemon stopped");
    } else {
        println!("daemon is not running");
    }
    Ok(())
}

pub fn chat(args: &[String]) -> Result<()> {
    chat::run(args)
}

#[cfg(test)]
#[path = "cli/tests.rs"]
mod tests;
