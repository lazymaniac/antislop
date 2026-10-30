use super::*;
use serde_json::Value;

/// A human name for a comment's target.
pub fn target_name(target: &str) -> String {
    if let Some(diff) = crate::lean::diff::target(target) {
        return match (diff.file, diff.line) {
            (Some(file), Some(line)) => format!("{} diff {file}:{line}", diff.task),
            (Some(file), None) => format!("{} diff {file}", diff.task),
            _ => format!("{} diff", diff.task),
        };
    }
    if let Some((kind, rest)) = target.split_once(':')
        && let Some(name) = structured_target_name(kind, rest)
    {
        return name;
    }
    agent_name(target)
}

fn structured_target_name(kind: &str, rest: &str) -> Option<String> {
    Some(match kind {
        "plan" => plan_target_name(rest),
        "task" => format!("Task {rest}"),
        "check" => compound_target_name(rest, "check", "Check"),
        "finding" => compound_target_name(rest, "finding #", "Finding"),
        "report" => format!("Task {rest} report"),
        "artifact" => compound_target_name(rest, "artifact", "Artifact"),
        _ => return None,
    })
}

fn plan_target_name(rest: &str) -> String {
    if let Some((plan, line)) = rest.split_once(":line:") {
        return format!("Plan {plan} line {line}");
    }
    if let Some(line) = rest.strip_prefix("line:") {
        return format!("Plan line {line}");
    }
    format!("Plan {rest}")
}

fn compound_target_name(rest: &str, label: &str, fallback: &str) -> String {
    match rest.split_once(':') {
        Some((task, item)) => format!("Task {task} {label} {item}"),
        None => format!("{fallback} {rest}"),
    }
}

/// A human name for an agent key.
pub fn agent_name(key: &str) -> String {
    match key.split_once(':') {
        Some(("worker", id)) => format!("{id} worker"),
        Some(("verifier", id)) => format!("{id} verifier"),
        _ => match key {
            ORCHESTRATOR => "orchestrator".into(),
            PLAN_VERIFIER => "plan verifier".into(),
            AUDITOR => "auditor".into(),
            "conversation" => "conversation".into(),
            other => other.into(),
        },
    }
}

pub fn status_label(status: &str) -> &str {
    match status {
        "proposed" => "proposed",
        "queued" => "queued",
        "working" => "working",
        "verifying" => "verifying",
        "approved" => "approved",
        "coordinating" => "awaiting orchestrator",
        "needs_user" => "needs decision",
        "failed" => "failed",
        "skipped" => "skipped",
        "cancelled" => "cancelled",
        "waiting" => "waiting",
        "revised" => "sent back",
        other => other,
    }
}

/// A duration as people say it: `45 s`, `3 min 12 s`, `2 h 5 min`.
pub fn duration(ms: u64) -> String {
    let seconds = ms / 1000;
    match seconds {
        0..60 => format!("{seconds} s"),
        60..3600 => format!("{} min {} s", seconds / 60, seconds % 60),
        _ => format!("{} h {} min", seconds / 3600, seconds % 3600 / 60),
    }
}

/// Agents' text may carry terminal escapes; keep it printable.
pub fn clean(text: &str) -> String {
    crate::lean::tui::sanitize::sanitize(text)
}

/// Agents' single-line fields (titles, ids, paths, sections) as one
/// printable line, folded before they ever reach a row; a break or a
/// smuggled escape can never shift or corrupt the line it is drawn on.
pub fn clean_line(text: &str) -> String {
    crate::lean::tui::sanitize::sanitize_line(text)
}

/// A coverage preview folded into its bounded shape: every listed unknown
/// id becomes a printable line within [`UNKNOWN_ID_BYTES`], while the
/// array keeps its full length so the renderer can still name the real
/// count. Feedback written after the daemon's own preview bound already
/// satisfies every one of these and comes back unchanged, `unknown_more`
/// included.
pub fn bounded_coverage(coverage: &Value) -> Value {
    let mut coverage = coverage.clone();
    if let Some(ids) = coverage.get_mut("unknown").and_then(Value::as_array_mut) {
        for id in ids.iter_mut() {
            if let Some(text) = id.as_str() {
                *id = Value::String(bound(&clean_line(text), UNKNOWN_ID_BYTES));
            }
        }
    }
    coverage
}

/// A landing folded into its bounded shape: the printable detail line cut
/// at [`DETAIL_LIMIT`] (marked when it had to be cut, like every bounded
/// projection), everything else riding along whole. The full landing text
/// stays in the task's report and mail, outside this compact row.
pub fn bounded_landing(integration: &Value) -> Value {
    let mut integration = integration.clone();
    if let Some(object) = integration.as_object_mut()
        && let Some(detail) = object.get("detail").and_then(Value::as_str)
    {
        let detail = bound(&clean_line(detail), DETAIL_LIMIT);
        object.insert("detail".into(), Value::String(detail));
    }
    integration
}

/// Word-wraps `text` to `width` columns, breaking long words safely by grapheme clusters.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    crate::lean::tui::theme::wrap(text, width)
}

/// One line of at most `width` columns.
pub fn truncate(text: &str, width: usize) -> String {
    let t = crate::lean::tui::theme::Theme::new(true, false);
    t.fit(text, width)
}

/// Renders the minimum terminal size warning guard when terminal is smaller than 12x5.
/// Returns true if the guard was triggered and drawn.
pub fn draw_min_term_guard(frame: &mut ratatui::Frame, area: Rect) -> bool {
    if area.width < MIN_TERM_WIDTH || area.height < MIN_TERM_HEIGHT {
        let msg = format!("Terminal too small (min. {MIN_TERM_WIDTH}x{MIN_TERM_HEIGHT} required)");
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(msg)
                .wrap(ratatui::widgets::Wrap { trim: false })
                .style(
                    Style::default()
                        .fg(crate::lean::tui::theme::ayu::YELLOW)
                        .add_modifier(Modifier::BOLD),
                ),
            area,
        );
        true
    } else {
        false
    }
}
