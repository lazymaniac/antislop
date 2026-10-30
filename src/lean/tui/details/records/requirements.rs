use super::*;

pub(super) fn requirements(
    details: &mut Details,
    _view: &View,
    records: &mut Vec<Rc<DetailRecord>>,
) {
    records.push(record(ledger_lines(details), " "));
}

fn ledger_lines(details: &Details) -> Vec<(Style, String)> {
    let mut lines = vec![label("Requirement ledger · shared across all rounds")];
    if let Some(stored) = &details.task {
        append_requirements(&mut lines, &stored["requirements"]);
    }
    lines.push(plain("Open the requirements (F2) for sources, evidence and version history. Task approval alone does not verify requirements.".into()));
    lines
}

fn append_requirements(lines: &mut Vec<(Style, String)>, value: &Value) {
    let requirements = value.as_array().map(Vec::as_slice).unwrap_or_default();
    if requirements.is_empty() {
        lines.push(plain("No requirements are assigned to this task.".into()));
    }
    for requirement in requirements {
        append_requirement(lines, requirement);
    }
}

fn append_requirement(lines: &mut Vec<(Style, String)>, requirement: &Value) {
    lines.push(plain(format!(
        "• {}",
        requirement["text"].as_str().unwrap_or_default()
    )));
    for criterion in requirement["criteria"].as_array().into_iter().flatten() {
        lines.push(plain(criterion_line(criterion)));
    }
}

fn criterion_line(criterion: &Value) -> String {
    let optional = if criterion["mandatory"] == false {
        " (optional)"
    } else {
        ""
    };
    format!(
        "    – {}{optional}",
        criterion["text"].as_str().unwrap_or_default()
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
