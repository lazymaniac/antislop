use super::super::super::view::{Agent, TaskRow};
use super::*;

pub(super) fn overview(details: &mut Details, view: &View, records: &mut Vec<Rc<DetailRecord>>) {
    if details.key == "conversation" {
        records.push(conversation_record(view));
        return;
    }
    if !details.show_info {
        return;
    }
    append_agent_records(details, view, records);
}

fn append_agent_records(details: &Details, view: &View, records: &mut Vec<Rc<DetailRecord>>) {
    let agent = view.agents.get(&details.key);
    if let Some(agent) = agent.filter(|agent| !agent.provider.is_empty()) {
        records.push(cli_record(agent));
    }
    let task = details
        .task_id()
        .and_then(|id| view.tasks.iter().find(|task| task.id == id));
    if let Some(task) = task {
        records.push(task_record(
            view,
            task,
            details.key.starts_with("reviewer:"),
        ));
    }
    match agent {
        Some(agent) => records.push(activity_record(agent)),
        None if task.is_none() => records.push(record(
            vec![plain("This agent has not worked yet.".into())],
            " ",
        )),
        None => {}
    }
}

fn conversation_record(view: &View) -> Rc<DetailRecord> {
    let mut lines = vec![label("Conversation details")];
    lines.push(plain(format!("Status: {}", conversation_status(view))));
    if let Some(plan) = &view.plan {
        lines.push(plain(format!("Active plan: {plan}")));
    }
    let total = view.tasks.len();
    let done = view
        .tasks
        .iter()
        .filter(|task| status::finished(&task.status))
        .count();
    lines.push(plain(format!("Tasks: total {total}, completed {done}")));
    record(lines, " ")
}

fn conversation_status(view: &View) -> &'static str {
    if view.paused {
        "paused"
    } else if view.idle {
        "idle"
    } else {
        "active"
    }
}

fn cli_record(agent: &Agent) -> Rc<DetailRecord> {
    let mut cli = format!("CLI: {}", agent.provider);
    if let Some(model) = &agent.model {
        cli.push_str(&format!(" · model {model}"));
    }
    if let Some(effort) = &agent.effort {
        cli.push_str(&format!(" · effort {effort}"));
    }
    let mut lines = vec![plain(cli)];
    if agent.turns > 0 {
        lines.push(plain(format!("Tokens: {}", agent.tokens())));
    }
    if let Some(last) = agent.last_ms {
        lines.push(plain(format!(
            "Time in turns: {} (last {})",
            duration(agent.busy_ms),
            duration(last)
        )));
    }
    record(lines, " ")
}

fn task_record(view: &View, task: &TaskRow, reviewer: bool) -> Rc<DetailRecord> {
    let mut lines = task_summary_lines(view, task);
    lines.push(plain(format!("Task {}: {}", task.id, task.title)));
    lines.push(plain(format!(
        "Kind: {}{}",
        task.kind,
        if task.change_kind.has_delivery() {
            " · modifies files"
        } else {
            " · read-only"
        }
    )));
    append_list(&mut lines, "Depends on", &task.depends_on);
    lines.push(plain(String::new()));
    lines.push(label("Goal"));
    lines.push(plain(
        crate::lean::tui::view::task_goal(
            &task.kind,
            &task.title,
            &task.goal,
            task.summary.as_deref(),
        )
        .into_owned(),
    ));
    if task
        .summary
        .as_deref()
        .is_some_and(|summary| !summary.trim().is_empty())
        && !task.goal.trim().is_empty()
    {
        let context = readable_goal_context(&task.goal);
        if !context.is_empty() {
            lines.push(plain(String::new()));
            lines.push(label("Additional goal context"));
            lines.extend(context.lines().map(|line| plain(line.to_owned())));
        }
    }
    if let Some(criteria) = view
        .task_meta
        .get(&task.id)
        .map(|meta| &meta.acceptance_criteria)
        && !criteria.is_empty()
    {
        lines.push(plain(String::new()));
        lines.push(label("Acceptance criteria"));
        lines.extend(
            criteria
                .iter()
                .map(|criterion| plain(format!("{}: {}", criterion.id, criterion.text))),
        );
    }
    if reviewer || !task.focus.is_empty() {
        lines.push(plain(String::new()));
        lines.push(label("Review"));
        lines.push(plain(task.focus.clone()));
    }
    if !task.risks.is_empty() {
        lines.push(plain(String::new()));
        lines.push(label("Risks"));
        lines.extend(task.risks.iter().map(|risk| plain(format!("- {risk}"))));
    }
    record(lines, " ")
}

