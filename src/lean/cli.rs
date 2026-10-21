//! `antislop chat`: talk to the orchestrator. In a terminal without a message
//! it opens the TUI; otherwise it prints events line by line. Either way the
//! project's daemon does the work, so it goes on after the frontend exits.
//! Lines typed while agents work are accepted at once and answered in the
//! orchestrator's next turn. `/accept` starts the proposed plan, `/resume`
//! runs again what the last plan left unfinished, `/cancel` stops running
//! work.

use super::daemon::{Client, Request};
use super::tui::view::clean;
use crate::{Result, require};
use serde_json::Value;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// Select and validate the configuration without creating files or directories.
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
    let path = path.canonicalize()?;
    crate::config::Config::load(&path)?;
    Ok(path)
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
        let mut config = None;
        let mut state = None;
        let mut conversation = None;
        let mut new = false;
        let mut trust = false;
        let mut words = vec![];
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let mut value = |flag: &str| {
                args.next()
                    .cloned()
                    .ok_or_else(|| crate::Error::Invalid(format!("{flag} requires a value")))
            };
            match arg.as_str() {
                "--config" => config = Some(PathBuf::from(value(arg)?)),
                "--state" => state = Some(PathBuf::from(value(arg)?)),
                "--conversation" => conversation = Some(value(arg)?),
                "--new" => new = true,
                "--trust" => trust = true,
                flag if flag.starts_with("--") => {
                    return Err(crate::Error::Invalid(format!("unknown option {flag}")));
                }
                _ => words.push(arg.clone()),
            }
        }
        let config = resolve_config(&project, config.as_deref())?;
        // Nothing is created yet: the directory may not be trusted.
        let state = state.unwrap_or_else(|| project.join(".antislop"));
        let state = if state.is_absolute() {
            state
        } else {
            project.join(state)
        };
        Ok(Self {
            project,
            config,
            state,
            conversation,
            new,
            trust,
            message: (!words.is_empty()).then(|| words.join(" ")),
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
        Ok(client.call(&Request::Start)?["id"]
            .as_str()
            .unwrap_or_default()
            .to_owned())
    }
    /// This project's latest conversation.
    pub fn latest(&self, client: &Client) -> Result<Option<String>> {
        let project = self.project.to_string_lossy();
        Ok(client
            .call(&Request::Conversations)?
            .as_array()
            .into_iter()
            .flatten()
            .filter(|conversation| conversation["project"] == project.as_ref())
            .max_by_key(|conversation| conversation["created_ms"].as_i64())
            .and_then(|conversation| conversation["id"].as_str().map(str::to_owned)))
    }
}

/// Makes sure the user let antislop work in `project`: it is remembered, or
/// `--trust` says so, or the user answers now. Without a terminal and
/// without `--trust` there is nobody to ask, so the answer is no.
pub fn ensure_trusted<R: BufRead, W: Write>(
    registry: &super::trust::Registry,
    project: &std::path::Path,
    trust_flag: bool,
    interactive: bool,
    mut reader: R,
    mut writer: W,
) -> Result<()> {
    if registry.is_trusted(project) {
        return Ok(());
    }
    if trust_flag {
        return registry.trust(project);
    }
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
    let mut answer = String::new();
    reader.read_line(&mut answer)?;
    let answer = answer.trim().to_ascii_lowercase();
    require(
        answer == "y" || answer == "yes",
        "workspace directory is not trusted",
    )?;
    registry.trust(project)
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
    let mut options = Options::parse(args)?;
    // Before anything is created or started in this directory.
    ensure_trusted(
        &super::trust::Registry::from_env()?,
        &options.project,
        options.trust,
        std::io::stdin().is_terminal(),
        std::io::stdin().lock(),
        std::io::stderr(),
    )?;
    options.prepare()?;
    let client = Client::connect(&options.project, &options.config, &options.state)?;
    if options.message.is_none() && std::io::stdin().is_terminal() {
        return super::tui::run(&options, client);
    }
    if let Some(notice) = &client.notice {
        eprintln!("{notice}");
    }
    let conversation = options.conversation(&client)?;
    eprintln!("conversation {conversation}");
    let events = |after: u64, wait_ms: u64| -> Result<(Vec<Value>, bool)> {
        let reply = client.call(&Request::Events {
            conversation: conversation.clone(),
            after,
            wait_ms,
        })?;
        Ok((
            reply["events"].as_array().cloned().unwrap_or_default(),
            reply["idle"] == true,
        ))
    };
    // Only what happens from now on, but remember the latest plan so
    // `/accept` refers to the plan that was shown.
    let mut last = 0;
    let mut plan = None;
    loop {
        let (page, _) = events(last, 0)?;
        let Some(event) = page.last() else { break };
        last = event["seq"].as_u64().unwrap_or(last);
        plan = latest_plan(&page).or(plan);
    }
    if let Some(message) = &options.message {
        client.call(&Request::Send {
            conversation: conversation.clone(),
            text: message.clone(),
        })?;
    }
    let (lines, typed) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let mut input_open = true;
    loop {
        loop {
            let line = match typed.try_recv() {
                Ok(line) => line,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    input_open = false;
                    break;
                }
            };
            let conversation = conversation.clone();
            let request = match line.trim() {
                "" => continue,
                "/accept" => match &plan {
                    Some(plan) => Request::Accept {
                        conversation,
                        plan: plan.clone(),
                    },
                    None => {
                        eprintln!("✗ there is no proposed plan");
                        continue;
                    }
                },
                "/resume" => match &plan {
                    Some(plan) => Request::Resume {
                        conversation,
                        plan: plan.clone(),
                    },
                    None => {
                        eprintln!("✗ there is no plan to resume");
                        continue;
                    }
                },
                "/cancel" => Request::Cancel { conversation },
                "/pause" => Request::Pause {
                    conversation,
                    paused: true,
                },
                "/continue" => Request::Pause {
                    conversation,
                    paused: false,
                },
                "/mode autonomous" | "/mode supervised" => Request::Mode {
                    conversation,
                    autonomous: line.trim().ends_with("autonomous"),
                },
                text => Request::Send {
                    conversation,
                    text: text.to_owned(),
                },
            };
            if let Err(error) = client.call(&request) {
                eprintln!("✗ {}", clean(&error.to_string()));
            }
        }
        let (page, idle) = events(last, if input_open { 200 } else { 2_000 })?;
        plan = latest_plan(&page).or(plan);
        for event in &page {
            last = event["seq"].as_u64().unwrap_or(last);
            print(event);
        }
        if !input_open && idle && page.is_empty() {
            return Ok(());
        }
    }
}

