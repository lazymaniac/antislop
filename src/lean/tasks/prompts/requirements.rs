use super::*;

/// How a plan, audit or code reviewer ends its turn.
pub const REVIEW_FORMAT: &str = "End with review: your report and the findings, each with \
     severity, issue and where. The runtime approves only when no finding is critical, high \
     or medium; give findings that do not block as low. Keep the same `where` while a \
     problem stays open, so it is recognised across rounds.";

pub(crate) const READ_ONLY_REVIEW: &str = "Review only: do not modify, create, delete or commit project files, or repair the reviewed result. Read files, inspect diffs and run checks that preserve the reviewed source. Report repairs for the worker; preserve user changes.";

/// What every agent of a task keeps to (design §10.3, §13).
pub(super) const PROJECT_RULES: &str = "Follow applicable repository instructions and the user's current request. Runtime state and evidence belong in the durable runtime directory. Historical allocation hashes, branches and paths are context, not prerequisites. Preserve and inspect an explicit user-selected review commit, range, branch, baseline or source; never silently substitute another target. Inspect the actual checkout and report concrete observations. Ask about inputs only when actually unavailable or unusable.";

/// The context a plan, audit or code reviewer may inspect.
pub(super) const OUTSIDE: &str = "Inspect the project and any supporting files needed to verify the task. Preserve user files and the independence of the review.";

/// How a reviewer reports while it works, so the user can follow each check
/// and its evidence as it is settled, not only in the final verdict.
pub(super) const REVIEW_RULES: &str = "Review independently. On later rounds verify earlier fixes and reassess the whole original contract. New in-scope defects keep their actual severity even if they existed before; do not add unrelated deliverables. For each acceptance criterion, separate its conditions, qualifiers and failure cases before deciding its overall status. Trace the real entry point through component boundaries to the final observable effect and state, including when success is acknowledged and what remains after failure, cancellation or retry. A failure reported later does not prove an earlier successful action was rolled back. Verify each qualifier with specific source or execution evidence; do not mark a compound criterion ok from evidence for only one part. Inspect files, diffs and retained evidence, and run checks when useful to verify a claim. Do not make implementation repairs; report actionable findings for the worker. Preserve the delivered result while reviewing it. Missing evidence must be described precisely; do not claim a check passed without observing its evidence. \
     Assess the actual files and whether the report accurately describes its observations. \
     Revision, snapshot, path or changed-file mismatches alone are never blocking findings, \
     even when input files differ. Identify a concrete defect in the work or its evidence. \
     When the work breaks a measurable bound (a line, length or size limit, a naming rule, a \
     forbidden pattern), measure everything the task owns with read-only search and list \
     every violation, each with its location and size, in one finding; never give examples, \
     because the worker fixes what you name and each violation left out costs another round. \
     A criterion that something did not happen (no write outside the project, no \
     configuration change, no data sent away) is ok when the files and recorded evidence \
     show no violation; never require a record proving the absence unless the requirement \
     itself names that record. \
     Judge the content of the work, not who produced it: \
     never require independent proof of which agent, turn, session or directory made the \
     report, a runtime allocation record, or a receipt of delivery; none exists to give.";

pub(super) const COVERAGE_FORMAT: &str = "An issue is a blocking failure of an acceptance obligation, not an optional suggestion or a harmless report detail. Low findings must not turn an otherwise satisfied check into issue. Keep such a check ok and explain the non-blocking observation. Choose severity from actual impact: critical for catastrophic loss or compromise, high for a broken required behavior or serious security/data loss, medium for another material contract violation, low for non-blocking improvements. Never lower severity because a defect was found in a later round. Use blocked for a check you could not settle (missing \
     data is not ok) and n/a only with a reason tied to this task. When the task is \
     report-only analysis, judge the report, not the code: ok means the report \
     covers the obligation correctly (including defects it rightly reports in \
     the code); issue means the report is wrong, incomplete or unsupported. A command the \
     environment denied (`Operation not permitted`, `os error 1`, `PermissionDenied`) is \
     blocked, never issue: the worker cannot fix the environment.";

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
            Value::String(text) => format!("- {text}"),
            other => {
                let severity = other["severity"].as_str().filter(|s| !s.is_empty());
                let location = other["where"].as_str().filter(|s| !s.is_empty());
                format!(
                    "- {}{}{}{}",
                    other["id"]
                        .as_str()
                        .map(|id| format!("{id} "))
                        .unwrap_or_default(),
                    severity.map(|s| format!("[{s}] ")).unwrap_or_default(),
                    other["issue"].as_str().unwrap_or_default(),
                    location.map(|s| format!(" ({s})")).unwrap_or_default(),
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
) -> String {
    let (protection, report, findings) = handoff_sections(handoff, reviewer);
    format!(
        "\n\nRound handoff\nWorktree: {}\nRound: {}\nDelivery: {:?}\nInterrupted before a report: {}\n{}\nPrevious reviewer findings: {}\nBase commit: {:?}\nLatest result commit: {:?}\nFiles changed in earlier rounds: {}\nInspect the actual local files. For a task delivering files, inspect its base/result diff; a report-only task has no delivery commits. Earlier-round results are unreviewed against changed inputs. Preserve user changes and follow repository instructions. A reviewer must assess every obligation and the complete task, including areas outside earlier findings.\n{}",
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
) -> (String, String, String) {
    let protection = handoff_protection(handoff);
    let report = handoff.report.as_ref().map_or_else(
        || "No completed worker report is available.".into(),
        |(round, _)| format!("Previous worker report: read_dependency_result dependency=self, round={round}; use report_history.result_ref for historical inputs. Full verdict: read_dependency_result dependency=self, view=review with the relevant round."));
    let findings = handoff.verdict.as_ref().and_then(|v| v["findings"].as_array()).map_or_else(
        || "none".into(), |items| if reviewer { "Use the earlier findings listed for this turn; full verdict: read_dependency_result dependency=self, view=review with the relevant round.".into() } else { crate::lean::prompt_context::untrusted("Unresolved findings", &findings_text(items)) });
    (protection, report, findings)
}

fn handoff_protection(handoff: &crate::lean::rounds::RoundHandoff) -> String {
    if handoff.protected_paths.is_empty() {
        return String::new();
    }
    format!(
        "\nChanges already present in the primary working tree at {}:\n{}\nPreserve these changes when integrating. Work only in the allocated worktree; never edit the primary tree during repair. These paths do not prohibit task changes inside your own worktree.\n",
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
        "\nPrevious round interruptions (fresh session continues from files and the report):\n{}\nCurrent open comments:\n{}{}",
        crate::lean::prompt_context::markdown(&handoff.interruptions),
        crate::lean::prompt_context::untrusted(
            "Open comments",
            &crate::lean::prompt_context::markdown(&handoff.comments)
        ),
        protection
    )
}
