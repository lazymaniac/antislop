use crate::lean::daemon::Request;
use crate::lean::tui::App;
use crate::lean::tui::ledger::{self, LedgerView};
#[path = "app/execute.rs"]
mod execute;
use execute::{LedgerWork, spawn_ledger_work};

impl App {
    pub(in crate::lean::tui) fn open_ledger(&mut self) {
        let view = self.ledger_view.get_or_insert_with(LedgerView::new);
        view.visible = true;
        self.ledger_action(ledger::Action::Fetch);
    }

    pub(in crate::lean::tui) fn ledger_action(&mut self, action: ledger::Action) {
        let Some(view) = self.ledger_view.as_mut() else {
            return;
        };
        if matches!(action, ledger::Action::None) || view.pending {
            return;
        }
        let conversation = self.conversation.clone();
        let Some((kind, request)) = route_action(&action, view, &conversation) else {
            return;
        };
        mark_pending(view, &action, kind);
        let work = LedgerWork {
            conversation,
            kind: kind.into(),
            action,
            request,
        };
        spawn_ledger_work(
            self.client.clone(),
            self.updates.clone(),
            work,
            view.request_id,
        );
    }
}

fn route_action(
    action: &ledger::Action,
    view: &LedgerView,
    conversation: &str,
) -> Option<(&'static str, Option<Request>)> {
    match action {
        ledger::Action::Fetch => Some(("page", Some(view.request(conversation)))),
        ledger::Action::Detail | ledger::Action::Source => route_record(action, view, conversation),
        ledger::Action::Apply { .. } => route_apply(action),
        ledger::Action::Reject { .. } => route_reject(action, conversation),
        _ => route_navigation(action, view, conversation),
    }
}

fn route_navigation(
    action: &ledger::Action,
    view: &LedgerView,
    conversation: &str,
) -> Option<(&'static str, Option<Request>)> {
    match action {
        ledger::Action::Prepare { .. } => Some(("preview", None)),
        ledger::Action::Runs => Some((
            "runs",
            Some(Request::Ledger(
                crate::lean::daemon::LedgerRequest::ListRuns {
                    conversation: conversation.into(),
                },
            )),
        )),
        ledger::Action::Review(run) => Some(("preview", Some(proposal_request(conversation, run)))),
        ledger::Action::Proposal => view.page.as_ref().map(|page| {
            (
                "preview",
                Some(proposal_request(conversation, &page.run.id)),
            )
        }),
        ledger::Action::Create => route_create(conversation),
        ledger::Action::None => None,
        _ => None,
    }
}

fn route_create(conversation: &str) -> Option<(&'static str, Option<Request>)> {
    let request = Request::Ledger(crate::lean::daemon::LedgerRequest::CreateRun {
        conversation: conversation.into(),
        goal: "Manually maintained requirements".into(),
    });
    Some(("created", Some(request)))
}

fn route_apply(action: &ledger::Action) -> Option<(&'static str, Option<Request>)> {
    let ledger::Action::Apply {
        proposal,
        hash,
        change,
    } = action
    else {
        return None;
    };
    Some((
        "applied",
        Some(Request::Ledger(
            crate::lean::daemon::LedgerRequest::ApplyLedgerChange {
                proposal_id: proposal.clone(),
                diff_hash: hash.clone(),
                change: change.clone(),
            },
        )),
    ))
}

fn route_reject(
    action: &ledger::Action,
    conversation: &str,
) -> Option<(&'static str, Option<Request>)> {
    let ledger::Action::Reject {
        proposal,
        hash,
        run,
        comment,
    } = action
    else {
        return None;
    };
    let request = Request::Ledger(crate::lean::daemon::LedgerRequest::RejectLedgerProposal {
        conversation: conversation.into(),
        run_id: run.clone(),
        proposal_id: proposal.clone(),
        diff_hash: hash.clone(),
        comment: comment.clone(),
    });
    Some(("rejected", Some(request)))
}

fn route_record(
    action: &ledger::Action,
    view: &LedgerView,
    conversation: &str,
) -> Option<(&'static str, Option<Request>)> {
    match action {
        ledger::Action::Detail => route_detail(view, conversation),
        ledger::Action::Source => route_source(view, conversation),
        _ => None,
    }
}

fn route_detail(view: &LedgerView, conversation: &str) -> Option<(&'static str, Option<Request>)> {
    let entry = view.selected_entry()?;
    Some((
        "detail",
        Some(Request::Ledger(
            crate::lean::daemon::LedgerRequest::GetLedgerEntry {
                conversation: conversation.into(),
                run_id: entry.run_id.clone(),
                id: entry.spec.id.clone(),
                revision: None,
            },
        )),
    ))
}

fn route_source(view: &LedgerView, conversation: &str) -> Option<(&'static str, Option<Request>)> {
    let entry = view.selected_entry()?;
    Some((
        "source",
        Some(Request::Ledger(
            crate::lean::daemon::LedgerRequest::GetLedgerSource {
                conversation: conversation.into(),
                run_id: entry.run_id.clone(),
                source_id: entry.spec.source.source_id.clone(),
            },
        )),
    ))
}

fn proposal_request(conversation: &str, run: &str) -> Request {
    Request::Ledger(crate::lean::daemon::LedgerRequest::GetLedgerProposal {
        conversation: conversation.into(),
        run_id: run.into(),
    })
}

fn mark_pending(view: &mut LedgerView, action: &ledger::Action, kind: &str) {
    view.pending_read = matches!(action, ledger::Action::Review(_) | ledger::Action::Proposal);
    view.pending_scope_epoch = view.scope_epoch;
    view.pending_run = pending_run(action, view);
    view.pending_kind = kind.into();
    if matches!(action, ledger::Action::Proposal | ledger::Action::Review(_)) {
        view.prepared_draft = None;
        view.stale = view.preview.is_some();
    }
    if matches!(action, ledger::Action::Prepare { .. }) {
        capture_prepared_draft(view);
    }
    view.pending = true;
    view.request_id = view.request_id.wrapping_add(1);
}

fn pending_run(action: &ledger::Action, view: &LedgerView) -> Option<String> {
    match action {
        ledger::Action::Review(run) => Some(run.clone()),
        ledger::Action::Proposal => view.page.as_ref().map(|page| page.run.id.clone()),
        ledger::Action::Prepare { change, .. } => Some(change.run_id.clone()),
        _ => None,
    }
}

fn capture_prepared_draft(view: &mut LedgerView) {
    view.prepared_draft = view
        .editing
        .as_ref()
        .and_then(|key| view.drafts.get(key).map(|form| (key.clone(), form.clone())));
}