fn latest_plan(events: &[Value]) -> Option<String> {
    events
        .iter()
        .rev()
        .find(|event| event["kind"] == "plan_proposed")
        .and_then(|event| event["payload"]["plan"].as_str().map(str::to_owned))
}

/// One terminal line for an event, if it is worth one.
pub fn describe(event: &Value) -> Option<String> {
    let payload = &event["payload"];
    let text = |key: &str| payload[key].as_str().unwrap_or_default().to_owned();
    Some(match event["kind"].as_str()? {
        "message" if payload["author"] == "runtime" => {
            "· task results handed to orchestrator".into()
        }
        "message" if payload["author"] == "plan-verifier" => {
            format!("\nplan verifier sent the plan back:\n{}\n", text("text"))
        }
        "message" if payload["author"] != "user" => {
            format!("\n{}:\n{}\n", text("author"), text("text"))
        }
        "turn_started" => format!(
            "· {} ({}) is working…",
            text("role").split(':').next().unwrap_or_default(),
            text("provider")
        ),
        "plan_proposed" => {
            let tasks: Vec<String> = payload["tasks"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|task| {
                    format!(
                        "  [{}] {} ({}{})",
                        task["id"].as_str().unwrap_or_default(),
                        task["title"].as_str().unwrap_or_default(),
                        task["kind"].as_str().unwrap_or_default(),
                        if task["writes"] == true {
                            ", modifies files"
                        } else {
                            ""
                        }
                    )
                })
                .collect();
            let mut lines = review_lines("plan verifier", &payload["review"]);
            lines.insert(0, format!("Plan:\n{}", tasks.join("\n")));
            lines.push(if payload["automatic"] == true {
                "Running (autonomous mode).".to_owned()
            } else {
                let waiting = payload["waiting"]
                    .as_str()
                    .map(|reason| format!("Waiting for you: {reason}. "))
                    .unwrap_or_default();
                format!("{waiting}Type /accept to run it.")
            });
            lines.join("\n")
        }
        "audit" => review_lines("audit", payload).join("\n"),
        "paused" if payload["paused"] == true => {
            "⏸ paused: new steps wait, /continue resumes".into()
        }
        "paused" => "▶ resumed".into(),
        "notice" => format!("⚠ {}", text("text")),
        "mode" if payload["autonomous"] == true => {
            "⚙ autonomous mode: next plans and rounds start automatically".into()
        }
        "mode" => "⚙ supervised mode: next plans wait for /accept".into(),
        "mail" if payload["kind"] != "copy" => format!(
            "✉ {} → {} ({}): {}",
            text("sender"),
            text("recipient"),
            text("kind"),
            text("text")
        ),
        "mail_delivered" => format!("✓ delivered messages {}", payload["ids"]),
        "mail_rejected" => format!(
            "✗ message to {} undeliverable: {}",
            text("recipient"),
            text("reason")
        ),
        "plan_invalid" => format!("✗ plan rejected: {}", text("error")),
        "task" => format!(
            "· [{}] {}: {} (round {}){}",
            text("id"),
            text("title"),
            text("status"),
            payload["round"],
            match payload["reason"].as_str() {
                Some(reason) if payload["status"] == "waiting" => format!(
                    " — {reason}; agent CLI is waiting for login or rate limit reset, then /resume"
                ),
                Some(reason) => format!(" — {reason}"),
                None => String::new(),
            }
        ),
        "agent" if payload["type"] == "tool" => {
            format!("  ↳ {} {}", text("name"), text("detail"))
        }
        "turn_finished" if payload["status"] != "done" => {
            format!("✗ {}: {}", text("status"), text("error"))
        }
        _ => return None,
    })
}

