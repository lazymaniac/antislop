//! A worker's report of what stops its task, and the rules for the task
//! that clears it. The worker says so once, in a block; the runtime stops
//! the rounds and hands the problem to the orchestrator, who may insert a
//! task before the blocked one instead of asking the same worker again.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::lean::{
    feasibility,
    tasks::{TaskSpec, fenced, status},
};
use crate::{Result, require};

/// The most tasks that may be inserted to clear one task's blockers; a
/// third attempt is the user's decision.
pub const ATTEMPTS: usize = 2;

pub const FORMAT: &str = "If the goal cannot be reached because something outside your task is \
     missing, do not repeat the shortfall in prose round after round. End your report with \
     this block, once:\n```blocker\n{\"kind\":\"missing_input|permission|unsatisfiable_requirement|too_large\",\
     \"needs\":\"what must exist or change\",\"why\":\"why you cannot supply it\",\
     \"suggest\":\"optional: a task or change that would supply it\"}\n```\nUse `permission` for \
     what your sandbox denies, `missing_input` for a result, file or decision unavailable at this \
     task's dependency boundary (including a result assigned to a downstream task), `unsatisfiable_requirement` for a demand your task cannot meet as written, `too_large` when \
     the task is more than you can finish well in one piece (say in `suggest` how you would cut \
     it into parts that can be done and checked one by one). \
     The runtime stops the task and hands the problem to the orchestrator, who fixes the plan. \
     Do not use it for work you can still do.\n\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    MissingInput,
    Permission,
    UnsatisfiableRequirement,
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Blocker {
    pub kind: Kind,
    pub needs: String,
    pub why: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggest: Option<String>,
}

impl Blocker {
    /// What a card and the orchestrator read.
    pub fn summary(&self) -> String {
        let suggest = self
            .suggest
            .as_deref()
            .map(|text| format!(" Suggested: {text}"))
            .unwrap_or_default();
        format!(
            "blocked ({:?}): needs {} — {}.{suggest}",
            self.kind, self.needs, self.why
        )
    }

    /// A refused permission is no work for another agent: it runs in the
    /// same sandbox. Only the user can change it.
    pub fn for_user(&self) -> bool {
        self.kind == Kind::Permission
    }
}

/// The worker's blocker, when its report ends in a valid block. A damaged
/// or empty block is ignored, so the report goes to the verifier as before.
pub fn parse(report: &str) -> Option<Blocker> {
    let (_, body) = fenced(report, "blocker")?;
    let blocker: Blocker = serde_json::from_str(body).ok()?;
    (!blocker.needs.trim().is_empty() && !blocker.why.trim().is_empty()).then_some(blocker)
}

/// What a worker is told about blockers, and about the one it clears.
pub fn worker_note(task: &TaskSpec) -> String {
    let part = task.part_of.as_deref().map(|whole| {
        format!(
            "You are one part of task {whole}, which was cut up because it was too large. Do \
             only your own part; the other parts are done by other tasks, and {whole} puts the \
             results together afterwards.\n\n"
        )
    });
    let clears = task.resolves.as_deref().map(|blocked| {
        format!(
            "This task exists to clear the blocker of task {blocked}. Its goal states what must be \
             true afterwards: report the evidence, in a form {blocked}'s worker can rely on.\n\n"
        )
    });
    format!(
        "{FORMAT}{}{}",
        clears.unwrap_or_default(),
        part.unwrap_or_default()
    )
}

/// The check the runtime adds to the verifier of a task that clears `blocked`.
pub fn obligation(blocked: &str) -> String {
    format!(
        "This task clears the blocker of {blocked}. Assess whether what its goal asks for is now \
         demonstrably satisfied, with evidence {blocked}'s worker can use. A plan to satisfy it \
         later, or a restated blocker, is an issue"
    )
}

