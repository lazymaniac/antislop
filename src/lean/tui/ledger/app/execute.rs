use crate::lean::daemon::{Client, Request};
use crate::lean::tui::{Update, ledger};
use serde_json::Value;

pub(super) struct LedgerWork {
    pub(super) conversation: String,
    pub(super) kind: String,
    pub(super) action: ledger::Action,
    pub(super) request: Option<Request>,
}

pub(super) fn spawn_ledger_work(
    client: Client,
    updates: std::sync::mpsc::Sender<Update>,
    work: LedgerWork,
    request_id: u64,
) {
    std::thread::spawn(move || {
        let reply = execute_work(&client, &work).map_err(|error| error.to_string());
        let _ = updates.send(Update::Ledger {
            conversation: work.conversation,
            request_id,
            kind: work.kind,
            reply,
        });
    });
}

fn execute_work(client: &Client, work: &LedgerWork) -> crate::Result<Value> {
    if let ledger::Action::Prepare { change, source } = &work.action {
        return prepare_change(client, &work.conversation, change.clone(), source.as_str());
    }
    let value = client.call(
        work.request
            .as_ref()
            .expect("non-prepare ledger action has a request"),
    )?;
    if work.kind == "created" {
        Ok(empty_run_page(value))
    } else {
        Ok(value)
    }
}

fn prepare_change(
    client: &Client,
    conversation: &str,
    mut change: crate::lean::ledger::LedgerChange,
    source: &str,
) -> crate::Result<Value> {
    if !source.trim().is_empty() {
        register_source(client, conversation, &mut change, source)?;
    }
    let proposal = client.call(&Request::ProposeLedgerChange {
        change: change.clone(),
    })?;
    Ok(serde_json::json!({"change":change,"proposal":proposal}))
}

fn register_source(
    client: &Client,
    conversation: &str,
    change: &mut crate::lean::ledger::LedgerChange,
    source: &str,
) -> crate::Result<()> {
    let registered = client.call(&Request::AddLedgerSource {
        conversation: conversation.into(),
        run_id: change.run_id.clone(),
        text: source.into(),
    })?;
    let id = registered["id"]
        .as_str()
        .ok_or_else(|| crate::Error::Invalid("source registration returned no identity".into()))?;
    for entry in &mut change.entries {
        entry.source = crate::lean::ledger::SourceSpan {
            source_id: id.into(),
            start: 0,
            end: source.len(),
        };
    }
    Ok(())
}

fn empty_run_page(run: Value) -> Value {
    serde_json::json!({"run":run,"entries":[],"total":0,"offset":0,"has_more":false})
}
