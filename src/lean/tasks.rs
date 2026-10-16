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
    /// A plan the plan verifier sent back; the orchestrator proposes another.
    pub const REVISED: &str = "revised";
    /// The agent's CLI needs the user: a login, or a usage limit to reset.
    pub const WAITING: &str = "waiting";

    pub fn finished(status: &str) -> bool {
        matches!(
            status,
            APPROVED | NEEDS_USER | FAILED | SKIPPED | CANCELLED | REVISED | WAITING
        )
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
         Set `writes` only for tasks that must change files. A task that needs \
         another's result depends on it; tasks that change the same files or \
         contract are ordered. An unknown that matters is established by an \
         earlier task, never assumed.\n\n\
         Shape the team to the request; small requests get few, short tasks:\n\
         - change to existing code: context, then implementation per module \
         (with its tests), then integration that checks the whole change; add \
         architecture, dependencies, security or performance tasks when the \
         change carries those risks;\n\
         - refactoring: context and characterization tests first, then the \
         refactoring, then integration;\n\
         - security fix: threat and reproduction, fix, adversarial tests;\n\
         - new project: requirements, architecture, a vertical slice, then \
         increments;\n\
         - research: independent areas researched separately, then a synthesis;\n\
         - document: outline, research if needed, writing;\n\
         - analysis: method, the analysis, an independent recalculation.\n\n\
         An independent plan verifier checks your plan before the user sees it \
         and may send it back with findings. After the plan runs you receive \
         each task's result and an independent audit of whether the request was \
         fully delivered; answer the user, or propose a plan for what is \
         missing.",
        kinds.join(", ")
    )
}

/// What a verifier checks besides its own focus, by the kind of work: only
/// the concerns of its worker's domain.
pub fn checklist(kind: WorkKind) -> &'static str {
    use WorkKind::*;
    match kind {
        Orchestration | Planning => {
            "dependencies complete and correctly ordered; no intermediate state \
             or ordering assumed that the system does not guarantee; unknowns \
             established, not assumed; every requirement covered by a task"
        }
        Context => {
            "every claim backed by files actually read; dependencies, contracts \
             and conventions that matter are covered; open unknowns named"
        }
        Requirements => {
            "every requirement of the source found, including ones in the middle; \
             each qualifier of a requirement kept; contradictions named"
        }
        Options => {
            "real alternatives considered under the same conditions; a variant \
             that misses an obligation is excluded before ranking; the choice \
             follows from the evidence"
        }
        Architecture => {
            "fits the project's existing structure and idioms; the simplest \
             adequate design; no pattern or layer without a present need; no \
             policy duplicated across modules"
        }
        Dependencies => {
            "an existing maintained library considered before custom code; \
             licenses of the actual dependency graph (vendored files included) \
             compatible; chosen license option and its obligations stated"
        }
        Implementation => {
            "does exactly what the goal asks, nothing more; no duplicated module \
             or needless layer; follows project conventions; edge cases and \
             combinations of inputs handled; tests exercise the change with \
             meaningful assertions; run the project's checks when you can"
        }
        Refactoring => {
            "behaviour preserved, with evidence; structure actually improved; no \
             speculative abstraction; when no change is needed, the report says \
             why instead of a cosmetic patch"
        }
        StaticAnalysis => {
            "analyzers actually run with the project's configuration; warnings \
             count even when the exit code is 0; no suppression, rule weakening \
             or baseline change to make findings disappear"
        }
        TestDesign => {
            "scenarios follow from the requirements and the likely failures; \
             combinations of parameters that interact are covered, not only \
             each one separately"
        }
        Testing => {
            "tests have assertions that fail on the bug they target; they run \
             the real code path, not a mock of it; green runs actually observed"
        }
        Security => {
            "threats relevant to the change modelled; findings reproduced, false \
             alarms separated; the fix closes the issue without new exposure"
        }
        Performance => {
            "benchmark representative and compared to a baseline under the same \
             conditions; semantics unchanged"
        }
        Integration => {
            "the combined change builds and passes checks as a whole; contracts \
             between changed modules agree; no regression"
        }
        Research => {
            "every source exists and supports the claim attributed to it; facts \
             separated from conclusions; sources current enough"
        }
        Writing => {
            "factually correct against the sources and the project; consistent; \
             matches the brief"
        }
        Analysis => {
            "data and assumptions stated; calculations recomputed independently; \
             units and denominators correct; reproducible"
        }
        Delivery => {
            "the report matches what was actually done; limitations and open \
             risks stated; instructions work"
        }
    }
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