fn task_summary_lines(view: &View, task: &TaskRow) -> Vec<(Style, String)> {
    let round = taskinfo::round_shown(task.round).map_or_else(
        || " (not started yet)".into(),
        |round| format!(" (round {round})"),
    );
    let mut lines = vec![plain(format!(
        "Task status: {}{round}",
        status_label(&task.status)
    ))];
    if let Some(reason) = &task.reason {
        lines.push(plain(format!("Reason: {reason}")));
    } else if let Some(why) = taskinfo::cause(task) {
        lines.push(plain(format!("Reason: {why}")));
    }
    append_task_status(&mut lines, view, task);
    lines
}

fn append_task_status(lines: &mut Vec<(Style, String)>, view: &View, task: &TaskRow) {
    if let Some(effect) = taskinfo::effect_line(task) {
        lines.push(plain(effect));
    }
    lines.push(plain(format!(
        "Obligations: {}",
        taskinfo::coverage_full(task)
    )));
    lines.push(plain(format!(
        "Integration: {}",
        taskinfo::integration_line(task)
    )));
    lines.push(plain(taskinfo::next_step(view, task)));
}

fn append_list(lines: &mut Vec<(Style, String)>, label: &str, values: &[String]) {
    if !values.is_empty() {
        lines.push(plain(format!("{label}: {}", values.join(", "))));
    }
}

fn activity_record(agent: &Agent) -> Rc<DetailRecord> {
    let state = if agent.running { "working" } else { "idle" };
    record(
        vec![
            plain(String::new()),
            label("Now"),
            plain(format!("{state} · {}", agent.activity)),
        ],
        " ",
    )
}

fn plain(text: String) -> (Style, String) {
    (Style::default(), text)
}

fn label(text: &str) -> (Style, String) {
    (
        Style::default().add_modifier(Modifier::BOLD),
        text.to_owned(),
    )
}

fn readable_goal_context(goal: &str) -> String {
    let mut output = String::with_capacity(goal.len());
    let mut cursor = 0;
    let bytes = goal.as_bytes();
    while cursor < bytes.len() {
        let Some(relative_start) = bytes[cursor..]
            .iter()
            .position(|byte| matches!(byte, b'{' | b'['))
        else {
            output.push_str(&goal[cursor..]);
            break;
        };
        let start = cursor + relative_start;
        output.push_str(&goal[cursor..start]);
        if let Some(end) = json_value_end(goal, start) {
            cursor = end;
        } else {
            output.push(bytes[start] as char);
            cursor = start + 1;
        }
    }
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn json_value_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut stack = Vec::new();
    let mut quoted = false;
    let mut escaped = false;
    for (offset, byte) in bytes.iter().copied().enumerate().skip(start) {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'{' | b'[' => stack.push(byte),
            b'}' | b']' => {
                let opening = stack.pop()?;
                if !matches!((opening, byte), (b'{', b'}') | (b'[', b']')) {
                    return None;
                }
                if stack.is_empty() {
                    let end = offset + 1;
                    serde_json::from_str::<serde_json::Value>(&text[start..end]).ok()?;
                    return Some(end);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_record_formats_full_context_without_embedded_json() {
        let task = TaskRow {
            id: "t1".into(),
            title: "Storage".into(),
            kind: "implementation".into(),
            summary: Some("Improve the storage experience.".into()),
            goal: "Add a readable storage summary. Run cargo test afterwards. {\"runtime\":\"internal\"}".into(),
            ..Default::default()
        };
        let rendered = task_record(&View::default(), &task, false)
            .lines
            .iter()
            .map(|(_, line)| line.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("Add a readable storage summary."));
        assert!(rendered.contains("Run cargo test afterwards."));
        assert!(!rendered.contains("\"runtime\""));
        assert!(rendered.contains("Additional goal context"));
    }
}
