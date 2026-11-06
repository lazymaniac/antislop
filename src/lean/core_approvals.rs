//! Approval dialogs of agent CLIs. The CLI asks; the runtime turns the dialog
//! into a question that the user answers, or, in an autonomous conversation,
//! that the orchestrator decides, and the chosen key goes back to the CLI.
use super::native::approvals::{self, Approval, Decision, Kind, Verdict};
use super::{Core, store};
use crate::{Error, Result};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Whose dialog it is.
#[derive(Clone)]
pub(super) struct ApprovalSeat {
    pub conversation: String,
    pub role: String,
    pub turn: String,
    pub provider: String,
    /// Directories the runtime itself gave this CLI to work in.
    pub directories: Vec<PathBuf>,
}

/// Dialogs the runtime answers itself: the CLI asks to trust a directory the
/// runtime allocated to it. Anything else is a decision for the user or the
/// orchestrator.
fn automatic_choice(seat: &ApprovalSeat, approval: &Approval) -> Option<String> {
    // The runtime's own gateway is how the agent answers the runtime.
    if approval.mcp_server() == Some(super::mcp::SERVER) {
        return approval
            .choices
            .iter()
            .find(|choice| choice.decision == Decision::Allow)
            .map(|choice| choice.id.clone());
    }
    // The second step of a choice already made.
    if approval.kind == Kind::Confirm {
        return approval
            .choices
            .iter()
            .find(|choice| choice.decision == Decision::Allow)
            .map(|choice| choice.id.clone());
    }
    if approval.kind != Kind::Trust {
        return None;
    }
    let compact = |text: &str| text.split_whitespace().collect::<String>();
    let shown = compact(&approval.summary);
    let ours = seat.directories.iter().any(|directory| {
        [Some(directory.clone()), directory.canonicalize().ok()]
            .into_iter()
            .flatten()
            .any(|path| shown.contains(&compact(&path.to_string_lossy())))
    });
    ours.then(|| {
        approval
            .choices
            .iter()
            .find(|choice| choice.decision == Decision::Allow)
            .map(|choice| choice.id.clone())
    })
    .flatten()
}

fn decision_name(decision: Decision) -> &'static str {
    match decision {
        Decision::Allow => "allow",
        Decision::AllowAlways => "allow always",
        Decision::Deny => "deny",
        Decision::Other => "other",
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Command => "command",
        Kind::Mcp => "mcp",
        Kind::Confirm => "confirm",
        Kind::Trust => "trust",
        Kind::Other => "other",
    }
}

