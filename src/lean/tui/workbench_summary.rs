//! Read-only Workbench summary projections built from the existing view.
//! Actions remain owned by the existing recovery and session-action commands.

use super::{TranscriptCard, unblock, view};

#[derive(Debug, Clone, PartialEq)]
pub enum Summary {
    Recovery(Recovery),
    Completion(Completion),
    Empty,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Recovery {
    pub task: String,
    pub title: String,
    pub status: String,
    /// Preserve the daemon's full cause; renderers wrap and scroll this text.
    pub reason: String,
    pub choices: Vec<unblock::Choice>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    pub branch: Option<String>,
    pub summary: String,
    pub actions: Vec<view::CompletionActionState>,
    pub selected: usize,
    pub latest_result: Option<String>,
}

pub fn project(view: &view::View, key: &str) -> Summary {
    if key == "conversation" {
        return view
            .cards
            .iter()
            .rev()
            .find_map(|card| {
                if let TranscriptCard::Completion {
                    branch,
                    summary,
                    actions,
                    selected,
                    latest_result,
                    ..
                } = card
                {
                    Some(Summary::Completion(Completion {
                        branch: branch.clone(),
                        summary: summary.clone(),
                        actions: actions.clone(),
                        selected: *selected,
                        latest_result: latest_result.clone(),
                    }))
                } else {
                    None
                }
            })
            .unwrap_or(Summary::Empty);
    }
    if key == view::AUDITOR
        && view.audit_retryable()
        && let (Some(plan), Some(reason)) = (view.plan.as_deref(), view.audit_error.as_deref())
    {
        return Summary::Recovery(Recovery::from_unblock(unblock::Unblock::auditor(
            plan, reason,
        )));
    }
    if key == view::ORCHESTRATOR {
        return Summary::Recovery(Recovery::from_unblock(unblock::Unblock::orchestrator()));
    }
    let task_id = key
        .strip_prefix("worker:")
        .or_else(|| key.strip_prefix("reviewer:"));
    let Some(task_id) = task_id else {
        return Summary::Empty;
    };
    let Some(task) = view.tasks.iter().find(|task| task.id == task_id) else {
        return Summary::Empty;
    };
    if !unblock::is_blocked(&task.status)
        && !matches!(task.status.as_str(), "queued" | "working" | "reviewing")
    {
        return Summary::Empty;
    }
    Summary::Recovery(Recovery {
        task: task.id.clone(),
        title: task.title.clone(),
        status: task.status.clone(),
        reason: task
            .reason
            .clone()
            .unwrap_or_else(|| "no reason was recorded".into()),
        choices: unblock::choices_for(task),
    })
}

/// Compare borrowed inputs before cloning or wrapping potentially large text.
pub(in crate::lean::tui) fn unchanged(summary: &Summary, view: &view::View, key: &str) -> bool {
    if key == "conversation" {
        let card = view
            .cards
            .iter()
            .rev()
            .find(|card| matches!(card, TranscriptCard::Completion { .. }));
        return match (summary, card) {
            (Summary::Empty, None) => true,
            (
                Summary::Completion(old),
                Some(TranscriptCard::Completion {
                    branch,
                    summary,
                    actions,
                    selected,
                    latest_result,
                    ..
                }),
            ) => {
                &old.branch == branch
                    && &old.summary == summary
                    && &old.actions == actions
                    && old.selected == *selected
                    && &old.latest_result == latest_result
            }
            _ => false,
        };
    }
    if key == view::AUDITOR
        && view.audit_retryable()
        && view.plan.is_some()
        && view.audit_error.is_some()
    {
        return matches!(summary, Summary::Recovery(old) if old.task == view::AUDITOR && Some(old.reason.as_str()) == view.audit_error.as_deref());
    }
    if key == view::ORCHESTRATOR {
        return matches!(summary, Summary::Recovery(old) if old.task == view::ORCHESTRATOR);
    }
    let task = key
        .strip_prefix("worker:")
        .or_else(|| key.strip_prefix("reviewer:"))
        .and_then(|id| view.tasks.iter().find(|task| task.id == id))
        .filter(|task| {
            unblock::is_blocked(&task.status)
                || matches!(task.status.as_str(), "queued" | "working" | "reviewing")
        });
    match (summary, task) {
        (Summary::Empty, None) => true,
        (Summary::Recovery(old), Some(task)) => {
            old.task == task.id
                && old.title == task.title
                && old.status == task.status
                && old.reason == task.reason.as_deref().unwrap_or("no reason was recorded")
                && old.choices == unblock::choices_for(task)
        }
        _ => false,
    }
}

impl Recovery {
    fn from_unblock(state: unblock::Unblock) -> Self {
        Self {
            task: state.task,
            title: state.title,
            status: state.status,
            reason: state.reason,
            choices: state.choices,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lean::{
        daemon::{SessionActionKind, SessionActionResult},
        tasks::ChangeKind,
    };

    #[test]
    fn recovery_projection_preserves_the_raw_cause_and_existing_choices() {
        let mut view = view::View::default();
        view.plan = Some("plan-7".into());
        view.tasks.push(view::TaskRow {
            id: "task-7".into(),
            title: "Ship safely".into(),
            status: "needs_user".into(),
            reason: Some("worker\nraw cause 👩‍💻".repeat(80)),
            change_kind: ChangeKind::Code,
            ..view::TaskRow::default()
        });
        let Summary::Recovery(projected) = project(&view, "worker:task-7") else {
            panic!("expected recovery summary");
        };
        assert_eq!(projected.reason, view.tasks[0].reason.clone().unwrap());
        assert_eq!(projected.choices, unblock::choices_for(&view.tasks[0]));
    }

    #[test]
    fn task_event_keeps_a_long_raw_cause_through_the_recovery_summary() {
        let mut view = view::View::default();
        view.apply(&serde_json::json!({
            "seq": 1,
            "kind": "plan_proposed",
            "payload": {
                "plan": "plan-7",
                "tasks": [{"id": "task-7", "title": "Ship safely", "change_kind": "code"}]
            }
        }));
        let reason = format!("full raw cause 👩‍💻\n{} end", "界🙂".repeat(400));
        view.apply(&serde_json::json!({
            "seq": 2,
            "kind": "task",
            "payload": {
                "id": "task-7",
                "task": "task-uuid-7",
                "status": "needs_user",
                "round": 1,
                "reason": reason,
            }
        }));

        let Summary::Recovery(projected) = project(&view, "worker:task-7") else {
            panic!("expected recovery summary");
        };
        assert_eq!(projected.reason, reason);
        assert!(projected.reason.ends_with(" end"));
    }

    #[test]
    fn completion_projection_keeps_branch_summary_actions_and_real_result_state() {
        let mut view = view::View::default();
        view.cards.push(TranscriptCard::Completion {
            sent_at: None,
            branch: Some("feature/session".into()),
            summary: "Delivered result".into(),
            actions: [
                SessionActionKind::CreatePr,
                SessionActionKind::MergeWithMain,
                SessionActionKind::ContinueWork,
            ]
            .into_iter()
            .map(|kind| view::CompletionActionState {
                kind,
                result: (kind == SessionActionKind::CreatePr).then_some(
                    SessionActionResult::Failed {
                        details: "gh raw error".into(),
                    },
                ),
            })
            .collect(),
            selected: 1,
            latest_result: Some("Failed: gh raw error".into()),
        });
        let Summary::Completion(projected) = project(&view, "conversation") else {
            panic!("expected completion summary");
        };
        assert_eq!(projected.branch.as_deref(), Some("feature/session"));
        assert_eq!(projected.summary, "Delivered result");
        assert_eq!(projected.selected, 1);
        assert_eq!(
            projected.latest_result.as_deref(),
            Some("Failed: gh raw error")
        );
        assert_eq!(
            projected.actions[0].result,
            Some(SessionActionResult::Failed {
                details: "gh raw error".into()
            })
        );
    }
}
