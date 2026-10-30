use super::*;

pub(super) fn synthesize_document(tasks: &[TaskRow]) -> String {
    if tasks.is_empty() {
        return "# Execution Plan\n\nNo tasks proposed yet.".into();
    }
    let mut out = String::from("# Execution Plan\n\n## Plan Sections & Task Breakdown\n\n");
    for (idx, task) in tasks.iter().enumerate() {
        append_task_section(&mut out, idx + 1, task);
    }
    out.trim_end().to_owned()
}

fn append_task_section(out: &mut String, number: usize, task: &TaskRow) {
    append_task_heading(out, number, task);
    append_task_facts(out, task);
}

fn append_task_heading(out: &mut String, number: usize, task: &TaskRow) {
    let section = task
        .section
        .as_deref()
        .filter(|section| !section.trim().is_empty())
        .unwrap_or(&task.title);
    out.push_str(&format!(
        "### {number}. {section} [{}] ({})\n",
        task.id,
        status_label(&task.status)
    ));
}

fn append_task_facts(out: &mut String, task: &TaskRow) {
    out.push_str(&format!("- **Task**: `{}` — {}\n", task.id, task.title));
    out.push_str(&format!(
        "- **Kind**: `{}` ({})\n",
        task.kind,
        if task.writes { "writes" } else { "read-only" }
    ));
    out.push_str(&format!("- **Goal**: {}\n", task.goal));
    out.push_str(&format!("- **Depends on**: {}\n", dependency_label(task)));
    out.push_str(&format!(
        "- **Parallel with**: {}\n\n",
        parallel_label(task)
    ));
}

fn dependency_label(task: &TaskRow) -> String {
    if task.depends_on.is_empty() {
        "none (can start immediately)".to_owned()
    } else {
        task.depends_on.join(", ")
    }
}

fn parallel_label(task: &TaskRow) -> String {
    if task.parallel_with.is_empty() {
        "sequential".to_owned()
    } else {
        task.parallel_with.join(", ")
    }
}

/// The tasks a heading stands for: the one it names as `[id]`, otherwise
/// every task assigned to a section of exactly that name, otherwise the task
/// titled like the heading.
pub(super) fn section_tasks(heading: &str, tasks: &[TaskRow]) -> Vec<String> {
    if let Some(task) = tasks
        .iter()
        .find(|task| heading.contains(&format!("[{}]", task.id)))
    {
        return vec![task.id.clone()];
    }
    let named = |name: &str| heading.eq_ignore_ascii_case(name.trim());
    let by_section: Vec<String> = tasks
        .iter()
        .filter(|task| {
            task.section
                .as_deref()
                .is_some_and(|section| !section.trim().is_empty() && named(section))
        })
        .map(|task| task.id.clone())
        .collect();
    if !by_section.is_empty() {
        return by_section;
    }
    tasks
        .iter()
        .filter(|task| !task.title.trim().is_empty() && named(&task.title))
        .map(|task| task.id.clone())
        .collect()
}

pub(super) fn section_state(ids: &[String], tasks: &[TaskRow]) -> SectionState {
    let statuses: Vec<&str> = ids
        .iter()
        .filter_map(|id| tasks.iter().find(|task| &task.id == id))
        .map(|task| task.status.as_str())
        .collect();
    if statuses.is_empty() {
        SectionState::Open
    } else if statuses
        .iter()
        .any(|s| matches!(*s, "failed" | "needs_user"))
    {
        SectionState::Problem
    } else if statuses.iter().all(|s| *s == "approved") {
        SectionState::Done
    } else if statuses
        .iter()
        .any(|s| matches!(*s, "working" | "verifying"))
    {
        SectionState::Working
    } else {
        SectionState::Open
    }
}

mod build;
