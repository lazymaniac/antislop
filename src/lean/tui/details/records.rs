use super::*;
mod checks;
mod comments;
mod mailbox;
mod messages;
mod overview;
mod plan;
mod requirements;
mod results;
mod review;

impl Details {
    pub(in crate::lean::tui) fn records(&mut self, view: &View) -> Vec<Rc<DetailRecord>> {
        let mut records = Vec::new();
        match self.tab {
            PLAN => plan::plan(self, view, &mut records),
            REQUIREMENTS => requirements::requirements(self, view, &mut records),
            0 => overview::overview(self, view, &mut records),
            1 => messages::messages(self, view, &mut records),
            REVIEW => review::review(self, view, &mut records),
            COMMENTS => comments::comments(self, view, &mut records),
            CHECKS => checks::checks(self, view, &mut records),
            _ => results::results(self, view, &mut records),
        }
        records
    }
}

fn record(lines: Vec<(Style, String)>, separator: &str) -> Rc<DetailRecord> {
    let search_text = lines
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join(separator);
    Rc::new(DetailRecord { lines, search_text })
}

fn plain(text: String) -> (Style, String) {
    (Style::default(), text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_search_text_joins_lines_with_the_screen_separator() {
        let lines = vec![plain("first".into()), plain("second".into())];
        assert_eq!(record(lines.clone(), " ").search_text, "first second");
        assert_eq!(record(lines, "\n").search_text, "first\nsecond");
    }
}
