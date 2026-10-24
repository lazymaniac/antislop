use super::*;

/// What a comment's target is, in the orchestrator's words.
pub fn comment_label(target: &str, plan: &[Task]) -> String {
    if let Some(rest) = target.strip_prefix("plan:") {
        return plan_label(rest);
    }
    if let Some(rest) = target.strip_prefix("task:") {
        return format!("specification of {}", task_label(rest, plan));
    }
    if let Some(rest) = target.strip_prefix("check:") {
        return check_label(rest, plan);
    }
    if let Some(rest) = target.strip_prefix("finding:") {
        return finding_label(rest, plan);
    }
    if let Some(rest) = target.strip_prefix("report:") {
        return format!("report of {}", task_label(rest, plan));
    }
    if let Some(rest) = target.strip_prefix("artifact:") {
        return artifact_label(rest, plan);
    }
    addressed_target_label(target, plan)
}

fn task_label(id: &str, plan: &[Task]) -> String {
    plan.iter()
        .find(|task| task.id == id || task.spec["id"] == id)
        .map(|task| {
            format!(
                "task {} \"{}\"",
                task.spec["id"].as_str().unwrap_or(id),
                task.spec["title"].as_str().unwrap_or_default()
            )
        })
        .unwrap_or_else(|| format!("task {id}"))
}

fn plan_label(rest: &str) -> String {
    match rest.split_once(":line:") {
        Some((plan, line)) => format!("line {line} of the plan {plan}"),
        None => format!("the plan {rest}"),
    }
}

fn check_label(rest: &str, plan: &[Task]) -> String {
    match rest.split_once(':') {
        Some((task, check)) => format!("check {check} of {}", task_label(task, plan)),
        None => format!("check {rest}"),
    }
}

fn finding_label(rest: &str, plan: &[Task]) -> String {
    match rest.split_once(':') {
        Some((task, finding)) => format!("finding #{finding} of {}", task_label(task, plan)),
        None => format!("finding {rest}"),
    }
}

fn artifact_label(rest: &str, plan: &[Task]) -> String {
    match rest.split_once(':') {
        Some((task, path)) => format!("artifact {path} of {}", task_label(task, plan)),
        None => format!("artifact {rest}"),
    }
}

fn addressed_target_label(target: &str, plan: &[Task]) -> String {
    match target.split_once(':') {
        Some(("worker", id)) => format!("the worker of {}", task_label(id, plan)),
        Some(("verifier", id)) => format!("the verifier of {}", task_label(id, plan)),
        Some(("diff", _)) => diff_label(target, plan),
        _ => named_target_label(target),
    }
}

fn diff_label(target: &str, plan: &[Task]) -> String {
    let Some(diff::Target { task, file, line }) = diff::target(target) else {
        return target.into();
    };
    let change = format!("the change of {}", task_label(task, plan));
    diff_location_label(file, line, &change)
}

fn diff_location_label(file: Option<&str>, line: Option<&str>, change: &str) -> String {
    match (file, line) {
        (Some(file), Some(line)) => match line.strip_prefix("old") {
            Some(old) => format!("removed line {old} of {file} in {change}"),
            None => format!("line {line} of {file} in {change}"),
        },
        (Some(file), None) => format!("{file} in {change}"),
        _ => change.to_owned(),
    }
}

fn named_target_label(target: &str) -> String {
    match target {
        ORCHESTRATOR => "your own work".into(),
        PLAN_VERIFIER => "the plan verifier's review".into(),
        AUDITOR => "the auditor's report".into(),
        other => other.into(),
    }
}

/// Whether a comment target is the plan document, the whole plan or a line.
pub(super) fn is_plan_target(target: &str) -> bool {
    target == "plan" || target.starts_with("plan:")
}
