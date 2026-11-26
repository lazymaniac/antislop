use super::*;
use crate::lean::agent_tools::turn_inputs::{ReadFor, TurnInputs};
use crate::lean::prompts;

/// How a plan, audit or code reviewer ends its turn.
pub const REVIEW_FORMAT: &str = prompts::REVIEW_FORMAT;

pub(crate) const READ_ONLY_REVIEW: &str = prompts::READ_ONLY_REVIEW;

/// What every agent of a task keeps to (design §10.3, §13).
pub(super) const PROJECT_RULES: &str = prompts::PROJECT_RULES;

/// The context a plan, audit or code reviewer may inspect.
pub(super) const OUTSIDE: &str = prompts::OUTSIDE;

/// How a reviewer reports while it works, so the user can follow each check
/// and its evidence as it is settled, not only in the final verdict.
pub(super) const REVIEW_RULES: &str = prompts::REVIEW_RULES;

pub(super) const COVERAGE_FORMAT: &str = prompts::COVERAGE_FORMAT;

/// What identifies a set of findings across rounds: severity, place and the
/// first sentence of each issue, so rewording of the notes does not hide a
/// repeat.
pub fn findings_key(findings: &[Value]) -> String {
    findings
        .iter()
        .map(|finding| match finding {
            Value::String(text) => first_sentence(text),
            other => format!(
                "{}|{}|{}",
                other["severity"].as_str().unwrap_or("?"),
                other["where"].as_str().unwrap_or_default(),
                first_sentence(other["issue"].as_str().unwrap_or_default())
            ),
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join("\n")
}

fn first_sentence(text: &str) -> String {
    let end = text
        .find([';', ':'])
        .or_else(|| text.find(". "))
        .unwrap_or(text.len());
    text[..end].trim().to_lowercase()
}

pub fn findings_text(findings: &[Value]) -> String {
    findings
        .iter()
        .map(|finding| match finding {
            Value::String(text) => {
                format!(crate::lean::prompts::template!(PLAIN_FINDING), text = text)
            }
            other => {
                let severity = other["severity"].as_str().filter(|s| !s.is_empty());
                let location = other["where"].as_str().filter(|s| !s.is_empty());
                format!(
                    crate::lean::prompts::template!(FINDING),
                    other["id"]
                        .as_str()
                        .map(|id| format!(crate::lean::prompts::template!(FINDING_ID), id = id))
                        .unwrap_or_default(),
                    severity
                        .map(|s| format!(crate::lean::prompts::template!(FINDING_SEVERITY), s = s))
                        .unwrap_or_default(),
                    other["issue"].as_str().unwrap_or_default(),
                    location
                        .map(|s| format!(crate::lean::prompts::template!(FINDING_LOCATION), s = s))
                        .unwrap_or_default(),
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What a fresh session of a later round adds to its base prompt, which
/// already carries the task and its dependency data: the requirements, what
/// earlier rounds changed and what the reviewer found. The previous report
/// is repeated only when the files do not say what was done: a turn that never
/// finished, a task that writes nothing, or nothing to fix yet. A writer with
/// findings to fix gets the changed files and the diff range instead.
pub(crate) fn round_handoff_prompt(
    handoff: &crate::lean::rounds::RoundHandoff,
    reviewer: bool,
    inputs: &mut TurnInputs,
) -> String {
    let (protection, report, findings) = handoff_sections(handoff, reviewer, inputs);
    format!(
        prompts::template!(ROUND_HANDOFF_PROMPT),
        handoff.worktree.display(),
        handoff.round,
        handoff.change_kind,
        handoff.interrupted,
        report,
        findings,
        handoff.base,
        handoff.result,
        handoff.files.join(", "),
        handoff_tail(handoff, reviewer, &protection),
    )
}

fn handoff_sections(
    handoff: &crate::lean::rounds::RoundHandoff,
    reviewer: bool,
    inputs: &mut TurnInputs,
) -> (String, String, String) {
    let protection = handoff_protection(handoff);
    let report = handoff.report.as_ref().map_or_else(
        || prompts::NO_WORKER_REPORT.into(),
        |(round, _)| format!(prompts::template!(PREVIOUS_WORKER_REPORT), round = round),
    );
    let findings = handoff
        .verdict
        .as_ref()
        .and_then(|v| v["findings"].as_array())
        .map_or_else(
            || prompts::NONE.into(),
            |items| {
                if reviewer {
                    prompts::PREVIOUS_REVIEW_FINDINGS.into()
                } else {
                    inputs.block(
                        prompts::UNRESOLVED_FINDINGS_LABEL,
                        &findings_text(items),
                        ReadFor::Reference,
                    )
                }
            },
        );
    (protection, report, findings)
}

fn handoff_protection(handoff: &crate::lean::rounds::RoundHandoff) -> String {
    if handoff.protected_paths.is_empty() {
        return String::new();
    }
    format!(
        prompts::template!(HANDOFF_PROTECTION),
        handoff.primary.display(),
        handoff.protected_paths.join(", ")
    )
}

fn handoff_tail(
    handoff: &crate::lean::rounds::RoundHandoff,
    _reviewer: bool,
    protection: &str,
) -> String {
    format!(
        prompts::template!(HANDOFF_TAIL),
        crate::lean::prompt_context::markdown(&handoff.interruptions),
        crate::lean::prompt_context::untrusted(
            prompts::OPEN_COMMENTS_LABEL,
            &crate::lean::prompt_context::markdown(&handoff.comments)
        ),
        protection
    )
}
