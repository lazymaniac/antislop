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
use std::path::PathBuf;
use std::sync::mpsc;

pub struct Options {
    pub project: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
    pub conversation: Option<String>,
    pub new: bool,
    pub message: Option<String>,
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Self> {
        let mut project = std::env::current_dir()?;
        let mut config = None;
        let mut state = None;
        let mut conversation = None;
        let mut new = false;
        let mut words = vec![];
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let mut value = |flag: &str| {
                args.next()
                    .cloned()
                    .ok_or_else(|| crate::Error::Invalid(format!("{flag} requires a value")))
            };
            match arg.as_str() {
                "--project" => project = PathBuf::from(value(arg)?),
                "--config" => config = Some(PathBuf::from(value(arg)?)),
                "--state" => state = Some(PathBuf::from(value(arg)?)),
                "--conversation" => conversation = Some(value(arg)?),
                "--new" => new = true,
                flag if flag.starts_with("--") => {
                    return Err(crate::Error::Invalid(format!("unknown option {flag}")));
                }
                _ => words.push(arg.clone()),
            }
        }
        let project = project.canonicalize()?;
        let config = config.unwrap_or_else(|| project.join("architect.toml"));
        require(
            config.is_file(),
            &format!("no configuration at {}", config.display()),
        )?;
        let state = state.unwrap_or_else(|| project.join(".antislop"));
        super::daemon::private_directory(&state)?;
        Ok(Self {
            project,
            config: config.canonicalize()?,
            state: state.canonicalize()?,
            conversation,
            new,
            message: (!words.is_empty()).then(|| words.join(" ")),
        })
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

pub fn chat(args: &[String]) -> Result<()> {
    let options = Options::parse(args)?;
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
            "· wyniki zadań przekazane orkiestratorowi".into()
        }
        "message" if payload["author"] == "plan-verifier" => {
            format!("\nweryfikator planu odesłał plan:\n{}\n", text("text"))
        }
        "message" if payload["author"] != "user" => {
            format!("\n{}:\n{}\n", text("author"), text("text"))
        }
        "turn_started" => format!(
            "· {} ({}) pracuje…",
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
                            ", zmienia pliki"
                        } else {
                            ""
                        }
                    )
                })
                .collect();
            let mut lines = review_lines("weryfikator planu", &payload["review"]);
            lines.insert(0, format!("Plan:\n{}", tasks.join("\n")));
            lines.push(if payload["automatic"] == true {
                "Uruchamiam (tryb autonomiczny).".to_owned()
            } else {
                let waiting = payload["waiting"]
                    .as_str()
                    .map(|reason| format!("Czeka na Ciebie: {reason}. "))
                    .unwrap_or_default();
                format!("{waiting}Wpisz /accept, aby go uruchomić.")
            });
            lines.join("\n")
        }
        "audit" => review_lines("audyt", payload).join("\n"),
        "paused" if payload["paused"] == true => {
            "⏸ wstrzymano: nowe kroki czekają, /continue wznawia".into()
        }
        "paused" => "▶ wznowiono".into(),
        "mode" if payload["autonomous"] == true => {
            "⚙ tryb autonomiczny: kolejne plany i rundy ruszają same".into()
        }
        "mode" => "⚙ tryb nadzorowany: kolejne plany czekają na /accept".into(),
        "mail" if payload["kind"] != "copy" => format!(
            "✉ {} → {} ({}): {}",
            text("sender"),
            text("recipient"),
            text("kind"),
            text("text")
        ),
        "mail_delivered" => format!("✓ dostarczono wiadomości {}", payload["ids"]),
        "mail_rejected" => format!(
            "✗ wiadomość do {} niedostarczalna: {}",
            text("recipient"),
            text("reason")
        ),
        "plan_invalid" => format!("✗ plan odrzucony: {}", text("error")),
        "task" => format!(
            "· [{}] {}: {} (runda {}){}",
            text("id"),
            text("title"),
            text("status"),
            payload["round"],
            match payload["reason"].as_str() {
                Some(reason) if payload["status"] == "waiting" => format!(
                    " — {reason}; CLI agenta czeka na logowanie albo odnowienie limitu, potem /resume"
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
        return vec![format!("⚠ {who} nie zadziałał: {error}")];
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
        format!("✓ {who}: bez zastrzeżeń")
    } else {
        format!("⚠ {who}: zastrzeżenia")
    }];
    if review["approved"] == true && !findings.is_empty() {
        lines[0] = format!("✓ {who}: drobne uwagi");
    }
    lines.extend(findings);
    if let Some(coverage) = review["coverage"].as_object().filter(|map| !map.is_empty()) {
        lines.push("  Rozliczenie obowiązków:".into());
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
