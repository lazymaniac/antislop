//! Mechanical checks that a proposed plan can be carried out by the agents
//! it names, run before any agent starts. A task asked for something its
//! permissions rule out would otherwise burn correction rounds on a blocker
//! no round can cure; the orchestrator hears of it while the plan is still
//! a proposal.
use std::collections::{BTreeMap, BTreeSet};

use crate::config::WorkKind;
use crate::lean::tasks::TaskSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The plan cannot work as written; it goes back before review.
    Block,
    /// Probably wrong, but a phrase can mislead; the plan verifier decides.
    Advise,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub task: String,
    pub rule: &'static str,
    pub severity: Severity,
    pub detail: String,
}

/// Phrases that ask a task to execute something, which a read-only task
/// may not do.
const EXECUTION: [&str; 11] = [
    "run cargo",
    "cargo test",
    "cargo build",
    "cargo clippy",
    "cargo check",
    "cargo fmt",
    "run the test",
    "run tests",
    "run the script",
    "run the checker",
    "execute ",
];

/// Words before a phrase that turn the request into a prohibition.
const NEGATIONS: [&str; 5] = ["not ", "n't ", "never ", "without ", "no "];

/// Words that make a task wait for another task's output.
const WAITING: [&str; 6] = [
    "after ",
    "once ",
    "from ",
    "requires ",
    "needs ",
    "wait for ",
];

pub fn check(specs: &[TaskSpec]) -> Vec<Finding> {
    let descendants = descendants(specs);
    let mut findings = Vec::new();
    for spec in specs {
        scope_escape(spec, &mut findings);
        read_only_writer(spec, &mut findings);
        asks_to_execute(spec, &mut findings);
        waits_for_descendant(spec, &descendants, &mut findings);
    }
    findings
}

fn finding(spec: &TaskSpec, rule: &'static str, severity: Severity, detail: String) -> Finding {
    Finding {
        task: spec.id.clone(),
        rule,
        severity,
        detail,
    }
}

/// A writer works in its own worktree; a scope outside the project is
/// outside what it may change.
fn scope_escape(spec: &TaskSpec, out: &mut Vec<Finding>) {
    if !spec.writes {
        return;
    }
    for path in &spec.scope {
        let escapes = path.starts_with('/') || path.split('/').any(|part| part == "..");
        if escapes {
            out.push(finding(
                spec,
                "writer-scope-outside-project",
                Severity::Block,
                format!("writes but its scope {path:?} lies outside the project worktree"),
            ));
        }
    }
}

fn read_only_writer(spec: &TaskSpec, out: &mut Vec<Finding>) {
    if !spec.writes && spec.kind == WorkKind::Implementation {
        out.push(finding(
            spec,
            "implementation-is-read-only",
            Severity::Block,
            "has kind `implementation` but `writes` is false, so it cannot change anything".into(),
        ));
    }
}

fn asks_to_execute(spec: &TaskSpec, out: &mut Vec<Finding>) {
    if spec.writes {
        return;
    }
    let goal = spec.goal.to_lowercase();
    let phrase = EXECUTION.iter().find(|phrase| requested(&goal, phrase));
    if let Some(phrase) = phrase {
        out.push(finding(
            spec,
            "read-only-asked-to-execute",
            Severity::Advise,
            format!("is read-only but its goal asks for {:?}", phrase.trim()),
        ));
    }
}

/// Whether `phrase` appears in `text` and is not preceded by a negation.
fn requested(text: &str, phrase: &str) -> bool {
    text.match_indices(phrase).any(|(at, _)| {
        let start = text[..at]
            .char_indices()
            .rev()
            .nth(23)
            .map_or(0, |(i, _)| i);
        let before = &text[start..at];
        !NEGATIONS.iter().any(|negation| before.contains(negation))
    })
}

fn waits_for_descendant(
    spec: &TaskSpec,
    descendants: &BTreeMap<&str, BTreeSet<&str>>,
    out: &mut Vec<Finding>,
) {
    let goal = spec.goal.to_lowercase();
    let Some(later) = descendants.get(spec.id.as_str()) else {
        return;
    };
    for id in later {
        let waits = WAITING
            .iter()
            .any(|word| goal.contains(&format!("{word}{}", id.to_lowercase())));
        if waits {
            out.push(finding(
                spec,
                "waits-for-its-own-dependent",
                Severity::Advise,
                format!("its goal waits for {id}, which depends on it"),
            ));
        }
    }
}

