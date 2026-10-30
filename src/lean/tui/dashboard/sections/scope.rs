use super::*;

pub(super) fn lines(
    details: &Details,
    view: &View,
    theme: &Theme,
    width: usize,
    row: &TaskRow,
) -> Vec<Line<'static>> {
    let mut lines = row
        .scope
        .iter()
        .flat_map(|path| {
            item(
                theme,
                Span::styled(theme.glyphs().dot, theme.dim()),
                None,
                path,
                width,
                theme.text(),
            )
        })
        .collect::<Vec<_>>();
    lines.extend(dependencies(view, theme, row));
    lines.extend(metadata(details, theme, width, row));
    lines
}

fn dependencies(view: &View, theme: &Theme, row: &TaskRow) -> Vec<Line<'static>> {
    row.depends_on
        .iter()
        .map(|dep| {
            let task = view.tasks.iter().find(|task| &task.id == dep);
            let (mark, style, status) = task
                .map(|task| {
                    (
                        theme.status_glyph(&task.status),
                        theme.status(&task.status),
                        status_label(&task.status).to_owned(),
                    )
                })
                .unwrap_or((theme.glyphs().queued, theme.dim(), "unknown".to_owned()));
            Line::from(vec![
                Span::styled(mark, style),
                Span::styled(format!(" after {dep} "), theme.text()),
                Span::styled(status, theme.dim()),
            ])
        })
        .collect()
}

fn metadata(details: &Details, theme: &Theme, width: usize, row: &TaskRow) -> Vec<Line<'static>> {
    let spec = details.task.as_ref().map(|task| &task["spec"]);
    let mut lines = Vec::new();
    append_section(&mut lines, row, theme, width);
    append_requirements(&mut lines, spec, theme, width);
    append_resolution(&mut lines, spec, theme, width);
    append_parallel(&mut lines, row, theme);
    lines
}

fn append_section(lines: &mut Vec<Line<'static>>, row: &TaskRow, theme: &Theme, width: usize) {
    if let Some(section) = &row.section {
        lines.extend(para(
            theme,
            &format!("section: {section}"),
            width,
            theme.dim(),
        ));
    }
}

fn append_requirements(
    lines: &mut Vec<Line<'static>>,
    spec: Option<&Value>,
    theme: &Theme,
    width: usize,
) {
    let ids = spec
        .and_then(|spec| spec["requirement_ids"].as_array())
        .map(|ids| ids.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    if !ids.is_empty() {
        lines.extend(para(
            theme,
            &format!("serves requirements {}", ids.join(", ")),
            width,
            theme.dim(),
        ));
    }
}

fn append_resolution(
    lines: &mut Vec<Line<'static>>,
    spec: Option<&Value>,
    theme: &Theme,
    width: usize,
) {
    if let Some(resolves) = spec.and_then(|spec| spec["resolves"].as_str()) {
        lines.extend(para(
            theme,
            &format!("clears the blocker of {resolves}"),
            width,
            theme.dim(),
        ));
    }
}

fn append_parallel(lines: &mut Vec<Line<'static>>, row: &TaskRow, theme: &Theme) {
    if !row.parallel_with.is_empty() {
        lines.push(Line::styled(
            format!(
                "{} parallel with {}",
                theme.glyphs().dot,
                row.parallel_with.join(", ")
            ),
            theme.dim(),
        ));
    }
}
