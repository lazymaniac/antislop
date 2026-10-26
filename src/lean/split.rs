//! Cutting a task that is too large into parts. The task stays in the
//! plan as the one that puts the parts together; the parts come before it,
//! each within its scope. The rules keep a split from widening the work,
//! from racing two writers on the same files, and from going on forever.
use std::collections::{BTreeMap, BTreeSet};

use crate::lean::{
    feasibility,
    tasks::{TaskSpec, scopes_overlap},
};
use crate::{Result, require};

pub const MAX_PARTS: usize = 8;

/// The parts ready to insert before `blocked`: owned by it, with the
/// defaults filled in. `plan` is every task of the plan, in order, with its
/// status.
pub fn prepare(
    parts: &[TaskSpec],
    blocked: &TaskSpec,
    plan: &[(TaskSpec, String)],
) -> Result<Vec<TaskSpec>> {
    require(
        (2..=MAX_PARTS).contains(&parts.len()),
        &format!("a split makes 2 to {MAX_PARTS} parts"),
    )?;
    check_parent(blocked, plan)?;
    let ready: Vec<TaskSpec> = parts.iter().map(|part| inherit(part, blocked)).collect();
    names(&ready, plan)?;
    for part in &ready {
        within(part, blocked)?;
    }
    dependencies(&ready, blocked, plan)?;
    races(&ready)?;
    for part in &ready {
        let found = feasibility::check(std::slice::from_ref(part));
        let blocking = found
            .iter()
            .find(|finding| finding.severity == feasibility::Severity::Block);
        require(
            blocking.is_none(),
            &format!(
                "part {}: {}",
                part.id,
                blocking.map(|f| f.detail.as_str()).unwrap_or_default()
            ),
        )?;
    }
    Ok(ready)
}

/// Parent-level eligibility before validating a proposed set of parts.
pub fn check_parent(blocked: &TaskSpec, plan: &[(TaskSpec, String)]) -> Result<()> {
    require(
        blocked.resolves.is_none() && blocked.part_of.is_none(),
        "this task is itself a part or clears a blocker; cutting it further is left to the user",
    )?;
    require(
        plan.iter()
            .all(|(other, _)| other.part_of.as_deref() != Some(&blocked.id)),
        "this task was already cut once; the user decides",
    )?;
    Ok(())
}

/// A part serves the requirements of the task it was cut from, works where
/// that did unless it says less, and first waits for what that waited for.
fn inherit(part: &TaskSpec, blocked: &TaskSpec) -> TaskSpec {
    let mut part = part.clone();
    part.part_of = Some(blocked.id.clone());
    if part.requirement_ids.is_empty() {
        part.requirement_ids = blocked.requirement_ids.clone();
    }
    if part.criterion_ids.is_empty() {
        part.criterion_ids = blocked.criterion_ids.clone();
    }
    if part.scope.is_empty() {
        part.scope = blocked.scope.clone();
    }
    let mut depends_on = blocked.depends_on.clone();
    for dependency in &part.depends_on {
        if !depends_on.contains(dependency) {
            depends_on.push(dependency.clone());
        }
    }
    part.depends_on = depends_on;
    part
}

fn names(parts: &[TaskSpec], plan: &[(TaskSpec, String)]) -> Result<()> {
    let mut seen: BTreeSet<&str> = plan.iter().map(|(task, _)| task.id.as_str()).collect();
    for part in parts {
        crate::lean::tasks::validate_task_id(&part.id)?;
        require(part.resolves.is_none(), "a part does not clear a blocker")?;
        require(
            seen.insert(part.id.as_str()),
            &format!("task id {} repeats", part.id),
        )?;
    }
    Ok(())
}

fn normal(path: &str) -> &str {
    path.trim().trim_start_matches("./").trim_end_matches('/')
}

