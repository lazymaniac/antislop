use super::*;

impl Card<'_> {
    pub(super) fn paint_body(&mut self, top: u16) {
        let inner = self.inner_width();
        if is_admin(&self.node.key) {
            let rows = self.node.height.saturating_sub(ADMIN_BODY_ROW);
            let branch = self.branch().map(|branch| (branch, rows.saturating_sub(1)));
            let lines = branch.as_ref().map_or(rows, |(_, row)| *row);
            self.paint_summary(inner, ADMIN_BODY_ROW, usize::from(lines));
            if let Some((branch, row)) = branch.filter(|(_, row)| *row < rows) {
                self.paint_branch_at(ADMIN_BODY_ROW + row, &branch);
            }
            return;
        }
        let (summary_max, effect_max) = self.node.body_lines();
        // Empty optional header rows belong to the description. The effect
        // and metadata keep their anchors, so the card's geometry stays fixed.
        let summary_max = summary_max + usize::from(BODY_ROW.saturating_sub(top));
        self.paint_summary(inner, top, summary_max);
        self.paint_effect(inner, top, summary_max, effect_max);
    }

    fn paint_summary(&mut self, inner: usize, top: u16, max: usize) {
        for (row, line) in summary_lines(self.view, &self.node.key, self.task, inner, max)
            .iter()
            .enumerate()
        {
            let y = self.node.y + top + row as u16;
            self.canvas
                .text(self.node.x + 2, y, line, self.fill.patch(self.theme.text()));
        }
    }

    fn paint_effect(&mut self, inner: usize, top: u16, summary_max: usize, effect_max: usize) {
        for (row, line) in effect_lines(self.view, &self.node.key, inner, effect_max)
            .iter()
            .enumerate()
        {
            let y = self.node.y + top + summary_max as u16 + row as u16;
            self.canvas
                .text(self.node.x + 2, y, line, self.fill.patch(self.theme.text()));
        }
    }

    pub(super) fn paint_metadata(&mut self) {
        let Some(task) = self.task else { return };
        let role = self.node.key.split_once(':').map_or("", |(role, _)| role);
        let meta = taskinfo::card_meta(role, self.view, task, self.inner_width());
        if meta.is_empty() {
            return;
        }
        let bottom = self
            .node
            .y
            .saturating_add(self.node.height)
            .saturating_sub(1);
        self.canvas.text(
            self.node.x + 2,
            bottom,
            &truncate(&meta, self.inner_width()),
            self.fill.patch(self.theme.text()),
        );
    }
}
