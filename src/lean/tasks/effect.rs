use super::*;

/// The bounded effect a card shows for a writer's change: what the diff
/// summed to and which files it touched, never the diff or the report.
/// Names are dropped from the end, never the counts, so a cut never hides
/// that there are more files. The flag says whether the stored text was
/// cut by [`bound_cut`].
pub fn effect(stat: &str, files: &[String]) -> (String, bool) {
    if files.is_empty() {
        return ("no file changes".into(), false);
    }
    let summary = stat
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.contains("changed") || line.contains("insertion"))
        .map_or_else(|| format!("{} file(s) changed", files.len()), str::to_owned);
    let mut shown = files.len().min(FILE_LIMIT);
    loop {
        let dropped = files.len() - shown;
        let candidate = if shown == 0 {
            format!("{summary} (+{dropped} more files)")
        } else {
            let names: Vec<&str> = files.iter().take(shown).map(String::as_str).collect();
            let more = if dropped > 0 {
                format!(" (+{dropped} more)")
            } else {
                String::new()
            };
            format!("{summary} — {}{more}", names.join(", "))
        };
        if shown == 0 || candidate.len() <= EFFECT_LIMIT {
            return bound_cut(&candidate, EFFECT_LIMIT);
        }
        shown -= 1;
    }
}

/// The bounded effect a card shows for a read-only task: the first text
/// line of its report outside code fences, not the report. The flag says
/// whether the stored text was cut by [`bound_cut`].
pub fn report_effect(report: &str) -> (String, bool) {
    let mut in_fence = false;
    for line in report.lines() {
        let line = line.trim();
        if line.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || line.is_empty() {
            continue;
        }
        return bound_cut(line, EFFECT_LIMIT);
    }
    (String::new(), false)
}

/// The two card-feedback keys of a bounded effect: its text (`null` when
/// the round has not stated one) and the explicit `effect_cut` flag, set
/// when [`bound_cut`] trimmed the text at the moment of the trim. A
/// consumer must read the flag, not the text's last grapheme: honest text
/// can end in `…` and a kept whole oversized cluster carries no mark.
/// Feedback written before this flag existed has neither key and reads
/// as "no cut" to a frontend falling back on the old mark.
pub fn effect_keys(effect: &Option<(String, bool)>) -> (Value, Value) {
    match effect {
        Some((text, cut)) => (Value::String(text.clone()), serde_json::json!(*cut)),
        None => (Value::Null, serde_json::json!(false)),
    }
}