/// Whether `spec` may be inserted before `blocked`. `plan` is every task of
/// the plan with its status.
pub fn check_unblocker(
    spec: &TaskSpec,
    blocked: &TaskSpec,
    plan: &[(TaskSpec, String)],
) -> Result<()> {
    crate::lean::tasks::validate_task_id(&spec.id)?;
    require(
        spec.part_of.is_none(),
        "an unblocker cannot also be a split part",
    )?;
    require(
        spec.resolves.as_deref() == Some(blocked.id.as_str()),
        "`resolves` must name the blocked task",
    )?;
    require(
        blocked.resolves.is_none(),
        "this task itself clears a blocker; a second level is left to the user",
    )?;
    require(
        !spec.writes,
        "a task that clears a blocker works read-only; one that must write is left to the user",
    )?;
    require(
        plan.iter().all(|(other, _)| other.id != spec.id),
        &format!("task id {} repeats", spec.id),
    )?;
    let attempts: Vec<&str> = plan
        .iter()
        .filter(|(other, _)| other.resolves == spec.resolves)
        .map(|(_, state)| state.as_str())
        .collect();
    require(
        attempts.iter().all(|state| status::finished(state)),
        "another task is already clearing this blocker",
    )?;
    require(
        attempts.len() < ATTEMPTS,
        "the blocker was already tried twice; the user decides",
    )?;
    dependencies(spec, blocked, plan)?;
    scope(spec, blocked)?;
    let found = feasibility::check(std::slice::from_ref(spec));
    let blocking = found
        .iter()
        .find(|finding| finding.severity == feasibility::Severity::Block);
    require(
        blocking.is_none(),
        &blocking.map(|f| f.detail.clone()).unwrap_or_default(),
    )
}

/// Dependencies must exist and must not wait on the blocked task.
fn dependencies(spec: &TaskSpec, blocked: &TaskSpec, plan: &[(TaskSpec, String)]) -> Result<()> {
    let mut later = BTreeSet::from([blocked.id.as_str()]);
    while let Some((other, _)) = plan.iter().find(|(other, _)| {
        !later.contains(other.id.as_str())
            && other.depends_on.iter().any(|d| later.contains(d.as_str()))
    }) {
        later.insert(other.id.as_str());
    }
    for dependency in &spec.depends_on {
        require(
            plan.iter().any(|(other, _)| other.id == *dependency),
            &format!("dependency {dependency} is not a task of this plan"),
        )?;
        require(
            !later.contains(dependency.as_str()),
            &format!("dependency {dependency} would wait for the task it unblocks"),
        )?;
    }
    Ok(())
}