pub(crate) fn contained(path: &str, allowed: &[String]) -> bool {
    let path = normal(path);
    allowed.is_empty()
        || allowed.iter().any(|root| {
            let root = normal(root);
            root.is_empty()
                || root == "."
                || path == root
                || path
                    .strip_prefix(root)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
}

/// A part stays within what the whole was given.
fn within(part: &TaskSpec, blocked: &TaskSpec) -> Result<()> {
    require(
        !part.writes || blocked.writes,
        &format!("part {} writes but {} is read-only", part.id, blocked.id),
    )?;
    if let Some(path) = part
        .scope
        .iter()
        .find(|path| !contained(path, &blocked.scope))
    {
        require(
            false,
            &format!(
                "part {} reaches {path}, outside the scope of {}",
                part.id, blocked.id
            ),
        )?;
    }
    for (what, mine, theirs) in [
        (
            "requirement",
            &part.requirement_ids,
            &blocked.requirement_ids,
        ),
        ("criterion", &part.criterion_ids, &blocked.criterion_ids),
    ] {
        if let Some(id) = mine.iter().find(|id| !theirs.contains(id)) {
            require(
                false,
                &format!(
                    "part {} serves {what} {id}, which {} does not",
                    part.id, blocked.id
                ),
            )?;
        }
    }
    Ok(())
}

/// A part waits for earlier parts or for tasks that come before the whole.
fn dependencies(parts: &[TaskSpec], blocked: &TaskSpec, plan: &[(TaskSpec, String)]) -> Result<()> {
    let at = plan
        .iter()
        .position(|(task, _)| task.id == blocked.id)
        .unwrap_or(plan.len());
    let before: BTreeSet<&str> = plan[..at]
        .iter()
        .map(|(task, _)| task.id.as_str())
        .collect();
    let mut earlier = BTreeSet::new();
    for part in parts {
        for dependency in &part.depends_on {
            require(
                before.contains(dependency.as_str()) || earlier.contains(dependency.as_str()),
                &format!(
                    "part {} waits for {dependency}, which is neither an earlier part nor a \
                     task ahead of {}",
                    part.id, blocked.id
                ),
            )?;
        }
        earlier.insert(part.id.as_str());
    }
    Ok(())
}

/// Two writers whose scopes meet must be ordered by a dependency.
fn races(parts: &[TaskSpec]) -> Result<()> {
    let mut ancestors: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for part in parts {
        let mut all: BTreeSet<&str> = BTreeSet::new();
        for dependency in &part.depends_on {
            all.insert(dependency.as_str());
            all.extend(
                ancestors
                    .get(dependency.as_str())
                    .into_iter()
                    .flatten()
                    .copied(),
            );
        }
        ancestors.insert(part.id.as_str(), all);
    }
    for (index, later) in parts.iter().enumerate().filter(|(_, part)| part.writes) {
        for earlier in parts[..index].iter().filter(|part| part.writes) {
            let ordered = ancestors[later.id.as_str()].contains(earlier.id.as_str());
            require(
                ordered || !scopes_overlap(&earlier.scope, &later.scope),
                &format!(
                    "parts {} and {} write the same files and nothing orders them: make {} \
                     depend on {}, or give them separate scopes",
                    earlier.id, later.id, later.id, earlier.id
                ),
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkKind;
    use crate::lean::tasks::VerifierSpec;

    fn task(id: &str, writes: bool, scope: &[&str], depends_on: &[&str]) -> TaskSpec {
        TaskSpec {
            id: id.into(),
            requirement_ids: vec!["req".into()],
            criterion_ids: vec!["crit".into()],
            title: id.into(),
            goal: "Change it".into(),
            summary: None,
            section: None,
            scope: scope.iter().map(|s| (*s).to_owned()).collect(),
            writes,
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

    fn plan() -> Vec<(TaskSpec, String)> {
        vec![
            (task("t1", false, &[], &[]), "approved".into()),
            (
                task("big", true, &["src", "tests"], &["t1"]),
                "needs_user".into(),
            ),
            (task("after", false, &[], &["big"]), "queued".into()),
        ]
    }

    fn cut(parts: &[TaskSpec], plan: &[(TaskSpec, String)]) -> Result<Vec<TaskSpec>> {
        prepare(parts, &plan[1].0, plan)
    }

    fn rejects(parts: &[TaskSpec], plan: &[(TaskSpec, String)], text: &str) {
        let error = cut(parts, plan).unwrap_err().to_string();
        assert!(error.contains(text), "{error:?} lacks {text:?}");
    }

    #[test]
    fn parts_inherit_requirements_scope_and_what_the_whole_waited_for() {
        let parts = cut(
            &[
                task("a", true, &["src/a"], &[]),
                task("b", true, &["src/b"], &["a"]),
            ],
            &plan(),
        )
        .unwrap();
        assert_eq!(parts[0].part_of.as_deref(), Some("big"));
        assert_eq!(parts[0].depends_on, ["t1"]);
        assert_eq!(parts[1].depends_on, ["t1", "a"]);
        let bare = task("c", true, &[], &[]);
        let widened = cut(&[bare, task("d", false, &["tests"], &[])], &plan());
        assert_eq!(widened.unwrap()[0].scope, ["src", "tests"]);
    }

    #[test]
    fn a_split_makes_a_few_parts() {
        rejects(&[task("a", true, &["src"], &[])], &plan(), "2 to 8 parts");
        let many: Vec<_> = (0..9)
            .map(|n| task(&format!("p{n}"), false, &[], &[]))
            .collect();
        rejects(&many, &plan(), "2 to 8 parts");
    }

    #[test]
    fn parts_stay_inside_what_the_whole_was_given() {
        let two = |a: TaskSpec| [a, task("z", true, &["tests"], &[])];
        rejects(
            &two(task("a", true, &["docs"], &[])),
            &plan(),
            "outside the scope",
        );
        rejects(
            &two(task("a", true, &["srcs"], &[])),
            &plan(),
            "outside the scope",
        );
        let mut other = task("a", true, &["src"], &[]);
        other.requirement_ids = vec!["new".into()];
        rejects(&two(other), &plan(), "serves requirement new");
        let mut read_only = plan();
        read_only[1].0.writes = false;
        rejects(
            &two(task("a", true, &["src"], &[])),
            &read_only,
            "is read-only",
        );
    }

    #[test]
    fn dependencies_point_at_earlier_parts_or_tasks_ahead_of_the_whole() {
        let parts = |dep: &str| [task("a", false, &[], &[dep]), task("b", false, &[], &[])];
        rejects(&parts("b"), &plan(), "neither an earlier part");
        rejects(&parts("after"), &plan(), "neither an earlier part");
        rejects(&parts("big"), &plan(), "neither an earlier part");
        cut(&parts("t1"), &plan()).unwrap();
    }

    #[test]
    fn two_writers_on_the_same_files_need_an_order() {
        let racing = [
            task("a", true, &["src"], &[]),
            task("b", true, &["src/x"], &[]),
        ];
        rejects(&racing, &plan(), "nothing orders them");
        let ordered = [
            task("a", true, &["src"], &[]),
            task("b", true, &["src/x"], &["a"]),
        ];
        cut(&ordered, &plan()).unwrap();
        let apart = [
            task("a", true, &["src"], &[]),
            task("b", true, &["tests"], &[]),
        ];
        cut(&apart, &plan()).unwrap();
        let readers = [
            task("a", false, &["src"], &[]),
            task("b", false, &["src"], &[]),
        ];
        cut(&readers, &plan()).unwrap();
    }

    #[test]
    fn a_task_is_cut_once_and_a_part_not_at_all() {
        let parts = [
            task("a", true, &["src"], &[]),
            task("b", true, &["tests"], &[]),
        ];
        let mut again = plan();
        let mut done = task("a", true, &["src"], &[]);
        done.part_of = Some("big".into());
        again.push((done, "approved".into()));
        rejects(&parts, &again, "already cut once");
        let mut part = plan();
        part[1].0.part_of = Some("whole".into());
        rejects(&parts, &part, "itself a part");
    }

    #[test]
    fn ids_are_new_and_parts_clear_no_blocker() {
        rejects(
            &[task("t1", false, &[], &[]), task("b", false, &[], &[])],
            &plan(),
            "repeats",
        );
        let mut clearing = task("a", false, &[], &[]);
        clearing.resolves = Some("big".into());
        rejects(
            &[clearing, task("b", false, &[], &[])],
            &plan(),
            "does not clear a blocker",
        );
    }
}
