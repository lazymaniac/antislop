use super::*;
use crate::lean::tui::details::CheckRow;
#[path = "history.rs"]
mod history;

impl Details {
    pub(super) fn review_sections(
        &self,
        view: &View,
        theme: &Theme,
        width: usize,
        row: &TaskRow,
        rows: &[CheckRow],
        keys: &[String],
    ) -> Vec<Section> {
        let mut out = self.check_findings(theme, width, rows);
        out.extend(self.outcome_and_history(view, theme, width, row, keys));
        out
    }

    fn check_findings(&self, theme: &Theme, width: usize, rows: &[CheckRow]) -> Vec<Section> {
        let mut out = check_section(theme, width, rows)
            .into_iter()
            .collect::<Vec<_>>();
        out.extend(self.findings_section(theme, width));
        out
    }

    fn findings_section(&self, theme: &Theme, width: usize) -> Vec<Section> {
        self.task
            .as_ref()
            .and_then(|stored| {
                let verdict = &stored["verdict"];
                (!verdict.is_null()).then(|| finding_section(theme, width, verdict))
            })
            .into_iter()
            .collect()
    }
}

fn check_section(theme: &Theme, width: usize, rows: &[CheckRow]) -> Option<Section> {
    if rows.is_empty() {
        return None;
    }
    let settled = rows.iter().filter(|r| r.state == "settled").count();
    let issues = rows
        .iter()
        .filter(|r| matches!(r.status.as_deref(), Some("issue" | "blocked")))
        .count();
    let mut meta = gauge(theme, settled, rows.len(), 8, theme.status("approved"));
    meta.push(Span::styled(
        format!(" {settled}/{}", rows.len()),
        theme.dim(),
    ));
    if issues > 0 {
        meta.push(Span::styled(
            format!(" · {issues} with issues"),
            theme.error(),
        ));
    }
    Some(Section {
        id: "checks",
        title: "Checks",
        meta,
        body: check_body(theme, width, rows),
        preview: 14,
        right: true,
    })
}

fn check_body(theme: &Theme, width: usize, rows: &[CheckRow]) -> Vec<Line<'static>> {
    let mut ordered: Vec<_> = rows.iter().collect();
    ordered.sort_by_key(|r| check_rank(r));
    ordered
        .into_iter()
        .flat_map(|row| check_lines(theme, width, row))
        .collect()
}

fn check_rank(row: &CheckRow) -> u8 {
    match (row.state, row.status.as_deref()) {
        (_, Some("issue")) => 0,
        (_, Some("blocked")) => 1,
        ("checking", _) => 2,
        ("unaccounted", _) => 3,
        ("pending", _) => 4,
        _ => 5,
    }
}

fn check_lines(theme: &Theme, width: usize, row: &CheckRow) -> Vec<Line<'static>> {
    if row.state == "pending" {
        return pending_line(theme, width, row);
    }
    let mut lines = item(
        theme,
        Span::styled(
            row.mark,
            if theme.color {
                row.style
            } else {
                Style::default()
            },
        ),
        Some(row.id.clone()),
        &row.text,
        width,
        theme.text(),
    );
    lines.extend(
        row.note
            .as_ref()
            .into_iter()
            .flat_map(|note| wrap(&theme.clean(note), width.saturating_sub(4).max(1)))
            .map(|line| Line::styled(format!("    {line}"), theme.dim())),
    );
    lines
}

fn pending_line(theme: &Theme, width: usize, row: &CheckRow) -> Vec<Line<'static>> {
    let lead = format!("{} {} ", row.mark, row.id);
    let room = width.saturating_sub(lead.width()).max(1);
    vec![Line::from(vec![
        Span::styled(lead, theme.dim()),
        Span::styled(theme.fit(&theme.clean(&row.text), room), theme.dim()),
    ])]
}

fn finding_section(theme: &Theme, width: usize, verdict: &Value) -> Section {
    let mut findings: Vec<&Value> = verdict["findings"]
        .as_array()
        .map(|f| f.iter().collect())
        .unwrap_or_default();
    findings.sort_by_key(|finding| finding_rank(finding));
    let body = finding_body(theme, width, &findings);
    Section {
        id: "findings",
        title: "Findings",
        meta: vec![Span::styled(
            findings.len().to_string(),
            if findings.is_empty() {
                theme.dim()
            } else {
                theme.warning()
            },
        )],
        body,
        preview: 10,
        right: true,
    }
}

fn finding_body(theme: &Theme, width: usize, findings: &[&Value]) -> Vec<Line<'static>> {
    if findings.is_empty() {
        return vec![Line::styled(
            "No issues: the verifier approved without notes.",
            theme.status("approved"),
        )];
    }
    findings
        .iter()
        .flat_map(|finding| finding_lines(theme, width, finding))
        .collect()
}

fn finding_lines(theme: &Theme, width: usize, finding: &Value) -> Vec<Line<'static>> {
    let level = finding["severity"].as_str().unwrap_or("note");
    let style = match finding_rank(finding) {
        0 => theme.error(),
        1 => theme.warning(),
        _ => theme.dim(),
    };
    let mut lines = item(
        theme,
        Span::styled(theme.glyphs().problem, style),
        Some(format!("[{level}]")),
        finding["issue"].as_str().unwrap_or_default(),
        width,
        theme.text(),
    );
    if let Some(place) = finding["where"].as_str().filter(|place| !place.is_empty()) {
        lines.push(Line::styled(
            format!("    at {}", theme.clean(place)),
            theme.dim(),
        ));
    }
    lines
}

fn finding_rank(finding: &Value) -> u8 {
    match finding["severity"].as_str().unwrap_or("note") {
        "critical" | "high" | "blocker" => 0,
        "medium" | "major" => 1,
        _ => 2,
    }
}

impl Details {
    fn outcome_and_history(
        &self,
        view: &View,
        theme: &Theme,
        width: usize,
        row: &TaskRow,
        keys: &[String],
    ) -> Vec<Section> {
        history::outcome_and_history(self, view, theme, width, row, keys)
    }
}