/// For each task, the tasks that depend on it directly or through others.
fn descendants(specs: &[TaskSpec]) -> BTreeMap<&str, BTreeSet<&str>> {
    let mut map: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for spec in specs {
        for dependency in &spec.depends_on {
            map.entry(dependency).or_default().insert(&spec.id);
        }
    }
    loop {
        let mut grew = false;
        for id in map.keys().copied().collect::<Vec<_>>() {
            let reached: Vec<&str> = map[id].iter().copied().collect();
            for next in reached {
                let more = map.get(next).cloned().unwrap_or_default();
                let set = map.get_mut(id).expect("key came from the map");
                let before = set.len();
                set.extend(more);
                grew |= set.len() != before;
            }
        }
        if !grew {
            return map;
        }
    }
}

/// What the orchestrator is told when `findings` send the plan back.
pub fn message(findings: &[&Finding]) -> String {
    let lines: Vec<String> = findings
        .iter()
        .map(|f| format!("- {} [{}]: {}", f.task, f.rule, f.detail))
        .collect();
    format!(
        "The runtime checked the proposed plan before any agent started and found tasks that \
         cannot be carried out as written:\n{}\n\nReturn the complete corrected ```plan block for \
         the same requirements, changing only what these findings require: fix the scope, \
         `writes` or dependencies, or move the work that needs it into a task that has the \
         permission and runs first.",
        lines.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lean::tasks::VerifierSpec;

    fn task(id: &str, kind: WorkKind, writes: bool, depends_on: &[&str]) -> TaskSpec {
        TaskSpec {
            id: id.into(),
            requirement_ids: Vec::new(),
            criterion_ids: Vec::new(),
            title: id.into(),
            goal: "Read the code and report".into(),
            summary: None,
            section: None,
            scope: vec!["src".into()],
            writes,
            depends_on: depends_on.iter().map(|d| (*d).to_owned()).collect(),
            kind,
            verifier: VerifierSpec {
                focus: "check".into(),
                summary: None,
                risks: Vec::new(),
            },
            resolves: None,
        }
    }

    fn rules(specs: &[TaskSpec]) -> Vec<&'static str> {
        check(specs).iter().map(|f| f.rule).collect()
    }

    #[test]
    fn a_sound_plan_has_no_findings() {
        let plan = [
            task("t1", WorkKind::Context, false, &[]),
            task("t2", WorkKind::Implementation, true, &["t1"]),
        ];
        assert!(check(&plan).is_empty());
    }

    #[test]
    fn a_writer_cannot_be_scoped_outside_the_project() {
        for path in ["/etc", "../sibling", "src/../../x"] {
            let mut writer = task("t1", WorkKind::Implementation, true, &[]);
            writer.scope = vec![path.into()];
            assert_eq!(rules(&[writer]), ["writer-scope-outside-project"], "{path}");
        }
        let mut reader = task("t1", WorkKind::Context, false, &[]);
        reader.scope = vec!["../service".into()];
        assert!(check(&[reader]).is_empty());
    }

    #[test]
    fn implementation_needs_write_access() {
        let plan = [task("t1", WorkKind::Implementation, false, &[])];
        assert_eq!(rules(&plan), ["implementation-is-read-only"]);
        assert_eq!(check(&plan)[0].severity, Severity::Block);
    }

    #[test]
    fn a_read_only_task_is_not_asked_to_execute_unless_it_is_forbidden() {
        let mut ask = task("t1", WorkKind::Analysis, false, &[]);
        ask.goal = "Record HEAD, then run cargo test and report".into();
        assert_eq!(rules(&[ask.clone()]), ["read-only-asked-to-execute"]);
        ask.goal = "Record HEAD. Do not run cargo test; report only".into();
        assert!(check(&[ask.clone()]).is_empty());
        ask.writes = true;
        ask.goal = "Run cargo test".into();
        assert!(check(&[ask]).is_empty());
    }

    #[test]
    fn a_task_waiting_for_its_own_dependent_is_found_through_the_chain() {
        let mut first = task("t2", WorkKind::Options, false, &[]);
        first.goal = "Compare variants once t4 has certified the inventory".into();
        let plan = [
            first,
            task("t3", WorkKind::Analysis, false, &["t2"]),
            task("t4", WorkKind::StaticAnalysis, false, &["t3"]),
        ];
        assert_eq!(rules(&plan), ["waits-for-its-own-dependent"]);
        let mut owner = task("t2", WorkKind::Options, false, &[]);
        owner.goal = "t4 owns the checker; compare variants".into();
        assert!(check(&[owner, plan[1].clone(), plan[2].clone()]).is_empty());
    }

    #[test]
    fn the_message_names_each_task_and_rule() {
        let plan = [task("t1", WorkKind::Implementation, false, &[])];
        let found = check(&plan);
        let text = message(&found.iter().collect::<Vec<_>>());
        assert!(text.contains("- t1 [implementation-is-read-only]"));
        assert!(text.contains("```plan"));
    }
}
