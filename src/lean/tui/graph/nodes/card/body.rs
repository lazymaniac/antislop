use super::*;

impl Card<'_> {
    pub(super) fn paint_body(&mut self) {
        let inner = self.inner_width();
        let (summary_max, effect_max) = self.node.body_lines();
        self.paint_summary(inner, summary_max);
        self.paint_effect(inner, summary_max, effect_max);
    }

    fn paint_summary(&mut self, inner: usize, max: usize) {
        for (row, line) in summary_lines(self.view, &self.node.key, self.task, inner, max)
            .iter()
            .enumerate()
        {
            let y = self.node.y + BODY_ROW + row as u16;
            self.canvas
                .text(self.node.x + 2, y, line, self.fill.patch(self.theme.text()));
        }
    }

    fn paint_effect(&mut self, inner: usize, summary_max: usize, effect_max: usize) {
        for (row, line) in effect_lines(self.view, &self.node.key, inner, effect_max)
            .iter()
            .enumerate()
        {
            let y = self.node.y + BODY_ROW + summary_max as u16 + row as u16;
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
            self.fill.patch(self.theme.faint()),
        );
    }
}
