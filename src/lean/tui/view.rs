//! What the screen shows, built only from the conversation's events, so a
//! frontend that attaches later rebuilds the same state.

use super::super::tasks::{
    DETAIL_LIMIT, EFFECT_LIMIT, UNKNOWN_ID_BYTES, bound, feedback_is_current, recovery_outcome,
    resets_card, status,
};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Paragraph};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

mod comments;
mod conversation;
mod header;
mod model;
mod presentation;
mod projection;
pub use header::*;
pub use model::*;
pub use presentation::{
    agent_name, clean, clean_line, draw_min_term_guard, duration, status_label, target_name,
    truncate, wrap,
};
pub(super) use presentation::{bounded_coverage, bounded_landing};

pub const MIN_TERM_WIDTH: u16 = 12;
pub const MIN_TERM_HEIGHT: u16 = 5;
pub const SIDE_BY_SIDE_BREAKPOINT: u16 = 85;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelFocus {
    Chat,
    Graph,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    None,
    Help(super::palette::Help),
    Picker(super::palette::Kind),
    CommentEditor,
    Diff,
    Details,
}

pub const ORCHESTRATOR: &str = "orchestrator";
pub const PLAN_VERIFIER: &str = "plan-verifier";
pub const AUDITOR: &str = "auditor";
/// Lines kept in a progress card before the oldest go.
const PROGRESS_LINES: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub style: Style,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageEntry {
    pub id: String,
    pub sender: String,
    pub target: String,
    pub text: String,
    pub state: String,
    pub kind: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationAction {
    AcceptPlan,
    ResumePlan,
    ReviewDiff,
    CancelWork,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationStatus {
    Pending,
    Confirmed,
    Stale,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConfirmationCard {
    pub id: String,
    pub title: String,
    pub action: ConfirmationAction,
    pub plan_id: Option<String>,
    pub task_id: Option<String>,
    pub candidate: Option<String>,
    pub scope_hash: String,
    pub status: ConfirmationStatus,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptCard {
    Message(MessageEntry),
    Plan {
        plan: String,
        tasks: usize,
        text: String,
    },
    System {
        title: String,
        meta: String,
        text: String,
        tone: super::theme::Tone,
    },
    Comment {
        id: u64,
        target: String,
        text: String,
        status: String,
    },
    Confirmation(ConfirmationCard),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TaskRow {
    pub id: String,
    /// The task's database id, once it is queued.
    pub uuid: Option<String>,
    pub title: String,
    pub summary: Option<String>,
    pub verifier_summary: Option<String>,
    /// Optional section heading in the plan document this task is assigned to.
    pub section: Option<String>,
    pub kind: String,
    pub goal: String,
    pub scope: Vec<String>,
    pub depends_on: Vec<String>,
    /// Peer task ids that can execute in parallel with this task.
    pub parallel_with: Vec<String>,
    pub writes: bool,
    pub focus: String,
    pub risks: Vec<String>,
    pub status: String,
    pub round: u64,
    /// What its worker or verifier is doing right now.
    pub activity: String,
    /// Why it stopped, when it stopped short of approval.
    pub reason: Option<String>,
    pub scope_hash: Option<String>,
    /// The current round's bounded effect, as the daemon reported it in the
    /// task event's feedback. `None`: that round has reported no effect.
    pub effect: Option<String>,
    /// Whether the daemon or this bounded projection cut the attached effect.
    /// No text suffix is interpreted as truncation metadata.
    pub effect_cut: Option<bool>,
    /// The current round's obligation coverage
    /// (`{total, ok, issue, blocked, na, missing, unknown}`). `None`: no
    /// verdict has reported coverage; cards show `?`, never a quiet `0`.
    pub obligations: Option<Value>,
    /// How the current round's result landed
    /// (`{outcome, source, detail, ...}`). `None`: not reported yet.
    pub integration: Option<Value>,
}

/// One runtime agent: the orchestrator, the plan verifier, the auditor,
/// and each task's worker and verifier (shown together on one task card). Keys are `orchestrator`,
/// `plan-verifier`, `auditor`, `worker:t1`, `verifier:t1`.
#[derive(Debug, Default)]
pub struct View {
    pub workflow_phase: Option<String>,
    pub session_branch: Option<String>,
    pub interview_question: Option<(u64, crate::lean::interview::Question)>,
    pub user_decision: bool,
    pub requirements_summary: String,
    pub requirements_approval_run: Option<String>,
    pub requirements_approval_event: u64,
    pub requirements_progress: Option<(bool, String)>,
    latest_user_message: u64,
    pub comment_scopes: HashMap<u64, (Option<u64>, Option<String>)>,
    /// Local projection revision; repeated frames do not rebuild graph layout.
    pub layout_revision: u64,
    /// How many check events arrived: an open Checks tab refetches on change.
    pub check_events: u64,
    /// Latest check event for each plan-local task id; unrelated tasks do not
    /// invalidate an open inspector. Entries are bounded by task count.
    pub task_check_events: HashMap<String, u64>,
    pub transcript: Vec<Entry>,
    pub tasks: Vec<TaskRow>,
    /// The plan the task rows show.
    pub plan: Option<String>,
    /// Full Markdown document of the latest proposed plan.
    pub plan_doc: String,
    /// Latest submitted plan before it is published.
    pub plan_preview: Option<String>,
    /// The orchestrator's current activity, while it works.
    pub activity: Option<String>,
    pub idle: bool,
    pub after: u64,
    pub agents: HashMap<String, Agent>,
    /// Messages between agents by (sender key, recipient key).
    pub links: HashMap<(String, String), Link>,
    /// Every message to or from an agent, by its key, for its details.
    pub mailbox: HashMap<String, Vec<String>>,
    /// The user's comments by id, deleted ones included.
    pub comments: BTreeMap<u64, Comment>,
    pub comment_progress: BTreeMap<u64, CommentProgress>,
    /// The latest review of the plan verifier and the auditor, as lines.
    pub reviews: HashMap<String, Vec<String>>,
    /// The user paused new plan work.
    pub paused: bool,
    /// The conversation's mode, once the daemon said it.
    pub autonomous: Option<bool>,
    /// Structured messages with observed actual states.
    pub messages: Vec<MessageEntry>,
    /// Structured cards for rendering the conversation.
    pub cards: Vec<TranscriptCard>,
    pub plan_scope_hash: Option<String>,
    pub conversation_scope_hash: Option<String>,
    /// Agent key of each running turn.
    roles: HashMap<String, String>,
    comment_turns: HashMap<String, Vec<u64>>,
    /// Recipient of each shown message to an agent, by mail id.
    mail: HashMap<u64, String>,
    /// Task row index by the task's database id.
    task_rows: HashMap<String, usize>,
}

pub fn dim() -> Style {
    Style::default().fg(super::theme::ayu::DIM)
}

impl View {
    pub fn apply(&mut self, event: &Value) {
        projection::apply(self, event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn turn_branch_is_replayed_and_cleared_when_unreported() {
        let mut view = View::default();
        view.apply(&event(
            1,
            Some("t"),
            "turn_started",
            json!({"role":"orchestrator", "branch":"feature/tasks"}),
        ));
        assert_eq!(
            view.agents[ORCHESTRATOR].branch.as_deref(),
            Some("feature/tasks")
        );
        view.apply(&event(
            2,
            Some("next"),
            "turn_started",
            json!({"role":"orchestrator"}),
        ));
        assert_eq!(view.agents[ORCHESTRATOR].branch, None);
    }

    fn event(seq: u64, turn: Option<&str>, kind: &str, payload: Value) -> Value {
        json!({"seq": seq, "turn_id": turn, "kind": kind, "payload": payload})
    }

    #[test]
    fn live_graph_edits_preserve_agent_history_and_existing_task_state() {
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "plan_proposed",
            serde_json::json!({"plan":"p","tasks":[
                {"id":"implementation","title":"Implement"},
                {"id":"security","title":"Security","depends_on":["implementation"]}
            ]}),
        ));
        view.tasks[0].status = "working".into();
        view.tasks[0].round = 2;
        view.agents.insert(
            "worker:implementation".into(),
            Agent {
                provider: "codex".into(),
                turns: 3,
                ..Agent::default()
            },
        );
        view.apply(&event(2,None,"graph_task",serde_json::json!({"plan":"p","id":"extra-uuid","status":"queued","round":0,"spec":{"id":"extra","title":"Extra","depends_on":["implementation"]}})));
        view.apply(&event(3,None,"graph_task",serde_json::json!({"plan":"p","id":"security-uuid","status":"queued","round":0,"spec":{"id":"security","title":"Security","depends_on":["extra"]}})));
        view.apply(&event(4,None,"graph_updated",serde_json::json!({"plan":"p","text":"Insert extra work before security","scope_hash":"new","order":["implementation","extra","security"]})));
        assert_eq!(
            view.tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            ["implementation", "extra", "security"]
        );
        assert_eq!(view.tasks[0].status, "working");
        assert_eq!(view.tasks[0].round, 2);
        assert_eq!(view.tasks[2].depends_on, ["extra"]);
        assert_eq!(view.agents["worker:implementation"].turns, 3);
        assert_eq!(view.task_rows["extra-uuid"], 1);
        assert_eq!(view.plan_scope_hash.as_deref(), Some("new"));
    }

    #[test]
    fn requirements_approval_survives_replay_until_its_run_changes() {
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "ledger_approval_required",
            json!({"run":"r1"}),
        ));
        assert_eq!(view.requirements_approval_run.as_deref(), Some("r1"));
        view.apply(&event(2, None, "ledger_changed", json!({"run":"r2"})));
        assert_eq!(view.requirements_approval_run.as_deref(), Some("r1"));
        view.apply(&event(3, None, "ledger_changed", json!({"run":"r1"})));
        assert!(view.requirements_approval_run.is_none());
    }

    #[test]
    fn requirements_failure_shows_raw_reason_and_retry_guidance() {
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "requirements_failed",
            json!({
                "error": "entry p has unknown or incompatible parent r",
                "retry": "Send a message to retry planning."
            }),
        ));
        assert!(view.cards.iter().any(|card| matches!(card,
            TranscriptCard::System { title, text, tone: super::super::theme::Tone::Failed, .. }
                if title == "Requirements preparation failed"
                    && text.contains("incompatible parent r")
                    && text.contains("Send a message to retry planning.")
        )));
    }

    #[test]
    fn requirements_rejection_keeps_automatic_repair_in_progress() {
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "requirements_failed",
            json!({"error":"missing criterion", "retrying":true,
                "retry":"Regenerating the plan automatically."}),
        ));
        assert_eq!(
            view.requirements_progress,
            Some((false, "Regenerating the plan automatically.".into()))
        );
        assert!(!view.cards.iter().any(|card| matches!(card,
            TranscriptCard::System { title, .. } if title == "Requirements preparation failed"
        )));
    }

    #[test]
    fn planning_chat_shows_progress_instead_of_agent_plan_prose() {
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "session_started",
            json!({"branch":"feature/start"}),
        ));
        view.apply(&event(2, None, "workflow", json!({"phase":"planning"})));
        view.apply(&event(
            3,
            None,
            "message",
            json!({"author":"orchestrator","text":"Internal plan prose"}),
        ));
        view.apply(&event(
            4,
            None,
            "planning_progress",
            json!({"text":"Reviewing the plan"}),
        ));
        assert_eq!(view.session_branch.as_deref(), Some("feature/start"));
        assert!(view.messages.is_empty());
        assert!(
            !view
                .transcript
                .iter()
                .any(|entry| entry.text.contains("Internal plan prose"))
        );
        assert!(view.cards.iter().any(|card| matches!(card, TranscriptCard::System { text, .. } if text == "Reviewing the plan")));
        view.apply(&event(
            5,
            None,
            "workflow",
            json!({"phase":"implementation"}),
        ));
        view.apply(&event(
            6,
            None,
            "message",
            json!({"author":"orchestrator","text":"Implementation result"}),
        ));
        view.apply(&event(
            7,
            None,
            "message",
            json!({"author":"runtime","text":"Internal planning handoff","internal":true}),
        ));
        assert!(
            !view
                .transcript
                .iter()
                .any(|entry| entry.text.contains("Internal planning handoff"))
        );
        assert_eq!(view.messages.len(), 1);
    }

    #[test]
    fn workflow_replay_guides_interview_then_planning_without_task_rows() {
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "workflow",
            json!({"phase":"interview","summary":""}),
        ));
        assert_eq!(view.stage(), Stage::Interview);
        view.apply(&event(
            2,
            None,
            "workflow",
            json!({"phase":"interview","summary":"Goal and criteria"}),
        ));
        assert!(!view.requirements_summary.is_empty());
        view.apply(&event(
            3,
            None,
            "message",
            json!({"author":"user","text":"Change scope"}),
        ));
        assert!(view.requirements_summary.is_empty());
        view.apply(&event(4, None, "workflow", json!({"phase":"planning"})));
        assert_eq!(view.stage(), Stage::Planning);
    }

    fn staged(statuses: &[&str], activity: bool) -> View {
        let mut view = View::default();
        view.cards.push(TranscriptCard::System {
            title: "t".into(),
            meta: String::new(),
            text: "x".into(),
            tone: crate::lean::tui::Tone::Queued,
        });
        view.tasks = statuses
            .iter()
            .enumerate()
            .map(|(i, status)| TaskRow {
                id: format!("t{i}"),
                status: (*status).into(),
                ..TaskRow::default()
            })
            .collect();
        view.activity = activity.then(|| "working".to_owned());
        view
    }

    #[test]
    fn task_changes_become_short_lines_in_one_progress_card() {
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "plan_proposed",
            json!({"tasks": [{"id": "t1", "title": "Storage"}, {"id": "t2", "title": "API"}]}),
        ));
        let task = |seq, id: &str, status: &str, round, reason: Option<&str>| {
            event(
                seq,
                None,
                "task",
                json!({"id": id, "task": format!("u-{id}"), "status": status,
                       "round": round, "reason": reason}),
            )
        };
        for e in [
            task(2, "t1", "queued", 0, None),
            task(3, "t1", "working", 1, None),
            task(4, "t1", "verifying", 1, None),
            task(5, "t1", "working", 2, None),
            // The same state again says nothing new.
            task(6, "t1", "working", 2, None),
            task(7, "t1", "approved", 2, None),
            task(
                8,
                "t2",
                "needs_user",
                3,
                Some("the verifier repeated itself"),
            ),
        ] {
            view.apply(&e);
        }
        let progress: Vec<&TranscriptCard> = view
            .cards
            .iter()
            .filter(|c| matches!(c, TranscriptCard::System { title, .. } if title == "Progress"))
            .collect();
        assert_eq!(progress.len(), 1);
        let TranscriptCard::System { text, tone, .. } = progress[0] else {
            unreachable!()
        };
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "↺ t1 Storage: round 2 after the verifier asked for changes",
                "✓ t1 Storage approved (1/2)",
                "! t2 API needs your decision — the verifier repeated itself",
            ]
        );
        assert_eq!(*tone, crate::lean::tui::Tone::Waiting);

        // Another card in between starts a new progress card.
        view.cards.push(TranscriptCard::System {
            title: "Notice".into(),
            meta: String::new(),
            text: "x".into(),
            tone: crate::lean::tui::Tone::Queued,
        });
        view.apply(&task(9, "t2", "failed", 3, None));
        let count = view
            .cards
            .iter()
            .filter(|c| matches!(c, TranscriptCard::System { title, .. } if title == "Progress"))
            .count();
        assert_eq!(count, 2);
    }

    #[test]
    fn execution_decision_survives_workflow_replay_and_requires_its_own_answer() {
        let mut view = View {
            requirements_summary: "Approved requirements".into(),
            ..View::default()
        };
        let question = json!({"id":"metadata","context":"Scope decision required","question":"Allow metadata?","options":[{"label":"Allow"},{"label":"Reject"}]});
        view.apply(&event(
            1,
            None,
            "workflow",
            json!({"phase":"implementation"}),
        ));
        view.apply(&event(2, None, "user_decision_requested", question));
        assert!(view.user_decision);
        assert!(view.interview_question.is_some());
        view.apply(&event(
            3,
            None,
            "workflow",
            json!({"phase":"implementation"}),
        ));
        assert!(view.interview_question.is_some());
        view.apply(&event(
            4,
            None,
            "message",
            json!({"author":"user","seq":4,"text":"Interview answer (another)\nAnswer: Allow"}),
        ));
        assert!(view.user_decision);
        view.apply(&event(5, None, "message", json!({"author":"user","seq":5,"text":"Interview answer (metadata)\nQuestion: Allow metadata?\nAnswer: Reject"})));
        assert!(!view.user_decision);
        assert!(view.interview_question.is_none());
        assert_eq!(view.workflow_phase.as_deref(), Some("implementation"));
        assert!(view.tasks.is_empty());
    }

    #[test]
    fn tool_rejections_are_visible_without_finishing_the_turn() {
        let mut view = View::default();
        view.apply(&event(
            1,
            Some("t"),
            "turn_started",
            json!({"role":"orchestrator","provider":"codex"}),
        ));
        view.apply(&event(
            2,
            Some("t"),
            "agent_tool_rejected",
            json!({"tool":"finish_turn","error":"request: unknown question ID"}),
        ));
        assert!(view.agents[ORCHESTRATOR].running);
        assert_eq!(view.agents[ORCHESTRATOR].turns, 0);
        assert!(view.cards.iter().any(|card| matches!(card, TranscriptCard::System { title, text, .. } if title == "Agent tool rejected" && text.contains("request: unknown question ID"))));
        view.apply(&event(
            3,
            Some("t"),
            "turn_finished",
            json!({"role":"orchestrator","status":"done"}),
        ));
        assert!(!view.agents[ORCHESTRATOR].running);
        assert_eq!(view.agents[ORCHESTRATOR].turns, 1);
    }

    #[test]
    fn a_users_message_is_sent_then_delivered_then_answered() {
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "message",
            json!({"author": "user", "text": "hi"}),
        ));
        let states = |view: &View| -> Vec<String> {
            view.cards
                .iter()
                .filter_map(|c| match c {
                    TranscriptCard::Message(m) if m.kind == "user" => Some(m.state.clone()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(states(&view), ["sent"]);
        view.apply(&event(
            2,
            Some("t"),
            "turn_started",
            json!({"role": "orchestrator", "provider": "codex"}),
        ));
        assert_eq!(states(&view), ["delivered"]);
        view.apply(&event(
            3,
            Some("t"),
            "turn_finished",
            json!({"role": "orchestrator", "status": "done"}),
        ));
        assert_eq!(states(&view), ["answered"]);
        assert_eq!(view.messages[0].state, "answered");
    }

    #[test]
    fn plan_comments_stay_drafts_until_they_are_sent_for_review() {
        let mut view = View {
            plan: Some("p1".into()),
            ..View::default()
        };
        let comment = |status: &str, message: u64| {
            json!({"id": 1, "plan_id": "p1", "target": "plan:p1:line:2", "label": "line 2",
                   "text": "remark", "status": status, "message": message})
        };
        view.apply(&event(1, None, "comment", comment("draft", 0)));
        assert_eq!(view.comment_state(1), "draft");
        assert_eq!(view.draft_plan_comments().len(), 1);
        // A draft is not queued for any agent, but it blocks accepting.
        assert!(view.comment_progress.is_empty());
        assert_eq!(view.open_plan_comments().len(), 1);

        view.apply(&event(2, None, "comment", comment("open", 9)));
        assert_eq!(view.comment_state(1), "queued");
        assert!(view.draft_plan_comments().is_empty());
        let states: Vec<&str> = view
            .cards
            .iter()
            .filter_map(|c| match c {
                TranscriptCard::Comment { status, .. } => Some(status.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(states, ["draft", "sent"]);
    }

    #[test]
    fn a_comment_answered_in_words_is_marked_answered_not_addressed() {
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "comment",
            json!({"id": 4, "plan_id": "p", "target": "plan:p", "label": "plan", "text": "why?",
                   "status": "open", "message": 3}),
        ));
        view.apply(&event(
            2,
            Some("c"),
            "turn_started",
            json!({"role": "comment-orchestrator", "provider": "codex", "comments": [4]}),
        ));
        view.apply(&event(
            3,
            Some("c"),
            "turn_finished",
            json!({"role": "comment-orchestrator", "status": "done"}),
        ));
        assert_eq!(view.comment_state(4), "addressed");
        view.apply(&event(4, None, "comment_replied", json!({"comments": [4]})));
        assert_eq!(view.comment_state(4), "answered");
    }

    #[test]
    fn planner_turns_keep_their_own_agent_and_status() {
        let mut view = View::default();
        view.apply(&event(
            1,
            Some("o"),
            "turn_started",
            json!({"role":"orchestrator","provider":"codex"}),
        ));
        view.apply(&event(
            2,
            Some("p"),
            "turn_started",
            json!({"role":"planner","provider":"codex"}),
        ));
        assert!(view.agents["planner"].running);
        assert!(view.agents["orchestrator"].running);
        assert!(
            view.progress()
                .iter()
                .any(|line| line.starts_with("planner:"))
        );
        view.apply(&event(
            3,
            Some("p"),
            "turn_finished",
            json!({"role":"planner","status":"done"}),
        ));
        assert!(!view.agents["planner"].running);
        assert!(view.agents["orchestrator"].running);
        assert_eq!(View::mail_key("planner"), "planner");
    }

    #[test]
    fn the_fallback_for_parallel_tasks_follows_chains_of_dependencies() {
        let mut view = View::default();
        // t3 depends on t2 which depends on t1: none of them can overlap.
        // t4 is free of all of them.
        view.apply(&event(
            1,
            None,
            "plan_proposed",
            json!({"tasks": [
                {"id": "t1"}, {"id": "t2", "depends_on": ["t1"]},
                {"id": "t3", "depends_on": ["t2"]}, {"id": "t4"}]}),
        ));
        let peers = |id: &str| -> Vec<String> {
            view.tasks
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .parallel_with
                .clone()
        };
        assert_eq!(peers("t1"), ["t4"]);
        assert_eq!(peers("t3"), ["t4"]);
        assert_eq!(peers("t4"), ["t1", "t2", "t3"]);
    }

    #[test]
    fn mail_for_the_reviewers_stays_with_the_reviewers() {
        assert_eq!(View::mail_key("plan-verifier"), "plan-verifier");
        assert_eq!(View::mail_key("auditor"), "auditor");
        assert_eq!(View::mail_key("runtime"), "runtime");
        assert_eq!(View::mail_key("comment-orchestrator"), "orchestrator");
        assert_eq!(View::mail_key("t1"), "worker:t1");
        assert_eq!(View::mail_key("t1/verifier"), "verifier:t1");
    }

    #[test]
    fn workflow_stages_cover_planning_execution_and_replanning() {
        assert_eq!(View::default().stage(), Stage::Fresh);
        for (statuses, activity, stage) in [
            (vec![], false, Stage::Interview),
            (vec![], true, Stage::Interview),
            (vec!["proposed"], false, Stage::Review),
            (vec!["working", "queued"], true, Stage::Executing),
            // A rejected plan returns to planning.
            (vec!["cancelled"], true, Stage::Planning),
            (vec!["cancelled"], false, Stage::Blocked),
            (vec!["approved", "needs_user"], false, Stage::Blocked),
            (vec!["approved"], false, Stage::Done),
            // The orchestrator reporting on a finished plan keeps the graph.
            (vec!["approved"], true, Stage::Done),
        ] {
            let view = staged(&statuses, activity);
            assert_eq!(view.stage(), stage, "{statuses:?} {activity}");
        }
    }

    #[test]
    fn plan_projection_preserves_summaries_and_legacy_empty_values() {
        for summary in [
            Value::Null,
            json!(" \n\t "),
            json!("\x1b[31m\x1b[0m"),
            json!("Improve storage."),
        ] {
            let mut view = View::default();
            view.apply(&event(
                1,
                None,
                "plan_proposed",
                json!({"tasks": [{
                    "id": "t1", "title": "Storage", "summary": summary, "goal": "Full goal",
                    "verifier": {"summary": summary, "focus": "Full focus", "risks": ["Data loss"]}
                }]}),
            ));
            // Summaries arrive sanitized: blank and escape-only legacy values
            // fold to an empty line, which the views read as not written.
            let expected = summary.as_str().map(clean_line);
            assert_eq!(view.tasks[0].summary, expected);
            assert_eq!(view.tasks[0].verifier_summary, expected);
            assert_eq!(view.tasks[0].goal, "Full goal");
            assert_eq!(view.tasks[0].focus, "Full focus");
            assert_eq!(view.tasks[0].risks, ["Data loss"]);
        }
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "plan_proposed",
            json!({"tasks": [{"id": "old"}]}),
        ));
        assert!(view.tasks[0].summary.is_none());
        assert!(view.tasks[0].verifier_summary.is_none());
    }

    #[test]
    fn the_view_follows_a_conversation_and_its_tasks() {
        let mut view = View::default();
        for event in [
            event(
                1,
                None,
                "message",
                json!({"author": "user", "text": "analysis"}),
            ),
            event(
                2,
                Some("o1"),
                "turn_started",
                json!({"role": "orchestrator", "provider": "codex"}),
            ),
            event(
                4,
                None,
                "plan_proposed",
                json!({"tasks": [{"id": "t1", "title": "Store", "writes": false}]}),
            ),
            event(
                5,
                None,
                "task",
                json!({"id": "t1", "task": "uuid1", "title": "Store", "status": "working", "round": 1}),
            ),
            event(
                6,
                Some("w1"),
                "turn_started",
                json!({"role": "worker:uuid1", "provider": "agy"}),
            ),
        ] {
            view.apply(&event);
        }
        assert_eq!(view.after, 6);
        assert_eq!(
            view.activity.as_deref(),
            Some("orchestrator (codex) is working…")
        );
        assert_eq!(view.tasks[0].status, "working");
        assert_eq!(view.tasks[0].activity, "worker (agy)");
        assert!(!view.plan_proposed());
        view.apply(&event(
            8,
            Some("o1"),
            "turn_finished",
            json!({"status": "failed", "error": "limit"}),
        ));
        assert_eq!(view.activity, None);
        assert!(view.transcript.last().unwrap().text.contains("limit"));
    }

    #[test]
    fn reviews_and_audits_are_shown_with_their_findings() {
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "plan_proposed",
            json!({"plan": "p1", "tasks": [{"id": "t1", "title": "A"}],
                   "review": {"approved": false, "error": "login required"},
                   "automatic": false, "waiting": "the plan verifier did not approve this plan"}),
        ));
        view.apply(&event(
            2,
            None,
            "audit",
            json!({"plan": "p1", "approved": false,
                   "findings": [{"severity": "high", "issue": "README missing"}]}),
        ));
        let text: Vec<&str> = view
            .transcript
            .iter()
            .map(|entry| entry.text.as_str())
            .collect();
        assert!(text.contains(&"⚠ plan verifier failed: login required"));
        assert!(
            text.iter()
                .any(|line| line.contains("Waiting for you") && line.contains("Ctrl+A"))
        );
        assert!(text.contains(&"⚠ audit: issues found"));
        assert!(text.contains(&"  - [high] README missing"));
        assert!(view.plan_proposed());
    }

    #[test]
    fn messages_to_agents_show_their_delivery() {
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "mail",
            json!({"id": 7, "sender": "user", "recipient": "t2", "kind": "user_instruction", "text": "use UTF-8"}),
        ));
        view.apply(&event(
            2,
            None,
            "mail",
            json!({"id": 8, "sender": "user", "recipient": "orchestrator", "kind": "copy", "text": "x"}),
        ));
        view.apply(&event(
            3,
            Some("turn"),
            "mail_delivered",
            json!({"ids": [7, 8]}),
        ));
        let text: Vec<&str> = view
            .transcript
            .iter()
            .map(|entry| entry.text.as_str())
            .collect();
        assert_eq!(
            text,
            [
                "✉ You → t2 (user_instruction): use UTF-8 · waiting for recipient's next turn",
                "✓ t2 received message"
            ]
        );
    }

    #[test]
    fn wrapping_breaks_words_and_long_runs() {
        assert_eq!(wrap("cat on mat", 7), ["cat on ", "mat"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("a\n\nb", 10), ["a", "", "b"]);
        assert_eq!(clean("a\u{1b}[31mb\tc"), "ab    c");
        assert_eq!(wrap("a\u{1b}]0;x\u{7}b", 10), ["ab"]);
    }

    #[test]
    fn messages_and_cards_are_stored_structurally_with_actual_states() {
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "message",
            json!({"author": "user", "text": "start"}),
        ));
        assert_eq!(view.messages.len(), 1);
        assert_eq!(view.messages[0].sender, "You");
        assert_eq!(view.messages[0].target, "orchestrator");
        assert_eq!(view.messages[0].state, "sent");

        view.apply(&event(
            2,
            None,
            "mail",
            json!({"id": 101, "sender": "user", "recipient": "t1", "kind": "instruction", "text": "do it"}),
        ));
        assert_eq!(view.messages.len(), 2);
        assert_eq!(view.messages[1].id, "101");
        assert_eq!(view.messages[1].state, "waiting for turn");

        view.apply(&event(
            3,
            Some("turn"),
            "mail_delivered",
            json!({"ids": [101]}),
        ));
        assert_eq!(view.messages[1].state, "delivered");
        if let TranscriptCard::Message(msg) = &view.cards[1] {
            assert_eq!(msg.state, "delivered");
        } else {
            panic!("Expected message card");
        }
    }

    #[test]
    fn agents_keep_their_sessions_and_messages_between_them() {
        let mut view = View::default();
        for event in [
            event(
                1,
                None,
                "plan_proposed",
                json!({"plan": "p", "tasks": [{"id": "t1", "title": "A"}, {"id": "t2", "title": "B", "depends_on": ["t1"]}]}),
            ),
            event(
                2,
                None,
                "task",
                json!({"id": "t1", "task": "u1", "status": "verifying", "round": 1}),
            ),
            event(
                3,
                Some("v1"),
                "turn_started",
                json!({"role": "verifier:u1", "provider": "agy"}),
            ),
            event(
                5,
                None,
                "mail",
                json!({"id": 1, "sender": "t1/verifier", "recipient": "t2", "kind": "finding", "text": "x"}),
            ),
            event(
                6,
                None,
                "audit",
                json!({"plan": "p", "approved": true, "findings": []}),
            ),
        ] {
            view.apply(&event);
        }
        assert_eq!(view.tasks[1].depends_on, ["t1"]);
        let verifier = &view.agents["verifier:t1"];
        assert!(verifier.running);
        assert_eq!(verifier.provider, "agy");
        let link = &view.links[&("verifier:t1".to_owned(), "worker:t2".to_owned())];
        assert_eq!(link.count, 1);
        assert_eq!(view.mailbox["worker:t2"], ["t1/verifier → t2 (finding): x"]);
        assert_eq!(view.reviews[AUDITOR], ["✓ auditor: no issues"]);
        assert_eq!(view.progress(), ["t1 verifying: verifier (agy)"]);
    }

    #[test]
    fn admin_progress_names_a_starting_turn_as_no_activity_yet() {
        let mut view = View::default();
        view.agents.insert(
            PLAN_VERIFIER.into(),
            Agent {
                running: true,
                activity: "starting turn".into(),
                ..Agent::default()
            },
        );
        // The shared rule (taskinfo::doing) says the placeholder is not
        // work: the step is named, and its doing stays an honest "…".
        assert_eq!(view.progress(), ["plan verifier: …"]);
        view.agents.insert(
            AUDITOR.into(),
            Agent {
                running: true,
                activity: "auditing  two  plans".into(),
                ..Agent::default()
            },
        );
        assert_eq!(
            view.progress(),
            ["plan verifier: …", "auditor: auditing two plans"],
            "live activity is one normalized line, like every other view"
        );
        // A stopped turn never shows as live activity either.
        if let Some(auditor) = view.agents.get_mut(AUDITOR) {
            auditor.running = false;
        }
        assert_eq!(view.progress(), ["plan verifier: …"]);
    }

    #[test]
    fn comments_are_listed_on_their_element_and_marked_stale_on_a_new_plan() {
        let mut view = View::default();
        let plan = |seq, id: &str| {
            event(
                seq,
                None,
                "plan_proposed",
                json!({"plan": id, "tasks": [{"id": "t1", "title": "A"}]}),
            )
        };
        view.apply(&plan(1, "p1"));
        view.apply(&event(
            2,
            None,
            "message",
            json!({"author": "user", "text": "[user comment #1 on the worker of task t1] x"}),
        ));
        for (seq, id, target, status) in [
            (3, 1, "worker:t1", "open"),
            (4, 2, "diff:t1:src/a.rs:3", "open"),
            (5, 3, "verifier:t1", "open"),
            (6, 3, "verifier:t1", "deleted"),
        ] {
            view.apply(&event(
                seq,
                None,
                "comment",
                json!({"id": id, "plan_id": "p1", "target": target, "label": "l", "text": "x",
                       "status": status, "message": 2}),
            ));
        }
        let ids = |key| -> Vec<u64> { view.comments_on(key).iter().map(|c| c.id).collect() };
        assert_eq!(
            ids("worker:t1"),
            [2, 1],
            "a diff's comments count for its worker"
        );
        assert!(ids("verifier:t1").is_empty());
        let text: Vec<&str> = view.transcript[2..]
            .iter()
            .map(|e| e.text.as_str())
            .collect();
        assert_eq!(
            text,
            [
                "✎ #1 to t1 worker: x",
                "✎ #2 to t1 diff src/a.rs:3: x",
                "✎ #3 to t1 verifier: x",
                "✎ #3 deleted"
            ],
            "the comment's message is not shown twice"
        );
        assert!(!view.stale(&view.comments[&1]));
        view.apply(&plan(7, "p2"));
        assert!(view.stale(&view.comments[&1]));
    }

    #[test]
    fn shared_worker_node_aggregates_comments_from_both_roles() {
        let mut view = View::default();
        for (id, target) in [
            (1, "worker:t1"),
            (2, "verifier:t1"),
            (3, "diff:t1:src/a.rs:1"),
            (4, "check:t1:V01"),
            (5, "check:t10:V01"),
        ] {
            view.apply(&event(
                id,
                None,
                "comment",
                json!({"id":id,"target":target,"text":"evidence","status":"open"}),
            ));
        }
        let ids: Vec<_> = view
            .comments_on("worker:t1")
            .iter()
            .map(|comment| comment.id)
            .collect();
        assert_eq!(ids, [4, 3, 2, 1]);
        let ids: Vec<_> = view
            .comments_on("verifier:t1")
            .iter()
            .map(|comment| comment.id)
            .collect();
        assert_eq!(
            ids,
            [4, 2],
            "legacy verifier addresses keep their role-specific meaning"
        );
    }

    #[test]
    fn turn_times_add_up_only_when_known() {
        let mut view = View::default();
        let at = |seq, id: &str, kind: &str, ms: i64| {
            let mut event = event(
                seq,
                Some(id),
                kind,
                json!({"role": "worker:uuid1", "status": "done"}),
            );
            event["created_ms"] = json!(ms);
            event
        };
        for event in [
            at(1, "a", "turn_started", 0),
            at(2, "a", "turn_finished", 5_000),
            at(3, "b", "turn_started", 10_000),
            at(4, "b", "turn_finished", 85_000),
        ] {
            view.apply(&event);
        }
        let agent = view.agents.values().next().unwrap();
        assert_eq!(agent.turns, 2);
        assert_eq!((agent.busy_ms, agent.last_ms), (75_000, Some(75_000)));
        assert_eq!(duration(agent.busy_ms), "1 min 15 s");
    }

    #[test]
    fn token_use_is_unknown_because_an_interactive_cli_reports_none() {
        let mut view = View::default();
        for event in [
            event(
                1,
                Some("a"),
                "turn_started",
                json!({"role": "worker:uuid1", "provider": "codex"}),
            ),
            event(2, Some("a"), "turn_finished", json!({"status": "done"})),
        ] {
            view.apply(&event);
        }
        let agent = view.agents.values().next().unwrap();
        assert_eq!(agent.turns, 1);
        assert_eq!(agent.tokens(), "unavailable (CLI did not report them)");
    }

    #[test]
    fn ac_resp_01_responsive_breakpoint_compact_switch() {
        // Breakpoint: width 84 (< 85) -> compact mode with banner
        let header_chat = render_header(
            "proj",
            "conv-12345",
            Some(true),
            "ready",
            PanelFocus::Chat,
            84,
        );
        let chat_text: String = header_chat
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(chat_text.contains("[Chat]"));
        assert!(chat_text.contains("Team"));
        assert!(chat_text.contains("(Tab: switch)"));

        let header_graph = render_header(
            "proj",
            "conv-12345",
            Some(true),
            "ready",
            PanelFocus::Graph,
            84,
        );
        let graph_text: String = header_graph
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(graph_text.contains("Chat"));
        assert!(graph_text.contains("[Team]"));
        assert!(graph_text.contains("(Tab: switch)"));
    }

    #[test]
    fn ac_resp_02_wide_header() {
        // Wide: width 85 (>= 85) -> side-by-side mode, no compact toggle badge
        let header_wide = render_header(
            "proj",
            "conv-12345",
            Some(true),
            "ready",
            PanelFocus::Chat,
            85,
        );
        let wide_text: String = header_wide
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(!wide_text.contains("[Chat]"));
        assert!(!wide_text.contains("(Tab: switch)"));
        assert!(wide_text.contains("proj"));
        assert!(wide_text.contains("conv-123"));
        assert!(wide_text.contains("autonomous"));
        assert!(wide_text.contains("ready"));
    }

    #[test]
    fn ac_resp_03_minimum_terminal_guard() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        // Terminal size 11x4 is smaller than 12x5 guard floor
        let backend = TestBackend::new(11, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut guard_triggered = false;
        terminal
            .draw(|frame| {
                guard_triggered = draw_min_term_guard(frame, frame.area());
            })
            .unwrap();
        assert!(guard_triggered);
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
        let norm = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(norm.contains("Terminal too small (min. 12x5 required)"));

        // Terminal size 12x5 meets minimum guard
        let backend = TestBackend::new(12, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut guard_triggered_ok = false;
        terminal
            .draw(|frame| {
                guard_triggered_ok = draw_min_term_guard(frame, frame.area());
            })
            .unwrap();
        assert!(!guard_triggered_ok);
    }

    #[test]
    fn ac_copy_02_status_strings() {
        let header = render_header(
            "test-proj",
            "conv-12345",
            Some(false),
            "ready",
            PanelFocus::Chat,
            120,
        );
        let text: String = header.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("test-proj"));
        assert!(!text.contains("antislop"));
        assert!(text.contains("supervised"));
        assert!(text.contains("ready"));

        let header_auto = render_header(
            "test-proj",
            "conv-12345",
            Some(true),
            "tasks in progress…",
            PanelFocus::Chat,
            120,
        );
        let text_auto: String = header_auto
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(text_auto.contains("autonomous"));
        assert!(text_auto.contains("tasks in progress…"));
    }

    #[test]
    fn main_footer_hints_prioritized_by_width() {
        // Chat hints
        let wide = main_footer_hint(PanelFocus::Chat, false, false, None, 120);
        assert!(wide.contains("Alt+Enter newline"));
        assert!(wide.contains("F1 help"));

        let medium = main_footer_hint(PanelFocus::Chat, false, false, None, 80);
        assert!(medium.contains("Ctrl+P"));

        let narrow = main_footer_hint(PanelFocus::Chat, false, false, None, 55);
        assert!(narrow.contains("Esc"));
        assert!(!narrow.contains("Alt+Enter"));

        // Graph hints
        let graph_wide = main_footer_hint(PanelFocus::Graph, false, false, None, 120);
        assert!(graph_wide.contains("arrows select"));
        assert!(graph_wide.contains("d diff"));

        let graph_narrow = main_footer_hint(PanelFocus::Graph, false, false, None, 55);
        assert!(graph_narrow.contains("Tab chat"));
    }

    #[test]
    fn transcript_empty_state_text() {
        let lines = transcript_empty_state(80);
        let text = lines[0].spans[0].content.as_ref();
        assert_eq!(text, "No messages in this conversation.");
    }
}
