use super::super::daemon::{Client, Request};
use super::super::tui::view::clean;
use super::presentation::{latest_plan, print};
use super::{Options, ensure_trusted};
use crate::Result;
use serde_json::Value;
use std::io::{BufRead, IsTerminal};
use std::sync::mpsc;

pub(super) fn run(args: &[String]) -> Result<()> {
    let (options, client) = connect(args)?;
    if options.message.is_none() && std::io::stdin().is_terminal() {
        return super::super::tui::run(&options, client);
    }
    if let Some(notice) = &client.notice {
        eprintln!("{notice}");
    }
    let conversation = options.conversation(&client)?;
    eprintln!("conversation {conversation}");
    stream(&options, client, conversation)
}

fn connect(args: &[String]) -> Result<(Options, Client)> {
    let mut options = Options::parse(args)?;
    // Before anything is created or started in this directory.
    ensure_trusted(
        &super::super::trust::Registry::from_env()?,
        &options.project,
        options.trust,
        std::io::stdin().is_terminal(),
        std::io::stdin().lock(),
        std::io::stderr(),
    )?;
    crate::config::Config::load(&options.config)?;
    options.prepare()?;
    let client = Client::connect(&options.project, &options.config, &options.state)?;
    Ok((options, client))
}

fn events(
    client: &Client,
    conversation: &str,
    after: u64,
    wait_ms: u64,
) -> Result<(Vec<Value>, bool)> {
    let reply = client.call(&Request::History(
        crate::lean::daemon::HistoryRequest::Events {
            conversation: conversation.to_owned(),
            after,
            wait_ms,
        },
    ))?;
    Ok((
        reply["events"].as_array().cloned().unwrap_or_default(),
        reply["idle"] == true,
    ))
}

fn stream(options: &Options, client: Client, conversation: String) -> Result<()> {
    // Only what happens from now on, but remember the latest plan so
    // `/accept` refers to the plan that was shown.
    let mut last = 0;
    let mut plan = None;
    loop {
        let (page, _) = events(&client, &conversation, last, 0)?;
        let Some(event) = page.last() else { break };
        last = event["seq"].as_u64().unwrap_or(last);
        plan = latest_plan(&page).or(plan);
    }
    if let Some(message) = &options.message {
        send_message(&client, &conversation, message)?;
    }
    let (lines, typed) = mpsc::channel();
    std::thread::spawn(move || read_input(lines));
    poll_events(client, conversation, typed, last, plan)
}

fn send_message(client: &Client, conversation: &str, message: &str) -> Result<()> {
    client.call(&Request::Conversation(
        crate::lean::daemon::ConversationRequest::Send {
            conversation: conversation.to_owned(),
            text: message.to_owned(),
        },
    ))?;
    Ok(())
}

fn read_input(lines: mpsc::Sender<String>) {
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if lines.send(line).is_err() {
            break;
        }
    }
}

fn poll_events(
    client: Client,
    conversation: String,
    typed: mpsc::Receiver<String>,
    mut last: u64,
    mut plan: Option<String>,
) -> Result<()> {
    let mut input_open = true;
    loop {
        input_open = process_input(&client, &conversation, &typed, &plan, input_open);
        let (page, idle) = events(
            &client,
            &conversation,
            last,
            if input_open { 200 } else { 2_000 },
        )?;
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

fn process_input(
    client: &Client,
    conversation: &str,
    typed: &mpsc::Receiver<String>,
    plan: &Option<String>,
    mut open: bool,
) -> bool {
    loop {
        let line = match typed.try_recv() {
            Ok(line) => line,
            Err(mpsc::TryRecvError::Empty) => break,
            Err(mpsc::TryRecvError::Disconnected) => {
                open = false;
                break;
            }
        };
        if let Some(request) = input_request(conversation, &line, plan)
            && let Err(error) = client.call(&request)
        {
            eprintln!("✗ {}", clean(&error.to_string()));
        }
    }
    open
}

fn input_request(conversation: &str, line: &str, plan: &Option<String>) -> Option<Request> {
    let text = line.trim();
    if text.is_empty() {
        return None;
    }
    if matches!(text, "/accept" | "/resume") {
        return plan_request(conversation, text, plan);
    }
    if text == "/cancel" {
        return Some(cancel_request(conversation));
    }
    if matches!(text, "/pause" | "/continue") {
        return Some(pause_request(conversation, text));
    }
    if matches!(text, "/mode autonomous" | "/mode supervised") {
        return Some(mode_request(conversation, text));
    }
    Some(send_request(conversation, text))
}

fn plan_request(conversation: &str, command: &str, plan: &Option<String>) -> Option<Request> {
    let Some(plan) = plan else {
        eprintln!(
            "✗ {}",
            if command == "/accept" {
                "there is no proposed plan"
            } else {
                "there is no plan to resume"
            }
        );
        return None;
    };
    let conversation = conversation.to_owned();
    let request = if command == "/accept" {
        crate::lean::daemon::ConversationRequest::Accept {
            conversation,
            plan: plan.clone(),
        }
    } else {
        crate::lean::daemon::ConversationRequest::Resume {
            conversation,
            plan: plan.clone(),
        }
    };
    Some(Request::Conversation(request))
}

fn cancel_request(conversation: &str) -> Request {
    Request::Conversation(crate::lean::daemon::ConversationRequest::Cancel {
        conversation: conversation.to_owned(),
    })
}

fn pause_request(conversation: &str, command: &str) -> Request {
    Request::Conversation(crate::lean::daemon::ConversationRequest::Pause {
        conversation: conversation.to_owned(),
        paused: command == "/pause",
    })
}

fn mode_request(conversation: &str, command: &str) -> Request {
    Request::Conversation(crate::lean::daemon::ConversationRequest::Mode {
        conversation: conversation.to_owned(),
        autonomous: command.ends_with("autonomous"),
    })
}

fn send_request(conversation: &str, text: &str) -> Request {
    Request::Conversation(crate::lean::daemon::ConversationRequest::Send {
        conversation: conversation.to_owned(),
        text: text.to_owned(),
    })
}
