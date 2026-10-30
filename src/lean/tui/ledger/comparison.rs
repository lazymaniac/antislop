mod matching;
use crate::lean::tui::{Theme, sanitize};
use matching::changed_graphemes;
#[cfg(test)]
use matching::lcs_pairs;
use ratatui::{
    text::{Line, Span},
    widgets::{Block, Borders},
};
use serde_json::Value;

pub(super) fn panel<'a>(theme: &Theme, title: &'a str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(theme.separator(false))
        .border_set(if theme.ascii {
            ratatui::symbols::border::Set {
                top_left: "+",
                top_right: "+",
                bottom_left: "+",
                bottom_right: "+",
                vertical_left: "|",
                vertical_right: "|",
                horizontal_top: "-",
                horizontal_bottom: "-",
            }
        } else {
            ratatui::symbols::border::ROUNDED
        })
        .title(Span::styled(title, theme.title()))
}

pub(super) fn readable(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::String(text) => sanitize(text),
        Value::Bool(value) => if *value { "Yes" } else { "No" }.into(),
        Value::Array(values) => {
            if values.is_empty() {
                "None".into()
            } else {
                values.iter().map(readable).collect::<Vec<_>>().join("; ")
            }
        }
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| format!("{}: {}", key.replace('_', " "), readable(value)))
            .collect::<Vec<_>>()
            .join("; "),
        Value::Number(value) => value.to_string(),
    }
}

// Match lists independently; criterion identity survives edits to its text.
pub(super) fn list_diff_lines(
    before: Option<&Value>,
    after: &Value,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let empty = Vec::new();
    let old = before.and_then(Value::as_array).unwrap_or(&empty);
    let new = after.as_array().unwrap_or(&empty);
    let mut lines = Vec::new();
    append_changed_items(&mut lines, old, new, theme, width);
    append_added_items(&mut lines, old, new, theme, width);
    lines
}

fn same_list_item(left: &Value, right: &Value) -> bool {
    if left.is_object() && right.is_object() {
        left.get("id") == right.get("id")
    } else {
        left == right
    }
}

fn append_changed_items(
    out: &mut Vec<Line<'static>>,
    old: &[Value],
    new: &[Value],
    theme: &Theme,
    width: usize,
) {
    for item in old {
        let next = new.iter().find(|candidate| same_list_item(item, candidate));
        if next != Some(item) {
            let after = next.map(readable).unwrap_or_default();
            out.extend(text_diff_lines(&readable(item), &after, theme, width));
        }
    }
}

fn append_added_items(
    out: &mut Vec<Line<'static>>,
    old: &[Value],
    new: &[Value],
    theme: &Theme,
    width: usize,
) {
    for item in new {
        if !old.iter().any(|candidate| same_list_item(item, candidate)) {
            out.extend(text_diff_lines("", &readable(item), theme, width));
        }
    }
}

// Common context stays plain; changed graphemes are emphasized in both versions.
pub(super) fn text_diff_lines(
    before: &str,
    after: &str,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    use unicode_segmentation::UnicodeSegmentation;
    let old: Vec<_> = before.graphemes(true).collect();
    let new: Vec<_> = after.graphemes(true).collect();
    let (old_changed, new_changed) = changed_graphemes(before, after);
    render_versions(&old, &new, &old_changed, &new_changed, theme, width)
}

fn render_versions(
    old: &[&str],
    new: &[&str],
    old_changed: &[bool],
    new_changed: &[bool],
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    append_text_version(
        &mut lines,
        "- ",
        old,
        old_changed,
        theme.error(),
        theme,
        width,
    );
    append_text_version(
        &mut lines,
        "+ ",
        new,
        new_changed,
        theme.status("approved"),
        theme,
        width,
    );
    lines
}

struct DiffStyle<'a> {
    label: &'static str,
    style: ratatui::style::Style,
    theme: &'a Theme,
    width: usize,
}

struct DiffLine {
    spans: Vec<Span<'static>>,
    used: usize,
}

fn append_text_version(
    lines: &mut Vec<Line<'static>>,
    label: &'static str,
    graphemes: &[&str],
    changes: &[bool],
    style: ratatui::style::Style,
    theme: &Theme,
    width: usize,
) {
    if graphemes.is_empty() {
        return;
    }
    let render = DiffStyle {
        label,
        style,
        theme,
        width,
    };
    let mut line = DiffLine {
        spans: vec![Span::styled(label, style)],
        used: 2,
    };
    for (index, grapheme) in graphemes.iter().enumerate() {
        append_grapheme(lines, &mut line, grapheme, changes[index], &render);
    }
    lines.push(Line::from(line.spans));
}

fn append_grapheme(
    lines: &mut Vec<Line<'static>>,
    line: &mut DiffLine,
    grapheme: &&str,
    changed: bool,
    render: &DiffStyle<'_>,
) {
    let size = unicode_width::UnicodeWidthStr::width(*grapheme);
    if *grapheme == "\n" || (line.used + size > render.width.max(3) && line.used > 2) {
        lines.push(Line::from(std::mem::take(&mut line.spans)));
        line.spans.push(Span::styled(render.label, render.style));
        line.used = 2;
        if *grapheme == "\n" {
            return;
        }
    }
    let style = if changed {
        render
            .style
            .add_modifier(ratatui::style::Modifier::BOLD | ratatui::style::Modifier::REVERSED)
    } else {
        render.theme.dim()
    };
    line.spans.push(Span::styled((*grapheme).to_owned(), style));
    line.used += size;
}

