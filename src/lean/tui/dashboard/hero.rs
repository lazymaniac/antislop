use super::*;
mod identity;
mod metrics;

impl Details {
    /// Two lines that stay above both faces of the Overview.
    pub(in crate::lean::tui) fn hero(&self, view: &View, theme: &Theme) -> Vec<Line<'static>> {
        let key = self.hero_key();
        let mut chips = self.provider_chip(view, theme, &key);
        chips.extend(self.check_chips(theme));
        chips.extend(self.activity_chips(view, theme));
        vec![
            self.identity_line(view, theme, &key),
            join_chips(theme, chips),
            Line::raw(""),
        ]
    }
}
