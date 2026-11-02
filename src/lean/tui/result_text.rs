//! Human-readable worker reports, including structured options deliverables.
use crate::lean::turn_result::{Selection, TurnResult};

mod agent;
pub(super) use agent::agent_result;

const SELECTION_HEADER: &str =
    "Recorded structured selection (accepted by the runtime; this is the deliverable):\n```json\n";

pub(super) fn worker_report(raw: &str) -> String {
    if let Ok(TurnResult::Completed {
        summary,
        report,
        selection,
    }) = serde_json::from_str(raw)
    {
        let mut text = String::new();
        if !summary.trim().is_empty() && summary.trim() != report.trim() {
            text.push_str(&format!("Summary\n{summary}\n\n"));
        }
        if selection
            .as_ref()
            .is_none_or(|selection| report.trim() != selection.rationale.trim())
        {
            text.push_str(&report);
        }
        if let Some(selection) = selection {
            append_selection(&mut text, &selection);
        }
        return text;
    }
    // Completed rounds historically stored the selection as a fenced suffix.
    if let Some((report, suffix)) = raw.rsplit_once(SELECTION_HEADER)
        && let Some(json) = suffix.trim_end().strip_suffix("```")
        && let Ok(selection) = serde_json::from_str::<Selection>(json.trim())
    {
        let mut text = report.trim_end().to_owned();
        append_selection(&mut text, &selection);
        return text;
    }
    raw.to_owned()
}

/// Streams prose to the disk-backed document, retaining only a possible
/// structured result or selection suffix for decoding.
#[derive(Default)]
pub(super) struct ReportStream {
    pending: String,
    structured: Option<bool>,
    selection: bool,
}

impl ReportStream {
    pub(super) fn push(
        &mut self,
        chunk: &str,
        sink: &mut dyn FnMut(&str) -> crate::Result<()>,
    ) -> crate::Result<()> {
        use unicode_segmentation::UnicodeSegmentation;
        self.pending.push_str(chunk);
        if self.structured.is_none() && !self.pending.trim_start().is_empty() {
            self.structured = Some(self.pending.trim_start().starts_with('{'));
        }
        if self.structured != Some(false) || self.selection {
            return Ok(());
        }
        if let Some(start) = self.pending.find(SELECTION_HEADER) {
            sink(&self.pending[..start])?;
            self.pending.drain(..start);
            self.selection = true;
        } else {
            // Keep enough text for a marker split across history chunks, and
            // never divide a grapheme while moving prose to the document.
            let limit = self.pending.len().saturating_sub(SELECTION_HEADER.len());
            let end = self
                .pending
                .grapheme_indices(true)
                .map(|(offset, _)| offset)
                .take_while(|offset| *offset <= limit)
                .last()
                .unwrap_or(0);
            sink(&self.pending[..end])?;
            self.pending.drain(..end);
        }
        Ok(())
    }

    pub(super) fn finish(
        self,
        sink: &mut dyn FnMut(&str) -> crate::Result<()>,
    ) -> crate::Result<()> {
        self.finish_with(worker_report, sink)
    }

    pub(super) fn finish_with(
        self,
        format: fn(&str) -> String,
        sink: &mut dyn FnMut(&str) -> crate::Result<()>,
    ) -> crate::Result<()> {
        sink(&format(&self.pending))
    }
}

fn append_selection(text: &mut String, selection: &Selection) {
    if text.trim() == selection.rationale.trim() {
        text.clear();
    }
    text.push_str(&format!("\n\nSelected option: {}\n", selection.selected));
    if let Some(chosen) = selection.chosen() {
        text.push_str(&chosen.approach);
        text.push('\n');
    }
    text.push_str(&format!(
        "\nRationale\n{}\n\nAlternatives",
        selection.rationale
    ));
    for alternative in &selection.alternatives {
        let selected = if alternative.id == selection.selected {
            " (selected)"
        } else {
            ""
        };
        text.push_str(&format!(
            "\n\nOption {}{selected}\n{}\n\nTradeoffs\n{}",
            alternative.id, alternative.approach, alternative.tradeoffs
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_both_stored_options_forms_without_losing_the_deliverable() {
        let selection: Selection = serde_json::from_value(json!({
            "selected":"B", "rationale":"Keeps café 👩‍👩‍👧‍👦 intact.",
            "alternatives":[
                {"id":"A", "approach":"Replace the store", "tradeoffs":"Migration required"},
                {"id":"B", "approach":"Reuse the store", "tradeoffs":"Existing constraints"}
            ]
        }))
        .unwrap();
        let legacy = crate::lean::turn_result::report_with_deliverables("Report", Some(&selection));
        let structured = json!({"type":"completed","summary":"Summary text","report":"Report","selection":selection}).to_string();
        for raw in [legacy, structured] {
            let text = worker_report(&raw);
            for expected in [
                "Report",
                "Selected option: B",
                "Option A",
                "Option B (selected)",
                "Migration required",
                "Existing constraints",
                "café 👩‍👩‍👧‍👦",
            ] {
                assert!(text.contains(expected), "{text}");
            }
            assert!(
                !text.contains("```json") && !text.contains("\"alternatives\""),
                "{text}"
            );
        }
    }

    #[test]
    fn history_chunks_preserve_unicode_and_decode_split_selection_markers() {
        use unicode_segmentation::UnicodeSegmentation;
        let selection = Selection {
            selected: "A".into(),
            rationale: "Why A".into(),
            alternatives: vec![crate::lean::turn_result::Alternative {
                id: "A".into(),
                approach: "Approach A".into(),
                tradeoffs: "Cost A".into(),
            }],
        };
        let prose = "café 👩‍👩‍👧‍👦\n\n- Item\n".repeat(100);
        let legacy = crate::lean::turn_result::report_with_deliverables(&prose, Some(&selection));
        let structured = serde_json::json!({"type":"completed","summary":"Done","report":prose,"selection":selection}).to_string();
        for raw in [prose.clone(), legacy, structured] {
            let mut formatter = ReportStream::default();
            let mut output = String::new();
            let mut sink = |chunk: &str| {
                output.push_str(chunk);
                Ok(())
            };
            // Even a ZWJ sequence may arrive across transport chunks.
            for chunk in raw.split_inclusive(|_: char| true) {
                formatter.push(chunk, &mut sink).unwrap();
            }
            formatter.finish(&mut sink).unwrap();
            assert!(output.contains(&prose), "prose was altered");
            assert!(!output.contains(SELECTION_HEADER));
            assert!(output.graphemes(true).any(|glyph| glyph == "👩‍👩‍👧‍👦"));
        }
    }

    #[test]
    fn plain_reports_and_unrelated_code_blocks_are_preserved() {
        let raw = "# Findings\n\n- First\n- Second\n\n```json\n{\"example\":true}\n```";
        assert_eq!(worker_report(raw), raw);
        assert_eq!(worker_report("{invalid"), "{invalid");
    }
}