#[cfg(test)]
mod diff_tests {
    use super::*;
    use crate::lean::ledger::LedgerChange;
    use crate::lean::tui::ledger::{Action, LedgerView};
    use crate::lean::tui::{InputBuffer, Theme};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::style::Modifier;
    fn review() -> LedgerView {
        let mut view = LedgerView::new();
        view.preview = Some(serde_json::json!({"proposal_id":"p1","diff_hash":"hash"}));
        view.proposed_change = Some(LedgerChange {
            conversation: "c1".into(),
            run_id: "r1".into(),
            expected_spec_revision: 1,
            operation_id: "op".into(),
            reason: "fix".into(),
            entries: vec![],
        });
        view
    }
    #[test]
    fn rejection_comment_survives_errors_and_escape() {
        let mut view = review();
        view.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE), "c1");
        assert!(view.rejecting);
        assert!(matches!(
            view.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL), "c1"),
            Action::None
        ));
        view.paste("Keep 👩‍💻 é");
        view.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), "c1");
        assert!(view.preview.is_some());
        view.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE), "c1");
        assert!(matches!(
            view.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL), "c1"),
            Action::Reject { .. }
        ));
        view.pending = true;
        assert!(matches!(
            view.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL), "c1"),
            Action::None
        ));
        view.reply("rejected", Err("socket closed".into()));
        assert!(view.visible && view.preview.is_some() && view.rejecting);
        assert_eq!(view.rejection.text, "Keep 👩‍💻 é");
        assert_eq!(view.error.as_deref(), Some("socket closed"));
        view.stale = true;
        assert!(matches!(
            view.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL), "c1"),
            Action::None
        ));
        view.reply("rejected", Ok(serde_json::json!({})));
        assert!(!view.visible);
    }
    #[test]
    fn comparison_marks_separate_edits_and_single_graphemes() {
        let (old, new) = changed_graphemes("red keep cat", "bed keep cap");
        assert_eq!(old.iter().filter(|x| **x).count(), 2);
        assert_eq!(new.iter().filter(|x| **x).count(), 2);
        let (old, new) = changed_graphemes("👩‍💻 é", "👩‍🔬 è");
        assert_eq!(old, vec![true, false, true]);
        assert_eq!(new, vec![true, false, true]);
        let (old, new) = changed_graphemes("keep tail", "keep new tail");
        assert!(!old.iter().any(|x| *x));
        assert_eq!(new.iter().filter(|x| **x).count(), 4);
    }
    #[test]
    fn criterion_diff_keeps_other_criteria_out_of_changed_text() {
        let old = serde_json::json!([{"id":"a","text":"old"},{"id":"b","text":"unchanged"}]);
        let new = serde_json::json!([{"id":"a","text":"new"},{"id":"b","text":"unchanged"}]);
        let lines = list_diff_lines(Some(&old), &new, &Theme::from_env(), 10);
        let text: String = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect();
        assert!(!text.contains("unchanged"));
        assert!(text.contains("old") && text.contains("new"));
    }
    #[test]
    fn closing_a_reload_preserves_the_comment_and_invalidates_its_response() {
        let mut view = review();
        view.rejection = InputBuffer::with_text("Keep my correction");
        view.pending = true;
        view.pending_read = true;
        let request = view.request_id;
        view.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), "c1");
        assert!(!view.visible && !view.pending);
        assert_ne!(view.request_id, request);
        assert_eq!(view.rejection_drafts["p1"].text, "Keep my correction");
    }
    #[test]
    fn comparison_keeps_prefixes_and_emphasis_without_color() {
        let theme = Theme::new(false, true);
        for (before, after, prefix) in [("", "added 👩‍💻 é", "+ "), ("removed", "", "- ")]
        {
            let lines = text_diff_lines(before, after, &theme, 12);
            assert!(lines.iter().all(|line| line.width() <= 12));
            assert!(lines.iter().all(|line| line.spans[0].content == prefix));
            for span in lines.iter().flat_map(|line| &line.spans) {
                assert!(span.style.fg.is_none() && span.style.bg.is_none());
            }
            assert!(lines.iter().flat_map(|line| &line.spans).any(|span| {
                span.style
                    .add_modifier
                    .contains(Modifier::BOLD | Modifier::REVERSED)
            }));
        }
        let mut pairs = Vec::new();
        lcs_pairs(
            &["start", "a", "end"],
            &["start", "b", "end"],
            0,
            0,
            &mut pairs,
        );
        assert_eq!(pairs, vec![(0, 0), (2, 2)]);
    }
    #[test]
    fn large_comparison_bounds_work() {
        let a = vec!["a"; 1100];
        let b = vec!["b"; 1100];
        let mut out = vec![];
        lcs_pairs(&a, &b, 0, 0, &mut out);
        assert!(out.is_empty());
    }
    #[test]
    fn comparison_emphasizes_changes_without_splitting_graphemes() {
        let lines = text_diff_lines("User 👩‍💻 old é", "User 👩‍💻 new é", &Theme::from_env(), 12);
        let rendered: String = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect();
        assert!(rendered.contains("👩‍💻"));
        assert!(rendered.contains("é"));
        let changed: String = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .filter(|span| span.style.add_modifier.contains(Modifier::REVERSED))
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(changed, "oldnew");
        assert!(rendered.contains("- "));
        assert!(rendered.contains("+ "));
    }
}
