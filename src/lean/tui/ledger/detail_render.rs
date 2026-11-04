#[path = "detail_render/content.rs"]
mod content;
use super::*;
use crate::lean::tui::Theme;
use content::{
    append_coverage, append_criteria_with_evidence, history_entries, overview_lines, push_section,
    section_lines,
};
use ratatui::text::Line;

impl LedgerView {
    pub(super) fn detail_lines(&self, theme: &Theme, width: usize) -> Vec<Line<'static>> {
        let Some(entry) = self.selected_entry() else {
            return vec![Line::from(
                "Select a record to inspect its requirements and coverage.",
            )];
        };
        if let Some(source) = &self.source {
            return section_lines("Original source", source, theme, width);
        }
        match self.tab {
            0 => self.spec_lines(entry, theme, width),
            _ => self.history_lines(theme, width),
        }
    }

    /// One sheet: intent, criteria with the evidence that settles each one,
    /// and the tasks that implement the requirement.
    fn spec_lines(
        &self,
        entry: &crate::lean::ledger::LedgerEntry,
        theme: &Theme,
        width: usize,
    ) -> Vec<Line<'static>> {
        let mut lines = overview_lines(entry, theme, width);
        let evidence = self
            .detail
            .as_ref()
            .and_then(|value| value["evidence"].as_array())
            .map(Vec::as_slice);
        append_criteria_with_evidence(&mut lines, entry, evidence, theme, width);
        let tasks = self
            .detail
            .as_ref()
            .and_then(|value| value["tasks"].as_array())
            .map(Vec::as_slice);
        append_coverage(&mut lines, entry, tasks, evidence, theme, width);
        lines
    }

    fn history_lines(&self, theme: &Theme, width: usize) -> Vec<Line<'static>> {
        let Some(history) = self
            .detail
            .as_ref()
            .and_then(|value| value["history"].as_array())
        else {
            return section_lines("History", "Loading versions…", theme, width);
        };
        let mut lines = history_entries(history, theme, width);
        if history.is_empty() {
            push_section(&mut lines, "History", "No earlier versions.", theme, width);
        }
        lines
    }
}
