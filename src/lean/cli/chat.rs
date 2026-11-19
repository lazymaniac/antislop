use super::super::daemon::{Client, Request};
use super::super::tui::view::clean;
use super::Options;
use super::presentation::{PresentedPlan, latest_plan, print};
use crate::Result;
use serde_json::Value;
use std::io::{BufRead, IsTerminal};
use std::sync::mpsc;

pub(super) fn run(args: &[String]) -> Result<()> {
    let (options, client) = connect(args)?;
    if options.message.is_none() && std::io::stdin().is_terminal() {
        return super::super::tui::run(&options, client.named());
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
    crate::config::Config::load(&options.config)?;
    options.prepare()?;
    let client = Client::connect(
        &options.project,
        &options.config,
        &options.state,
        &std::env::current_exe()?,
    )?;
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
    // `/resume` refers to the plan that was shown.
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
            operation_id: Some(crate::id()),
            text: message.to_owned(),
            to: None,
            images: vec![],
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
    mut plan: Option<PresentedPlan>,
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
    plan: &Option<PresentedPlan>,
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

fn input_request(conversation: &str, line: &str, plan: &Option<PresentedPlan>) -> Option<Request> {
    let text = line.trim();
    if text.is_empty() {
        return None;
    }
    if text == "/resume" {
        return plan_request(conversation, plan);
    }
    if text == "/cancel" {
        return Some(cancel_request(conversation));
    }
    if matches!(text, "/pause" | "/continue") {
        return Some(pause_request(conversation, text));
    }
    Some(send_request(conversation, text))
}

fn plan_request(conversation: &str, plan: &Option<PresentedPlan>) -> Option<Request> {
    let Some(plan) = plan else {
        eprintln!("✗ there is no plan to resume");
        return None;
    };
    Some(Request::Conversation(
        crate::lean::daemon::ConversationRequest::Resume {
            conversation: conversation.to_owned(),
            operation_id: Some(crate::id()),
            plan: plan.id.clone(),
            revision_token: plan.revision_token.clone(),
        },
    ))
}

fn cancel_request(conversation: &str) -> Request {
    Request::Conversation(crate::lean::daemon::ConversationRequest::Cancel {
        conversation: conversation.to_owned(),
        operation_id: Some(crate::id()),
        revision_token: None,
    })
}

fn pause_request(conversation: &str, command: &str) -> Request {
    Request::Conversation(crate::lean::daemon::ConversationRequest::Pause {
        conversation: conversation.to_owned(),
        paused: command == "/pause",
    })
}

fn send_request(conversation: &str, text: &str) -> Request {
    Request::Conversation(crate::lean::daemon::ConversationRequest::Send {
        conversation: conversation.to_owned(),
        operation_id: Some(crate::id()),
        text: text.to_owned(),
        to: None,
        images: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resume_sends_the_displayed_scope_revision_and_clears_it_after_resolution() {
        let proposal = json!({"kind":"plan_proposed","payload":{
            "plan":"p1","scope_revision":"r1","revision_token":"reviewed-token"
        }});
        let plan = latest_plan(std::slice::from_ref(&proposal));
        let request = input_request("c1", "/resume", &plan).unwrap();
        assert_eq!(
            serde_json::to_value(request).unwrap()["revision_token"],
            "reviewed-token"
        );
        let plan = latest_plan(&[
            proposal,
            json!({"kind":"scope_revision_resolved","payload":{"plan":"p1"}}),
        ]);
        let request = input_request("c1", "/resume", &plan).unwrap();
        assert!(serde_json::to_value(request).unwrap()["revision_token"].is_null());
    }
}
