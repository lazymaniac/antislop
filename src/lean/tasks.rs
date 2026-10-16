//! Plans and worker/verifier pairs. Workers solve a task; each verifier judges
//! its worker's result, knowing only the risks of that worker's domain.

use crate::{Error, Result, config::WorkKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSpec {
    pub id: String,
    pub title: String,
    pub goal: String,
    /// Directories or files that matter: the project's own, or related ones
    /// such as a service it talks to. Agents read them themselves.
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub writes: bool,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub kind: WorkKind,
    pub verifier: VerifierSpec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierSpec {
    pub focus: String,
    #[serde(default)]
    pub risks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub approved: bool,
    #[serde(default)]
    pub findings: Vec<Value>,
}

pub const MAX_ROUNDS: u32 = 3;

pub mod status {
    pub const PROPOSED: &str = "proposed";
    pub const QUEUED: &str = "queued";
    pub const WORKING: &str = "working";
    pub const VERIFYING: &str = "verifying";
    pub const APPROVED: &str = "approved";
    pub const NEEDS_USER: &str = "needs_user";
    pub const FAILED: &str = "failed";
    pub const SKIPPED: &str = "skipped";
    pub const CANCELLED: &str = "cancelled";

    pub fn finished(status: &str) -> bool {
        matches!(status, APPROVED | NEEDS_USER | FAILED | SKIPPED | CANCELLED)
    }
}

/// The last ```<tag> fenced block: its body, and the text with it removed.
fn fenced<'a>(text: &'a str, tag: &str) -> Option<(String, &'a str)> {
    let open = format!("```{tag}");
    let start = text.rfind(&open)?;
    let body_start = start + open.len();
    let rest = &text[body_start..];
    if !rest.starts_with(['\n', '\r', ' ']) {
        return None;
    }
    let end = rest.find("```")?;
    let mut remaining = text[..start].trim_end().to_owned();
    let after = rest[end + 3..].trim();
    if !after.is_empty() {
        remaining.push_str("\n\n");
        remaining.push_str(after);
    }
    Some((remaining, rest[..end].trim()))
}

/// Splits an orchestrator answer into its text and an optional plan.
pub fn parse_plan(answer: &str) -> (String, Option<Result<Vec<TaskSpec>>>) {
    let Some((text, body)) = fenced(answer, "plan") else {
        return (answer.trim().to_owned(), None);
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Plan {
        tasks: Vec<TaskSpec>,
    }
    let plan = serde_json::from_str::<Plan>(body)
        .map_err(Error::from)
        .and_then(|plan| {
            validate(&plan.tasks)?;
            Ok(plan.tasks)
        });
    (text, Some(plan))
}

fn validate(tasks: &[TaskSpec]) -> Result<()> {
    crate::require(!tasks.is_empty(), "the plan has no tasks")?;
    let mut seen = std::collections::BTreeSet::new();
    for task in tasks {
        for dependency in &task.depends_on {
            crate::require(
                seen.contains(dependency.as_str()),
                &format!(
                    "task {} depends on {dependency}, which is not an earlier task",
                    task.id
                ),
            )?;
        }
        crate::require(
            seen.insert(task.id.as_str()),
            &format!("task id {} repeats", task.id),
        )?;
    }
    Ok(())
}

/// A verifier's verdict. Without a readable verdict block the whole answer
/// counts as one finding, so the worker sees what the verifier said.
/// An approval that still lists high or medium findings contradicts itself
/// and counts as a rejection.
pub fn parse_verdict(answer: &str) -> Verdict {
    let mut verdict = fenced(answer, "verdict")
        .and_then(|(_, body)| serde_json::from_str::<Verdict>(body).ok())
        .unwrap_or_else(|| Verdict {
            approved: false,
            findings: vec![Value::String(answer.trim().to_owned())],
        });
    let blocking = verdict.findings.iter().any(|finding| {
        matches!(
            finding["severity"]
                .as_str()
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("high" | "medium" | "critical")
        )
    });
    verdict.approved &= !blocking;
    verdict
}

pub fn orchestrator_instructions() -> String {
    let kinds = [
        "context",
        "requirements",
        "options",
        "planning",
        "architecture",
        "dependencies",
        "implementation",
        "refactoring",
        "static_analysis",
        "test_design",
        "testing",
        "security",
        "performance",
        "integration",
        "research",
        "writing",
        "analysis",
        "delivery",
    ];
    format!(
        "Answer simple questions yourself. When the request needs work by other \
         agents (thorough analysis of separate areas, changes, reviews), explain \
         briefly and end your answer with a plan block:\n\
         ```plan\n\
         {{\"tasks\":[{{\"id\":\"t1\",\"title\":\"short title\",\"goal\":\"what to achieve and report\",\
         \"scope\":[\"paths relative to the project, or absolute paths of related projects or services\"],\
         \"writes\":false,\"depends_on\":[],\"kind\":\"analysis\",\
         \"verifier\":{{\"focus\":\"what the verifier must check\",\"risks\":[\"risks specific to this task's domain\"]}}}}]}}\n\
         ```\n\
         Each task gets a worker that reads the files itself (give scope, not \
         content) and a verifier that checks the worker's result. Give each \
         verifier only the risks relevant to its own task. `kind` is one of: {}. \
         Set `writes` only for tasks that must change files. The user approves \
         the plan before it runs; afterwards you receive the results and \
         answer the user.",
        kinds.join(", ")
    )
}

pub fn worker_prompt(project: &str, task: &TaskSpec) -> String {
    format!(
        "You are a worker on a team of agents. The project is your current \
         working directory: {project}. Nothing is pasted for you: read the files \
         you need yourself with your tools.\n\n\
         Task: {}\nGoal: {}\nScope: {}\n\n{}\
         End with a complete report of your result; an independent verifier \
         will check it against the files.",
        task.title,
        task.goal,
        scope_text(&task.scope),
        if task.writes {
            "Make the changes in this working directory (a git worktree of the \
             project). Do not commit; the runtime commits your changes.\n\n"
        } else {
            "Work read-only; do not modify anything.\n\n"
        }
    )
}

pub fn worker_followup(findings: &[Value]) -> String {
    format!(
        "Your verifier found problems:\n{}\n\nAddress them and give your complete updated report.",
        findings_text(findings)
    )
}

/// How a verifier finds a writer's changes: the commit range, not the diff.
pub fn changes_text(base: &str, result: &str, stat: &str) -> String {
    format!(
        "The worker's changes are committed in your working directory. Inspect \
         them with `git diff {base} {result}`.\nSummary:\n{stat}"
    )
}

pub fn verifier_prompt(
    project: &str,
    task: &TaskSpec,
    report: &str,
    changes: Option<&str>,
) -> String {
    let risks = if task.verifier.risks.is_empty() {
        "none named; use your judgement for this domain".to_owned()
    } else {
        task.verifier
            .risks
            .iter()
            .map(|risk| format!("\n- {risk}"))
            .collect()
    };
    format!(
        "You verify another agent's work. The project is your current working \
         directory: {project}. Check the worker's claims against the actual \
         files yourself with your tools. Work read-only.\n\n\
         Task: {}\nGoal: {}\nScope: {}\nYour focus: {}\nRisks to check: {risks}\n\n\
         The worker's report:\n<<<\n{report}\n>>>\n\n{}{}",
        task.title,
        task.goal,
        scope_text(&task.scope),
        task.verifier.focus,
        changes
            .map(|text| format!("{text}\n\n"))
            .unwrap_or_default(),
        VERDICT_FORMAT
    )
}

pub fn verifier_followup(report: &str, changes: Option<&str>) -> String {
    format!(
        "The worker's updated report:\n<<<\n{report}\n>>>\n\n{}Check it again against the files. {VERDICT_FORMAT}",
        changes
            .map(|text| format!("{text}\n\n"))
            .unwrap_or_default()
    )
}

const VERDICT_FORMAT: &str = "End your answer with a verdict block:\n\
     ```verdict\n\
     {\"approved\": true, \"findings\": [{\"severity\": \"high|medium|low\", \"issue\": \"...\", \"where\": \"path:line\"}]}\n\
     ```\n\
     Approve only when no high or medium findings remain.";

fn scope_text(scope: &[String]) -> String {
    if scope.is_empty() {
        "the whole project".into()
    } else {
        scope.join(", ")
    }
}

pub fn findings_text(findings: &[Value]) -> String {
    findings
        .iter()
        .map(|finding| match finding {
            Value::String(text) => format!("- {text}"),
            other => format!(
                "- [{}] {} ({})",
                other["severity"].as_str().unwrap_or("?"),
                other["issue"].as_str().unwrap_or_default(),
                other["where"].as_str().unwrap_or_default()
            ),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_block_is_split_from_the_answer() {
        let answer = "Rozbiję to na dwa zadania.\n\n```plan\n{\"tasks\":[\
            {\"id\":\"t1\",\"title\":\"Storage\",\"goal\":\"Assess storage\",\"scope\":[\"src/storage.rs\"],\
             \"kind\":\"architecture\",\"verifier\":{\"focus\":\"claims\",\"risks\":[\"data loss\"]}},\
            {\"id\":\"t2\",\"title\":\"TUI\",\"goal\":\"Assess TUI\",\"depends_on\":[\"t1\"],\
             \"kind\":\"analysis\",\"verifier\":{\"focus\":\"claims\"}}]}\n```\n";
        let (text, plan) = parse_plan(answer);
        assert_eq!(text, "Rozbiję to na dwa zadania.");
        let plan = plan.unwrap().unwrap();
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[1].depends_on, ["t1"]);
        assert!(!plan[0].writes);
    }

    #[test]
    fn plans_with_unknown_dependencies_are_rejected() {
        let answer = "```plan\n{\"tasks\":[{\"id\":\"t1\",\"title\":\"a\",\"goal\":\"b\",\
            \"depends_on\":[\"t9\"],\"kind\":\"analysis\",\"verifier\":{\"focus\":\"c\"}}]}\n```";
        let (_, plan) = parse_plan(answer);
        assert!(plan.unwrap().unwrap_err().to_string().contains("t9"));
        assert!(parse_plan("no plan here").1.is_none());
        let looped = "```plan\n{\"tasks\":[{\"id\":\"t1\",\"title\":\"a\",\"goal\":\"b\",\
            \"depends_on\":[\"t1\"],\"kind\":\"analysis\",\"verifier\":{\"focus\":\"c\"}}]}\n```";
        assert!(parse_plan(looped).1.unwrap().is_err());
    }

    #[test]
    fn verdicts_parse_or_fall_back_to_the_whole_answer() {
        let verdict =
            parse_verdict("Looks fine.\n```verdict\n{\"approved\": true, \"findings\": []}\n```");
        assert!(verdict.approved);
        let contradictory = parse_verdict(
            "```verdict\n{\"approved\": true, \"findings\": [{\"severity\": \"High\", \"issue\": \"x\"}]}\n```",
        );
        assert!(!contradictory.approved);
        let fallback = parse_verdict("I could not decide.");
        assert!(!fallback.approved);
        assert_eq!(findings_text(&fallback.findings), "- I could not decide.");
    }
}