impl Core {
    /// Settles one dialog and returns what to press. Blocks until someone
    /// decided, the dialog left the screen, or the turn was cancelled.
    pub(super) fn settle_approval(
        self: &Arc<Self>,
        seat: &ApprovalSeat,
        approval: &Approval,
        cancel: &AtomicBool,
        shown: &dyn Fn() -> bool,
    ) -> Result<Verdict> {
        let conversation = &seat.conversation;
        if let Some(choice) = automatic_choice(seat, approval) {
            self.store.append_event(
                conversation,
                Some(&seat.turn),
                None,
                "approval_auto",
                &json!({"role":seat.role,"summary":approval.summary,"choice":choice}),
            )?;
            return Ok(Verdict::Choose(choice));
        }
        let options: Vec<Value> = approval
            .choices
            .iter()
            .map(|choice| {
                json!({"id":choice.id,"label":choice.label,"decision":decision_name(choice.decision)})
            })
            .collect();
        let decider = if self.store.approval_mode(conversation)? == store::AUTONOMOUS
            && !super::ORCHESTRATION_ROLES.contains(&seat.role.as_str())
        {
            "orchestrator"
        } else {
            "user"
        };
        let options = Value::Array(options);
        let request = self.store.open_approval(
            conversation,
            &store::ApprovalRequest {
                turn: &seat.turn,
                role: &seat.role,
                provider: &seat.provider,
                kind: kind_name(approval.kind),
                summary: &approval.summary,
                options: &options,
                fingerprint: &approval.fingerprint,
                decider,
            },
        )?;
        if decider == "orchestrator" {
            self.ask_orchestrator_to_decide(seat, &request, approval)?;
        }
        loop {
            if cancel.load(Ordering::SeqCst) {
                // Nothing is pressed; the wait loop then handles the cancel.
                self.store
                    .close_approval(conversation, &request, "cancelled")?;
                return Ok(Verdict::AnsweredInTerminal);
            }
            let (state, choice, _) = self.store.approval_state(&request)?;
            match (state.as_str(), choice) {
                ("answered", Some(choice)) => return Ok(Verdict::Choose(choice)),
                ("closed", _) => return Ok(Verdict::AnsweredInTerminal),
                _ => {}
            }
            if !shown() {
                self.store
                    .close_approval(conversation, &request, "answered in the terminal")?;
                return Ok(Verdict::AnsweredInTerminal);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn ask_orchestrator_to_decide(
        self: &Arc<Self>,
        seat: &ApprovalSeat,
        request: &str,
        approval: &Approval,
    ) -> Result<()> {
        let choices = approval
            .choices
            .iter()
            .map(|choice| {
                format!(
                    "- {}: {} ({})",
                    choice.id,
                    choice.label,
                    decision_name(choice.decision)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let text = format!(
            "Approval request {request}, source {}, turn {}: the {} CLI of this agent asks:\n{}\n{choices}\nDecide with decide_approval (request, choice). Allow what the agent's task needs and nothing broader; deny the rest. Choose escalate_to_user, with an explanation, when you cannot judge it or it goes beyond what the user asked.",
            seat.role, seat.turn, seat.provider, approval.summary
        );
        self.store
            .add_message(&seat.conversation, super::RUNTIME, &text)?;
        self.wake(&seat.conversation);
        Ok(())
    }

    /// The orchestrator's decision on an approval put to it.
    pub(super) fn decide_approval_tool(&self, conversation: &str, args: &Value) -> Result<Value> {
        let request = super::turn_result::required_text(args, "request")?;
        let choice = super::turn_result::required_text(args, "choice")?;
        let (state, _, decider) = self.store.approval_state(&request)?;
        if choice == "escalate_to_user" {
            let explanation = super::turn_result::required_text(args, "explanation")?;
            if decider != "user" && state == "open" {
                self.store.escalate_approval(conversation, &request)?;
                self.notice(
                    conversation,
                    &format!(
                        "An agent's approval request {request} needs your decision: {}",
                        super::tasks::bound(&explanation, 600)
                    ),
                );
            }
            return Ok(json!({"request":request,"state":"awaiting_user"}));
        }
        self.store
            .answer_approval(conversation, &request, &choice, "orchestrator")?;
        Ok(json!({"request":request,"state":"answered","choice":choice}))
    }

    /// The user's decision on an approval, from the terminal UI.
    pub fn answer_approval(&self, conversation: &str, request: &str, choice: &str) -> Result<()> {
        self.store
            .answer_approval(conversation, request, choice, "user")
    }

    pub fn approval_mode(&self, conversation: &str) -> Result<String> {
        self.store.approval_mode(conversation)
    }

    pub fn set_approval_mode(&self, conversation: &str, mode: &str) -> Result<()> {
        self.store.set_approval_mode(conversation, mode)
    }
}

/// The handler a session calls for each dialog of this turn.
pub(super) fn approver(
    core: &Core,
    conversation: &str,
    role: &str,
    turn: &str,
    spec: &super::agent::Spec,
) -> Arc<approvals::Approver> {
    let weak = core.weak.get().expect("core weak reference").clone();
    let mut directories = vec![spec.cwd.clone()];
    directories.extend(spec.add_dirs.iter().cloned());
    let seat = ApprovalSeat {
        conversation: conversation.to_owned(),
        role: role.to_owned(),
        turn: turn.to_owned(),
        provider: super::agent::name(spec.provider).to_owned(),
        directories,
    };
    Arc::new(move |approval, cancel, shown| {
        let core = weak
            .upgrade()
            .ok_or_else(|| Error::Invalid("daemon stopped".into()))?;
        core.settle_approval(&seat, approval, cancel, shown)
    })
}

#[cfg(test)]
#[path = "core_approvals/tests.rs"]
mod tests;
