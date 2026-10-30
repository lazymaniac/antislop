use super::*;

impl Details {
    pub fn lines(&mut self, view: &View) -> Vec<(Style, String)> {
        let records = self.records(view);
        Self::render_records(&records, &self.query)
    }

    /// Compact header; source prompts remain in the session document.
    pub(in crate::lean::tui) fn summary(&self, view: &View, theme: &Theme) -> Vec<Line<'static>> {
        self.hero(view, theme)
    }

    /// Both agents of this task, in the order they work: worker, verifier.
    pub(in crate::lean::tui) fn role_keys(&self) -> Vec<String> {
        match self.key.split_once(':') {
            Some(("worker" | "verifier", id)) => {
                vec![format!("worker:{id}"), format!("verifier:{id}")]
            }
            _ => vec![self.key.clone()],
        }
    }

    pub fn overview(&self) -> bool {
        is_overview(self.tab)
    }

    /// The terminal shows unless the info page took its place.
    pub fn terminal_visible(&self) -> bool {
        self.overview() && (!self.show_info || self.split)
    }

    /// The one agent the Overview in front shows: the worker's tab holds the
    /// worker, the verifier's tab the verifier.
    pub(in crate::lean::tui) fn session_keys(&self) -> Vec<String> {
        match self.key.split_once(':') {
            Some(("worker" | "verifier", id)) => {
                vec![if self.tab == VERIFIER {
                    format!("verifier:{id}")
                } else {
                    format!("worker:{id}")
                }]
            }
            _ => vec![self.key.clone()],
        }
    }

    fn render_records(records: &[Rc<DetailRecord>], query: &str) -> Vec<(Style, String)> {
        let mut out = vec![];
        let query_lower = query.trim().to_lowercase();
        if !query.is_empty() {
            out.push((
                Style::default()
                    .fg(super::theme::ayu::BLUE)
                    .add_modifier(Modifier::BOLD),
                format!("🔍 Record filter: \"{}\"  (Esc: clear)", query),
            ));
            out.push((Style::default(), String::new()));
        }
        let mut matched_records = 0;
        for rec in records {
            let is_match =
                query_lower.is_empty() || rec.search_text.to_lowercase().contains(&query_lower);
            if is_match {
                matched_records += 1;
                out.extend(rec.lines.iter().cloned());
            }
        }
        if !query_lower.is_empty() && matched_records == 0 {
            out.push((dim(), format!("No records matching query \"{}\".", query)));
        }
        out
    }
}
