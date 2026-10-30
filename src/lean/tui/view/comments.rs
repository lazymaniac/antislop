use super::*;
use serde_json::Value;

impl View {
    pub fn check_comments(&self, task: &str, check: &str) -> Vec<&Comment> {
        let target = format!("check:{task}:{check}");
        self.comments
            .values()
            .filter(|comment| comment.status != "deleted" && comment.target == target)
            .collect()
    }

    pub fn comments_on(&self, key: &str) -> Vec<&Comment> {
        let diff = key
            .strip_prefix("worker:")
            .map(|task| format!("diff:{task}"));
        let verifier = key
            .strip_prefix("worker:")
            .map(|task| format!("verifier:{task}"));
        let checks = key
            .strip_prefix("verifier:")
            .or_else(|| key.strip_prefix("worker:"))
            .map(|task| format!("check:{task}:"));
        self.comments
            .values()
            .rev()
            .filter(|comment| comment.status != "deleted")
            .filter(|comment| {
                comment.target == key
                    || verifier
                        .as_ref()
                        .is_some_and(|target| comment.target == *target)
                    || checks
                        .as_ref()
                        .is_some_and(|checks| comment.target.starts_with(checks))
                    || diff.as_ref().is_some_and(|diff| {
                        comment.target == *diff || comment.target.starts_with(&format!("{diff}:"))
                    })
            })
            .collect()
    }

    /// Whether a comment was made on an earlier plan than the one shown.
    pub fn stale(&self, comment: &Comment) -> bool {
        comment.plan_id.is_some() && comment.plan_id != self.plan
    }

    pub(super) fn review(&mut self, key: &str, review: &Value) {
        self.agents.entry(key.into()).or_default().activity = review_word(review);
        self.reviews.insert(
            key.into(),
            crate::lean::cli::review_lines(&agent_name(key), review),
        );
    }

    /// What is happening now, one line per active step, for the progress
    /// overlay. Waiting and idle states are named, never shown as activity.
    pub fn progress(&self) -> Vec<String> {
        let mut lines = initial_progress(self);
        lines.extend(task_progress(&self.tasks));
        lines.extend(agent_progress(self));
        if lines.is_empty() {
            lines.push(self.waiting_progress());
        }
        lines
    }

    fn waiting_progress(&self) -> String {
        if !self.idle {
            if matches!(
                self.stage(),
                Stage::Interview | Stage::Planning | Stage::Review
            ) {
                "waiting for the orchestrator to start".into()
            } else {
                "tasks in progress…".into()
            }
        } else if self.plan_proposed() {
            "waiting for you: plan to accept".into()
        } else if self.plan_resumable() {
            "waiting for you: plan to resume".into()
        } else {
            "idle".into()
        }
    }
}

fn initial_progress(view: &View) -> Vec<String> {
    let mut lines = Vec::new();
    if view.paused {
        lines.push("⏸ paused: new steps wait".into());
    }
    if let Some(activity) = view.orchestrator_activity() {
        lines.push(clean(&activity));
    }
    lines
}

fn task_progress(tasks: &[TaskRow]) -> Vec<String> {
    tasks
        .iter()
        .filter_map(|task| match task.status.as_str() {
            "working" | "verifying" => {
                let activity = if task.activity.is_empty() {
                    "…".into()
                } else {
                    clean(&task.activity)
                };
                Some(format!(
                    "{} {}: {activity}",
                    task.id,
                    status_label(&task.status)
                ))
            }
            "waiting" => Some(format!("{} waiting", task.id)),
            _ => None,
        })
        .collect()
}

fn agent_progress(view: &View) -> Vec<String> {
    [crate::lean::PLANNER, PLAN_VERIFIER, AUDITOR]
        .into_iter()
        .filter_map(|key| {
            view.agents.get(key).filter(|agent| agent.running).map(|_| {
                let doing =
                    crate::lean::tui::taskinfo::doing(view, key).unwrap_or_else(|| "…".to_owned());
                format!("{}: {doing}", agent_name(key))
            })
        })
        .collect()
}

fn review_word(review: &Value) -> String {
    if review["error"].is_string() {
        "✗ failed".into()
    } else if review["approved"] == true {
        "✓ approved".into()
    } else {
        "⚠ issues".into()
    }
}