/// For a worker resumed after its task stopped: its session still holds the
/// task, so only what happened is said.
pub fn worker_resume(findings: &[Value]) -> String {
    if findings.is_empty() {
        "Your work on this task was interrupted. Check the current state of the \
         files, finish the task and give your complete report."
            .into()
    } else {
        format!(
            "The task was resumed. Your verifier's open findings:\n{}\n\nCheck the \
             current state of the files, address them and give your complete \
             updated report.",
            findings_text(findings)
        )
    }
}

/// Whether a CLI's error means it waits for the user (a login, a usage or
/// rate limit) rather than that the task went wrong.
pub fn needs_user_action(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    [
        "login",
        "log in",
        "logged in",
        "authenticat",
        "unauthorized",
        "api key",
        "credentials",
        "rate limit",
        "rate_limit",
        "usage limit",
        "quota",
        "too many requests",
        "credit balance",
    ]
    .iter()
    .any(|marker| error.contains(marker))
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
         Task: {}\nGoal: {}\nScope: {}\nYour focus: {}\nAlso check: {}\n\
         Risks to check: {risks}\n\n\
         The worker's report:\n<<<\n{report}\n>>>\n\n{}{}",
        task.title,
        task.goal,
        scope_text(&task.scope),
        task.verifier.focus,
        checklist(task.kind),
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

/// Longest user request handed to a reviewer in one prompt.
const REQUEST_LIMIT: usize = 60_000;

fn bounded(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((end, _)) => format!("{}\n[truncated]", &text[..end]),
        None => text.to_owned(),
    }
}

pub fn plan_review_prompt(project: &str, request: &str, plan: &[TaskSpec]) -> String {
    format!(
        "You review a plan before any of it runs or the user sees it. The \
         project is your current working directory: {project}. Read the files \
         you need yourself; work read-only.\n\n\
         The user's request:\n<<<\n{}\n>>>\n\n\
         The orchestrator's plan:\n{}\n\n{}{}",
        bounded(request, REQUEST_LIMIT),
        plan_text(plan),
        PLAN_CHECKS,
        VERDICT_FORMAT
    )
}

pub fn plan_review_followup(request: &str, plan: &[TaskSpec]) -> String {
    format!(
        "{}The orchestrator's new plan:\n{}\n\nCheck it again. {}{}",
        if request.is_empty() {
            String::new()
        } else {
            format!(
                "The user wrote since:\n<<<\n{}\n>>>\n\n",
                bounded(request, REQUEST_LIMIT)
            )
        },
        plan_text(plan),
        PLAN_CHECKS,
        VERDICT_FORMAT
    )
}

const PLAN_CHECKS: &str = "Check that: every part of the request is covered \
     by some task and nothing unrequested is planned; dependencies are complete \
     and ordered (a task needing another's result depends on it; tasks changing \
     the same files or contract are not unordered); no ordering or intermediate \
     state is assumed that the system does not guarantee; no unknown that \
     matters is assumed instead of being established first; each goal is \
     checkable; scope names the places that matter, including related projects \
     or services; tasks that change files are marked `writes`; each verifier's \
     focus and risks fit its own task. ";

fn plan_text(plan: &[TaskSpec]) -> String {
    serde_json::to_string_pretty(plan).unwrap_or_default()
}

pub fn plan_rejected(findings: &[Value]) -> String {
    format!(
        "The plan verifier sent the plan back before the user saw it:\n{}\n\n\
         Propose a corrected plan, or explain to the user why a finding does \
         not apply.",
        findings_text(findings)
    )
}

pub fn audit_prompt(project: &str, request: &str, results: &str) -> String {
    format!(
        "You audit whether the user's request was fully delivered. The project \
         is your current working directory: {project}. Read the files yourself \
         to check what the results claim; work read-only.\n\n\
         The user's request:\n<<<\n{}\n>>>\n\n\
         Results of the plan that ran:\n{results}\n\n{}{}",
        bounded(request, REQUEST_LIMIT),
        AUDIT_CHECKS,
        VERDICT_FORMAT
    )
}

pub fn audit_followup(request: &str, results: &str) -> String {
    format!(
        "{}Results of the next plan:\n{results}\n\n{}{}",
        if request.is_empty() {
            String::new()
        } else {
            format!(
                "The user wrote since:\n<<<\n{}\n>>>\n\n",
                bounded(request, REQUEST_LIMIT)
            )
        },
        AUDIT_CHECKS,
        VERDICT_FORMAT
    )
}

const AUDIT_CHECKS: &str = "List every obligation in the request, each \
     qualifier included, and decide for each whether the results and the files \
     show it done. A missing, partial or unverified obligation is a high \
     finding. ";

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
    fn login_and_limit_errors_wait_for_the_user() {
        assert!(needs_user_action("Error: Not logged in. Run `agy login`."));
        assert!(needs_user_action("429 Too Many Requests"));
        assert!(needs_user_action("You've hit your usage limit"));
        assert!(!needs_user_action("exit status 1: panicked at src/main.rs"));
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
