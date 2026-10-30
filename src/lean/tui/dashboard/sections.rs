use super::super::view::Agent;
use super::*;
use crate::lean::tui::details::CheckRow;
mod review;
mod scope;

struct SectionContext<'a> {
    details: &'a Details,
    view: &'a View,
    theme: &'a Theme,
    width: usize,
    row: &'a TaskRow,
    checks: &'a [CheckRow],
    keys: &'a [String],
}

impl Details {
    pub(in crate::lean::tui) fn sections(
        &self,
        view: &View,
        theme: &Theme,
        width: usize,
    ) -> Vec<Section> {
        let row = self.row(view);
        let keys = self.session_keys();
        let rows = self.task.as_ref().map(check_rows).unwrap_or_default();
        let mut sections = now_sections(view, theme, width, row, &keys);
        if let Some(row) = row {
            let context = SectionContext {
                details: self,
                view,
                theme,
                width,
                row,
                checks: &rows,
                keys: &keys,
            };
            append_task_sections(&context, &mut sections);
        }
        filter_tab_sections(self.tab, &mut sections);
        sections
    }
}

fn now_sections(
    view: &View,
    theme: &Theme,
    width: usize,
    row: Option<&TaskRow>,
    keys: &[String],
) -> Vec<Section> {
    let mut body = agent_activity(view, theme, width, row, keys);
    if let Some(row) = row {
        body.extend(stop_reason(theme, width, row));
        body.extend(next_step(view, theme, width, row));
    }
    if body.is_empty() {
        return Vec::new();
    }
    vec![Section {
        id: "now",
        title: "Now",
        meta: vec![],
        body,
        preview: 14,
        right: false,
    }]
}

fn agent_activity(
    view: &View,
    theme: &Theme,
    width: usize,
    row: Option<&TaskRow>,
    keys: &[String],
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for key in keys {
        if let Some(agent) = view.agents.get(key) {
            lines.extend(agent_lines(view, theme, width, row, key, agent));
        }
    }
    lines
}

fn agent_lines(
    view: &View,
    theme: &Theme,
    width: usize,
    row: Option<&TaskRow>,
    key: &str,
    agent: &Agent,
) -> Vec<Line<'static>> {
    let role = key.split(':').next().unwrap_or(key);
    let state = agent_state(theme, agent);
    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!("{} {role:<9}", theme.role_icon(key)),
            theme.role_accent(key, row.map_or("", |r| r.kind.as_str())),
        ),
        state,
    ])];
    if let Some(doing) = taskinfo::doing(view, key)
        .or_else(|| (!agent.activity.is_empty()).then(|| clean(&agent.activity)))
    {
        lines.extend(
            wrap(&theme.clean(&doing), width.saturating_sub(4).max(1))
                .into_iter()
                .take(2)
                .map(|line| Line::styled(format!("    {line}"), theme.text())),
        );
    }
    lines
}

fn agent_state(theme: &Theme, agent: &Agent) -> Span<'static> {
    if agent.running {
        let since = agent
            .started_ms
            .map(|at| {
                format!(
                    " for {}",
                    duration(now_ms().saturating_sub(at).max(0) as u64)
                )
            })
            .unwrap_or_default();
        Span::styled(format!("running{since}"), theme.status("working"))
    } else {
        Span::styled("idle", theme.dim())
    }
}

fn stop_reason(theme: &Theme, width: usize, row: &TaskRow) -> Vec<Line<'static>> {
    let Some(cause) = taskinfo::cause(row) else {
        return Vec::new();
    };
    let reason = row
        .reason
        .as_deref()
        .map(theme_clean_one(theme))
        .unwrap_or(cause);
    item(
        theme,
        Span::styled(theme.glyphs().problem, theme.error()),
        Some("stopped:".into()),
        &reason,
        width,
        theme.error(),
    )
}

fn next_step(view: &View, theme: &Theme, width: usize, row: &TaskRow) -> Vec<Line<'static>> {
    let mut lines = vec![Line::raw("")];
    lines.extend(para(
        theme,
        &taskinfo::next_step(view, row),
        width,
        theme.accent(),
    ));
    lines
}

fn append_task_sections(context: &SectionContext<'_>, out: &mut Vec<Section>) {
    append_primary_sections(context, out);
    out.extend(review_sections(context));
}

fn append_primary_sections(context: &SectionContext<'_>, out: &mut Vec<Section>) {
    let SectionContext {
        theme, width, row, ..
    } = context;
    append_goal_sections(theme, *width, row, out);
    append_risk_scope(context, out);
}

fn append_goal_sections(theme: &Theme, width: usize, row: &TaskRow, out: &mut Vec<Section>) {
    out.push(text_section(
        "goal",
        "Goal",
        para(theme, &row.goal, width, theme.text()),
        8,
    ));
    if !row.focus.trim().is_empty() {
        out.push(text_section(
            "verify",
            "Verification",
            para(theme, &row.focus, width, theme.text()),
            6,
        ));
    }
}

fn append_risk_scope(context: &SectionContext<'_>, out: &mut Vec<Section>) {
    let SectionContext {
        details,
        view,
        theme,
        width,
        row,
        checks,
        ..
    } = context;
    if !row.risks.is_empty() {
        out.push(risks_section(theme, *width, row, checks));
    }
    let scope = scope::lines(details, view, theme, *width, row);
    if !scope.is_empty() {
        out.push(text_section("scope", "Scope & dependencies", scope, 8));
    }
}

fn review_sections(context: &SectionContext<'_>) -> Vec<Section> {
    context.details.review_sections(
        context.view,
        context.theme,
        context.width,
        context.row,
        context.checks,
        context.keys,
    )
}

fn text_section(
    id: &'static str,
    title: &'static str,
    body: Vec<Line<'static>>,
    preview: usize,
) -> Section {
    Section {
        id,
        title,
        meta: vec![],
        body,
        preview,
        right: false,
    }
}

fn risks_section(theme: &Theme, width: usize, row: &TaskRow, checks: &[CheckRow]) -> Section {
    let mut body = Vec::new();
    for (index, risk) in row.risks.iter().enumerate() {
        let id = format!("R{}", index + 1);
        let (mark, style) = checks
            .iter()
            .find(|check| check.id == id)
            .map_or((theme.glyphs().queued, theme.dim()), |check| {
                (check.mark, check.style)
            });
        body.extend(item(
            theme,
            Span::styled(mark, if theme.color { style } else { Style::default() }),
            Some(id),
            risk,
            width,
            theme.text(),
        ));
    }
    Section {
        id: "risks",
        title: "Risks",
        meta: vec![Span::styled(row.risks.len().to_string(), theme.dim())],
        body,
        preview: 8,
        right: false,
    }
}

fn filter_tab_sections(tab: usize, sections: &mut Vec<Section>) {
    let verifier = tab == super::super::details::VERIFIER;
    sections.retain(|section| match section.id {
        "goal" | "scope" => !verifier,
        "verify" | "risks" | "checks" | "findings" => verifier,
        _ => true,
    });
}