/// The requirements a clearing task serves are the blocked task's own.
fn scope(spec: &TaskSpec, blocked: &TaskSpec) -> Result<()> {
    for path in &spec.scope {
        require(
            crate::lean::split::contained(path, &blocked.scope),
            &format!(
                "unblocker scope {path} is outside the scope of {}",
                blocked.id
            ),
        )?;
    }
    for (what, mine, theirs) in [
        (
            "requirement",
            &spec.requirement_ids,
            &blocked.requirement_ids,
        ),
        ("criterion", &spec.criterion_ids, &blocked.criterion_ids),
    ] {
        let outside = mine.iter().find(|id| !theirs.contains(id));
        require(
            outside.is_none(),
            &format!(
                "{what} {} is not one of {}'s; clearing a blocker may not widen the scope",
                outside.map(String::as_str).unwrap_or_default(),
                blocked.id
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkKind;
    use crate::lean::tasks::VerifierSpec;

    fn task(id: &str, depends_on: &[&str]) -> TaskSpec {
        TaskSpec {
            id: id.into(),
            requirement_ids: vec!["req".into()],
            criterion_ids: vec!["crit".into()],
            title: id.into(),
            goal: "Produce the inventory".into(),
            summary: None,
            section: None,
            scope: vec!["src".into()],
            writes: false,
            depends_on: depends_on.iter().map(|d| (*d).to_owned()).collect(),
            kind: WorkKind::Analysis,
            verifier: VerifierSpec {
                focus: "check".into(),
                summary: None,
                risks: Vec::new(),
            },
            resolves: None,
            part_of: None,
        }
    }

    fn clearing(id: &str, depends_on: &[&str]) -> TaskSpec {
        TaskSpec {
            resolves: Some("t2".into()),
            ..task(id, depends_on)
        }
    }

    fn plan(extra: &[(TaskSpec, &str)]) -> Vec<(TaskSpec, String)> {
        let mut plan = vec![
            (task("t1", &[]), "approved".into()),
            (task("t2", &["t1"]), "needs_user".into()),
            (task("t3", &["t2"]), "queued".into()),
        ];
        plan.extend(extra.iter().map(|(t, s)| (t.clone(), (*s).to_owned())));
        plan
    }

    fn rejects(spec: &TaskSpec, plan: &[(TaskSpec, String)], text: &str) {
        let error = check_unblocker(spec, &plan[1].0, plan)
            .unwrap_err()
            .to_string();
        assert!(error.contains(text), "{error:?} lacks {text:?}");
    }

    #[test]
    fn a_report_ends_in_a_block_or_has_none() {
        let good = "Report.\n```blocker\n{\"kind\":\"missing_input\",\"needs\":\"an inventory\",\
                    \"why\":\"read-only\"}\n```";
        assert_eq!(parse(good).unwrap().kind, Kind::MissingInput);
        for bad in [
            "no block",
            "```blocker\nnot json\n```",
            "```blocker\n{\"kind\":\"missing_input\",\"needs\":\" \",\"why\":\"x\"}\n```",
        ] {
            assert!(parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn only_a_refused_permission_is_the_users_alone() {
        let block = |kind| Blocker {
            kind,
            needs: "n".into(),
            why: "w".into(),
            suggest: None,
        };
        assert!(block(Kind::Permission).for_user());
        assert!(!block(Kind::MissingInput).for_user());
    }

    #[test]
    fn a_read_only_task_before_the_blocked_one_is_accepted() {
        check_unblocker(&clearing("fix", &["t1"]), &plan(&[])[1].0, &plan(&[])).unwrap();
    }

    #[test]
    fn the_rules_that_keep_unblocking_bounded() {
        let base = plan(&[]);
        let mut writer = clearing("fix", &["t1"]);
        writer.writes = true;
        rejects(&writer, &base, "read-only");
        rejects(
            &clearing("fix", &["t3"]),
            &base,
            "would wait for the task it unblocks",
        );
        rejects(
            &clearing("fix", &["nope"]),
            &base,
            "not a task of this plan",
        );
        rejects(&clearing("t1", &[]), &base, "repeats");
        let mut wider = clearing("fix", &[]);
        wider.requirement_ids.push("other".into());
        rejects(&wider, &base, "may not widen the scope");
        let mut elsewhere = clearing("fix", &[]);
        elsewhere.resolves = Some("t1".into());
        rejects(&elsewhere, &base, "must name the blocked task");
    }

    #[test]
    fn one_attempt_at_a_time_and_two_in_all() {
        let running = plan(&[(clearing("a", &[]), "working")]);
        rejects(&clearing("b", &[]), &running, "already clearing");
        let once = plan(&[(clearing("a", &[]), "approved")]);
        check_unblocker(&clearing("b", &[]), &once[1].0, &once).unwrap();
        let twice = plan(&[
            (clearing("a", &[]), "approved"),
            (clearing("b", &[]), "failed"),
        ]);
        rejects(&clearing("c", &[]), &twice, "tried twice");
    }

    #[test]
    fn a_clearing_task_cannot_itself_be_cleared() {
        let mut base = plan(&[]);
        base[1].0.resolves = Some("t1".into());
        rejects(&clearing("fix", &[]), &base, "second level");
    }

    #[test]
    fn the_verifier_gets_an_obligation_for_the_blocker() {
        let ids: Vec<String> = crate::lean::tasks::obligations(&clearing("fix", &[]))
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids[0], "B1");
        assert!(
            crate::lean::tasks::obligations(&task("t", &[]))
                .iter()
                .all(|(id, _)| id != "B1")
        );
    }
}
