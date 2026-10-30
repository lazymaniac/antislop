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
            details.key.starts_with("verifier:"),
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
    lines.push(plain(format!(
        "Project: {}",
        view.plan.as_deref().unwrap_or("current")
    )));
    lines.push(plain(format!("Mode: {}", mode_label(view.autonomous))));
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
    lines.push(plain(format!(
        "Task reports: {done}/{total} ready (explicitly missing: {} in progress)",
        total.saturating_sub(done)
    )));
    record(lines, " ")
}

fn mode_label(autonomous: Option<bool>) -> &'static str {
    match autonomous {
        Some(true) => "autonomous",
        Some(false) => "supervised",
        None => "unspecified",
    }
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

fn task_record(view: &View, task: &TaskRow, verifier: bool) -> Rc<DetailRecord> {
    let mut lines = task_summary_lines(view, task);
    lines.push(plain(format!("Task {}: {}", task.id, task.title)));
    lines.push(plain(format!(
        "Kind: {}{}",
        task.kind,
        if task.writes {
            " · modifies files"
        } else {
            " · read-only"
        }
    )));
    append_list(&mut lines, "Depends on", &task.depends_on);
    append_list(&mut lines, "Scope", &task.scope);
    lines.push(plain(String::new()));
    lines.push(label("Goal"));
    lines.push(plain(task.goal.clone()));
    if verifier || !task.focus.is_empty() {
        lines.push(plain(String::new()));
        lines.push(label("Verification"));
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
