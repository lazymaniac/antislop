use super::*;
use std::borrow::Cow;
use unicode_segmentation::UnicodeSegmentation;

pub(in crate::lean::tui) fn task_goal<'a>(
    kind: &str,
    title: &str,
    goal: &'a str,
    summary: Option<&str>,
) -> Cow<'a, str> {
    if kind == "options" {
        let owner = goal
            .lines()
            .next_back()
            .and_then(|line| serde_json::from_str::<Value>(line).ok());
        let subject = owner
            .as_ref()
            .and_then(|value| value.get("owner_contract"))
            .and_then(|contract| {
                ["summary", "goal", "title"]
                    .into_iter()
                    .filter_map(|field| contract[field].as_str())
                    .find(|text| !text.trim().is_empty())
            });
        if subject.is_some() || goal.starts_with("Choose the best available approach") {
            let subject =
                subject.unwrap_or_else(|| title.strip_prefix("Options for ").unwrap_or(title));
            return Cow::Owned(format!(
                "Compare feasible approaches and recommend one, explaining the main tradeoffs. The choice concerns: {}.",
                first_sentence(subject)
            ));
        }
    }
    let focus = goal
        .strip_prefix("Required quality gate for the whole change: ")
        .or_else(|| goal.strip_prefix("Required audit of the whole change: "));
    if let Some(focus) = focus {
        let purpose = summary
            .filter(|text| !text.trim().is_empty())
            .unwrap_or(focus);
        return Cow::Owned(format!(
            "{}. It covers the changes delivered by the tasks this agent depends on.",
            first_sentence(purpose)
        ));
    }
    if let Some(summary) = summary.filter(|text| !text.trim().is_empty()) {
        let sentences = goal
            .unicode_sentences()
            .map(first_sentence)
            .filter(|sentence| {
                !sentence.is_empty()
                    && !sentence.starts_with("Consume ")
                    && !sentence.starts_with("Inspect ")
            })
            .take(2)
            .collect::<Vec<_>>();
        let text = if sentences.is_empty() {
            first_sentence(summary)
        } else {
            sentences.join(". ")
        };
        return Cow::Owned(format!("{text}."));
    }
    Cow::Borrowed(goal)
}

fn first_sentence(text: &str) -> String {
    let text = crate::lean::tui::view::clean_line(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut graphemes = text.graphemes(true).peekable();
    let mut sentence = String::new();
    while let Some(grapheme) = graphemes.next() {
        if matches!(grapheme, "." | "!" | "?")
            && graphemes
                .peek()
                .is_none_or(|next| next.chars().all(char::is_whitespace))
        {
            break;
        }
        sentence.push_str(grapheme);
    }
    crate::lean::tui::view::truncate(&sentence, 180)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lean::tasks::{TaskContract, with_options, with_required_agents};
    use crate::lean::tui::{details::Details, theme::Theme};

    #[test]
    fn generated_agent_goals_show_their_purpose_without_runtime_instructions_or_json() {
        let writer: TaskContract = serde_json::from_value(serde_json::json!({
            "id":"session-actions", "title":"Session actions", "summary":"Provide actions for completed sessions 👩‍💻e\u{301}.",
            "goal":"Inspect the completion contract. Add actions in src/session.rs for completed sessions 👩‍💻e\u{301}. Run checks afterwards.",
            "kind":"implementation", "change_kind":"code",
            "acceptance_criteria":[{"id":"C1","text":"Session actions preserve completed work"}]
        })).unwrap();
        let tasks = with_options(with_required_agents(vec![writer.clone()]));
        for task in &tasks {
            let row = TaskRow {
                id: task.id.clone(),
                title: task.title.clone(),
                kind: crate::lean::tasks::work_kind_name(task.kind).into(),
                goal: task.goal.clone(),
                summary: task.summary.clone(),
                ..Default::default()
            };
            let details = Details::new(format!("worker:{}", task.id));
            let mut view = View::default();
            view.tasks.push(row);
            let sections = details.sections(&view, &Theme::new(false, false), 1000);
            let shown = sections
                .iter()
                .find(|section| section.id == "goal")
                .unwrap()
                .body
                .iter()
                .flat_map(|line| &line.spans)
                .map(|span| span.content.as_ref())
                .collect::<String>();
            let checks = crate::lean::tui::details::check_rows(&serde_json::json!({"spec":task}));
            assert_eq!(
                checks.iter().find(|check| check.id == "goal").unwrap().text,
                format!("The goal is met: {shown}")
            );
            if task.options_for.is_some() {
                assert!(
                    shown.contains("Provide actions for completed sessions 👩‍💻e\u{301}"),
                    "{shown}"
                );
                assert!(!shown.contains("Run checks afterwards"));
                assert!(!shown.contains("owner_contract") && !shown.contains("Do not execute"));
                assert_eq!(shown.matches(". ").count(), 1, "{shown}");
                assert!(
                    task.goal.contains("owner_contract"),
                    "the execution contract is retained"
                );
            } else if task.quality_gate {
                assert!(
                    !shown.contains("Required quality gate") && !shown.contains("Required audit")
                );
                assert!(shown.contains("tasks this agent depends on"));
                assert_eq!(shown.matches(". ").count(), 1, "{shown}");
            } else {
                assert_eq!(
                    shown,
                    "Add actions in src/session.rs for completed sessions 👩‍💻e\u{301}. Run checks afterwards."
                );
            }
        }
        let legacy = TaskRow {
            kind: "options".into(),
            title: "Options for Session actions".into(),
            goal: "Choose the best available approach\n{invalid JSON".into(),
            ..Default::default()
        };
        assert_eq!(
            task_goal(
                &legacy.kind,
                &legacy.title,
                &legacy.goal,
                legacy.summary.as_deref()
            ),
            "Compare feasible approaches and recommend one, explaining the main tradeoffs. The choice concerns: Session actions."
        );
    }
}