/// What a reviewer (plan verifier, auditor) concluded, for the user.
pub fn review_lines(who: &str, review: &Value) -> Vec<String> {
    if review.is_null() {
        return vec![];
    }
    if let Some(error) = review["error"].as_str() {
        return vec![format!("⚠ {who} failed: {error}")];
    }
    let findings: Vec<String> = review["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|finding| match finding.as_str() {
            Some(text) => format!("  - {text}"),
            None => format!(
                "  - [{}] {}",
                finding["severity"].as_str().unwrap_or("?"),
                finding["issue"].as_str().unwrap_or_default()
            ),
        })
        .collect();
    let mut lines = vec![if review["approved"] == true {
        format!("✓ {who}: no issues")
    } else {
        format!("⚠ {who}: issues found")
    }];
    if review["approved"] == true && !findings.is_empty() {
        lines[0] = format!("✓ {who}: minor notes");
    }
    lines.extend(findings);
    if let Some(coverage) = review["coverage"].as_object().filter(|map| !map.is_empty()) {
        lines.push("  Obligation coverage:".into());
        for (id, entry) in coverage {
            let mark = match entry["status"]
                .as_str()
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("ok") => "✓",
                Some("issue") => "✗",
                Some("blocked") => "!",
                Some("n/a") => "–",
                _ => "?",
            };
            lines.push(format!(
                "    {mark} {id}: {}",
                entry["note"].as_str().unwrap_or_default()
            ));
        }
    }
    lines
}

fn print(event: &Value) {
    if let Some(line) = describe(event) {
        let mut out = std::io::stdout().lock();
        // Agents' text may carry terminal escapes (design §13).
        let _ = writeln!(out, "{}", clean(&line));
        let _ = out.flush();
    }
}

#[cfg(test)]
mod trust_tests {
    use super::*;
    use crate::lean::trust::Registry;
    use std::io::Cursor;

    fn ask(
        registry: &Registry,
        dir: &std::path::Path,
        flag: bool,
        tty: bool,
        input: &str,
    ) -> (Result<()>, String) {
        let mut shown = Vec::new();
        let result = ensure_trusted(
            registry,
            dir,
            flag,
            tty,
            Cursor::new(input.to_owned()),
            &mut shown,
        );
        (result, String::from_utf8(shown).unwrap())
    }

    #[test]
    fn the_directory_is_asked_about_once_and_never_without_a_terminal() {
        let config = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let registry = Registry::at(config.path());

        // No terminal and no flag: nobody to ask, so no.
        let (result, shown) = ask(&registry, project.path(), false, false, "y\n");
        assert!(result.unwrap_err().to_string().contains("--trust"));
        assert!(shown.is_empty() && !registry.is_trusted(project.path()));

        // Declining (or an empty answer) leaves it untrusted.
        for answer in ["n\n", "\n", "maybe\n"] {
            let (result, shown) = ask(&registry, project.path(), false, true, answer);
            assert!(result.is_err());
            assert!(shown.contains("Do you trust the files in this directory?"));
            assert!(!registry.is_trusted(project.path()));
        }

        // Yes is remembered: the next start does not ask.
        let (result, _) = ask(&registry, project.path(), false, true, "Yes\n");
        result.unwrap();
        let (result, shown) = ask(&registry, project.path(), false, false, "");
        result.unwrap();
        assert!(shown.is_empty());

        // The flag answers for a directory not yet known.
        let other = tempfile::tempdir().unwrap();
        let (result, _) = ask(&registry, other.path(), true, false, "");
        result.unwrap();
        assert!(registry.is_trusted(other.path()));
    }

    #[test]
    fn parsing_the_options_creates_nothing() {
        let before = std::env::current_dir().unwrap();
        let options =
            Options::parse(&["--config".into(), "Cargo.toml".into(), "hi".into()]).unwrap();
        assert!(!options.state.exists() || options.state.starts_with(&before));
        assert_eq!(options.message.as_deref(), Some("hi"));
    }
}
