use super::*;
mod progress;
use progress::{is_running, progress_line};

impl View {
    pub fn comment_state(&self, id: u64) -> &str {
        match self.comments.get(&id) {
            Some(comment) if comment.status != "open" => &comment.status,
            _ => self
                .comment_progress
                .get(&id)
                .map_or("queued", |progress| progress.state.as_str()),
        }
    }

    pub fn comment_error(&self, id: u64) -> Option<&str> {
        self.comment_progress.get(&id)?.error.as_deref()
    }

    /// Keep the comment identity visible even when tool activity changes.
    pub fn comment_activity(&self) -> Option<String> {
        let active: Vec<_> = self
            .comment_progress
            .iter()
            .filter(|(_, progress)| progress.state == "addressing")
            .filter_map(|(id, _)| self.comments.get(id))
            .collect();
        let first = active.first()?;
        let ids = active
            .iter()
            .map(|comment| format!("#{}", comment.id))
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!(
            "Addressing comment {ids} (comment agent): {}",
            clean(&first.text)
        ))
    }

    pub fn orchestrator_activity(&self) -> Option<String> {
        self.comment_activity().or_else(|| self.activity.clone())
    }

    pub(super) fn push(&mut self, style: Style, text: impl Into<String>) {
        self.transcript.push(Entry {
            style,
            text: clean(&text.into()),
        });
    }

    pub fn plan_proposed(&self) -> bool {
        !self.tasks.is_empty() && self.tasks.iter().all(|task| task.status == "proposed")
    }

    /// Where the conversation is: the graph only makes sense once work runs.
    pub fn stage(&self) -> Stage {
        if let Some(stage) = self.workflow_stage() {
            return stage;
        }
        if self.plan_proposed() {
            return Stage::Review;
        }
        if self.tasks.iter().any(|task| is_running(&task.status)) {
            return Stage::Executing;
        }
        self.stage_after_activity()
    }

    fn workflow_stage(&self) -> Option<Stage> {
        match self.workflow_phase.as_deref() {
            Some("interview") => Some(Stage::Interview),
            Some("planning")
                if self.tasks.is_empty()
                    || self.tasks.iter().all(|task| task.status == "cancelled") =>
            {
                Some(Stage::Planning)
            }
            _ => None,
        }
    }

    fn stage_after_activity(&self) -> Stage {
        if self.activity.is_some() && !self.tasks.iter().all(|task| task.status == "approved") {
            return if self.tasks.is_empty() {
                Stage::Interview
            } else {
                Stage::Planning
            };
        }
        if self.tasks.is_empty() {
            return if self.has_started() {
                Stage::Interview
            } else {
                Stage::Fresh
            };
        }
        if self.tasks.iter().all(|task| task.status == "approved") {
            Stage::Done
        } else {
            Stage::Blocked
        }
    }

    fn has_started(&self) -> bool {
        self.activity.is_some()
            || !self.transcript.is_empty()
            || !self.messages.is_empty()
            || !self.cards.is_empty()
    }

    /// Comments on the plan the user has written but not yet sent.
    pub fn draft_plan_comments(&self) -> Vec<&Comment> {
        self.open_plan_comments()
            .into_iter()
            .filter(|c| c.status == "draft")
            .collect()
    }

    /// Unresolved comments targeting the current plan document or whole plan.
    pub fn open_plan_comments(&self) -> Vec<&Comment> {
        let Some(plan_id) = self.plan.as_deref() else {
            return Vec::new();
        };
        let prefix = format!("plan:{plan_id}:");
        let exact = format!("plan:{plan_id}");
        self.comments
            .values()
            .filter(|c| c.status == "open" || c.status == "draft")
            .filter(|c| {
                (c.plan_id.is_none() || c.plan_id.as_deref() == Some(plan_id))
                    && (c.target == "plan"
                        || c.target == exact
                        || c.target.starts_with(&prefix)
                        || c.target.starts_with("plan:line:"))
            })
            .collect()
    }

    /// Whether the plan ended with tasks that can run again.
    pub fn plan_resumable(&self) -> bool {
        self.tasks.iter().all(|task| status::finished(&task.status))
            && self.tasks.iter().any(|task| {
                matches!(
                    task.status.as_str(),
                    "cancelled" | "failed" | "coordinating" | "needs_user" | "skipped" | "waiting"
                )
            })
    }

    /// The agent key of a session role (`worker:<uuid>` → `worker:t1`).
    pub(super) fn agent_key(&self, role: &str) -> String {
        if role == crate::lean::COMMENT_ORCHESTRATOR {
            return ORCHESTRATOR.into();
        }
        match role.split_once(':') {
            Some((who, uuid)) => match self.task_rows.get(uuid) {
                Some(&row) => format!("{who}:{}", self.tasks[row].id),
                None => role.to_owned(),
            },
            None => role.to_owned(),
        }
    }

    /// The agent key of a message's sender or recipient display name.
    pub(super) fn mail_key(name: &str) -> String {
        match name {
            ORCHESTRATOR | PLAN_VERIFIER | AUDITOR | "user" | "runtime" => name.to_owned(),
            crate::lean::PLANNER => crate::lean::PLANNER.to_owned(),
            crate::lean::COMMENT_ORCHESTRATOR => ORCHESTRATOR.to_owned(),
            name => match name.split_once('/') {
                Some((task, _)) => format!("verifier:{task}"),
                None => format!("worker:{name}"),
            },
        }
    }

    /// A short line for what just happened to a task, kept in one card.
    pub(super) fn report_progress(
        &mut self,
        id: &str,
        title: &str,
        was: (&str, u64),
        now: (&str, u64),
        reason: Option<String>,
    ) {
        let Some((line, tone)) = progress_line(self, id, title, was, now, reason) else {
            return;
        };
        self.append_progress_line(line, tone);
    }

    fn append_progress_line(&mut self, line: String, tone: crate::lean::tui::theme::Tone) {
        use crate::lean::tui::theme::Tone;
        if let Some(TranscriptCard::System {
            title,
            text,
            tone: card_tone,
            ..
        }) = self.cards.last_mut()
            && title == "Progress"
        {
            text.push('\n');
            text.push_str(&line);
            let skip = text.lines().count().saturating_sub(PROGRESS_LINES);
            *text = text.lines().skip(skip).collect::<Vec<_>>().join("\n");
            if tone != Tone::Done {
                *card_tone = tone;
            }
            return;
        }
        self.cards.push(TranscriptCard::System {
            title: "Progress".into(),
            meta: String::new(),
            text: line,
            tone,
        });
    }

    pub(super) fn system_card(
        &mut self,
        title: &str,
        text: String,
        tone: crate::lean::tui::theme::Tone,
    ) {
        self.cards.push(TranscriptCard::System {
            title: title.into(),
            meta: String::new(),
            text,
            tone,
        });
    }

    /// Moves the user's messages from one delivery state to the next.
    pub(super) fn advance_messages(&mut self, from: &str, to: &str) {
        for message in &mut self.messages {
            if message.kind == "user" && message.state == from {
                message.state = to.to_owned();
            }
        }
        for card in &mut self.cards {
            if let TranscriptCard::Message(message) = card
                && message.kind == "user"
                && message.state == from
            {
                message.state = to.to_owned();
            }
        }
    }

    pub(super) fn row_of(&mut self, key: &str) -> Option<&mut TaskRow> {
        let id = key.split_once(':')?.1;
        self.tasks.iter_mut().find(|task| task.id == id)
    }
}
