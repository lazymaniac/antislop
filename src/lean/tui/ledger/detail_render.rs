#[path = "detail_render/content.rs"]
mod content;
use super::*;
use crate::lean::tui::Theme;
use content::{
    append_evidence, append_linked_tasks, criteria_lines, history_entries, overview_lines,
    push_section, section_lines, task_owner_lines,
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
            0 => overview_lines(entry, theme, width),
            1 => criteria_lines(entry, theme, width),
            2 => self.task_lines(entry, theme, width),
            3 => self.evidence_lines(theme, width),
            _ => self.history_lines(theme, width),
        }
    }

    fn task_lines(
        &self,
        entry: &crate::lean::ledger::LedgerEntry,
        theme: &Theme,
        width: usize,
    ) -> Vec<Line<'static>> {
        let mut lines = task_owner_lines(entry, theme, width);
        let Some(tasks) = self
            .detail
            .as_ref()
            .and_then(|value| value["tasks"].as_array())
        else {
            push_section(
                &mut lines,
                "Linked tasks",
                "Press Enter to load task coverage.",
                theme,
                width,
            );
            return lines;
        };
        append_linked_tasks(&mut lines, tasks, theme, width);
        lines
    }

    fn evidence_lines(&self, theme: &Theme, width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let Some(evidence) = self
            .detail
            .as_ref()
            .and_then(|value| value["evidence"].as_array())
        else {
            push_section(
                &mut lines,
                "Verification evidence",
                "Press Enter to load independent checks.",
                theme,
                width,
            );
            return lines;
        };
        if evidence.is_empty() {
            push_section(
                &mut lines,
                "Verification evidence",
                "No independent evidence yet. Approval of requirements does not verify implementation.",
                theme,
                width,
            );
        }
        for item in evidence {
            append_evidence(&mut lines, item, theme, width);
        }
        lines
    }

    fn history_lines(&self, theme: &Theme, width: usize) -> Vec<Line<'static>> {
        let Some(history) = self
            .detail
            .as_ref()
            .and_then(|value| value["history"].as_array())
        else {
            return section_lines(
                "History",
                "Press Enter to load immutable versions.",
                theme,
                width,
            );
        };
        let mut lines = history_entries(history, theme, width);
        if history.is_empty() {
            push_section(&mut lines, "History", "No earlier versions.", theme, width);
        }
        lines
    }
}
