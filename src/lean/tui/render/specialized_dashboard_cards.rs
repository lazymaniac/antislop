use super::super::dashboard::Fold;
use super::super::*;

/// Draw one focused card as its header and currently visible body lines.
pub(super) fn dashboard_card_lines(
    details: &Details,
    section: &super::super::dashboard::Section,
    focused: bool,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let fold = details.fold_of(section.id);
    let shown = visible_card_body(fold, section);
    let mut lines = vec![dashboard_card_header(section, fold, focused, theme)];
    lines.extend(dashboard_card_body(section, shown, focused, theme));
    add_dashboard_card_tail(&mut lines, section, fold, focused, shown, theme);
    lines
}

fn visible_card_body(fold: Fold, section: &super::super::dashboard::Section) -> usize {
    match fold {
        Fold::Folded => 0,
        Fold::Preview => section.body.len().min(section.preview),
        Fold::Full => section.body.len(),
    }
}

fn dashboard_card_header(
    section: &super::super::dashboard::Section,
    fold: Fold,
    focused: bool,
    theme: &Theme,
) -> Line<'static> {
    let marker = card_marker(fold, theme);
    let title_style = if focused {
        theme.title().add_modifier(Modifier::UNDERLINED)
    } else {
        theme.title()
    };
    let mut header = vec![
        Span::styled(format!("{marker} {}", section.title), title_style),
        Span::raw("  "),
    ];
    header.extend(section.meta.iter().cloned());
    Line::from(header)
}

fn card_marker(fold: Fold, theme: &Theme) -> &'static str {
    let marker = if fold == Fold::Folded {
        theme.glyphs().arrow_right
    } else {
        "▾"
    };
    if theme.ascii && marker == "▾" {
        "v"
    } else {
        marker
    }
}

fn dashboard_card_body(
    section: &super::super::dashboard::Section,
    shown: usize,
    focused: bool,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let rail_style = if focused {
        theme.accent()
    } else {
        theme.faint()
    };
    section
        .body
        .iter()
        .take(shown)
        .map(|body| {
            let mut spans = vec![Span::styled(
                format!("{} ", theme.glyphs().vertical),
                rail_style,
            )];
            spans.extend(body.spans.iter().cloned().map(|mut span| {
                span.style = body.style.patch(span.style);
                span
            }));
            Line::from(spans)
        })
        .collect()
}

fn add_dashboard_card_tail(
    lines: &mut Vec<Line<'static>>,
    section: &super::super::dashboard::Section,
    fold: Fold,
    focused: bool,
    shown: usize,
    theme: &Theme,
) {
    let hidden = section.body.len() - shown;
    if hidden > 0 && fold != Fold::Folded {
        lines.push(hidden_card_line(hidden, focused, theme));
    } else if fold == Fold::Folded && focused {
        lines.push(folded_card_line(section.body.len(), theme));
    }
}

fn hidden_card_line(hidden: usize, focused: bool, theme: &Theme) -> Line<'static> {
    let rail = if focused {
        theme.accent()
    } else {
        theme.faint()
    };
    Line::from(vec![
        Span::styled(format!("{} ", theme.glyphs().vertical), rail),
        Span::styled(
            format!(
                "{} +{hidden} more{}",
                theme.glyphs().ellipsis,
                if focused { " · Enter expands" } else { "" }
            ),
            theme.dim(),
        ),
    ])
}

fn folded_card_line(hidden: usize, theme: &Theme) -> Line<'static> {
    Line::styled(
        format!(
            "  {} {hidden} hidden · Enter unfolds",
            theme.glyphs().ellipsis
        ),
        theme.faint(),
    )
}
