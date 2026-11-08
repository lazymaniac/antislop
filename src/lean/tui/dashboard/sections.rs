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
        let mut sections = Vec::new();
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
            "review",
            "Review",
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
        out.push(text_section("scope", "Dependencies", scope, 8));
    }
}

fn review_sections(context: &SectionContext<'_>) -> Vec<Section> {
    context.details.review_sections(
        context.view,
        context.theme,
        context.width,
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
    let reviewer = tab == super::super::details::REVIEWER;
    sections.retain(|section| match section.id {
        "goal" | "scope" => !reviewer,
        "review" | "risks" | "checks" | "findings" => reviewer,
        _ => true,
    });
}
