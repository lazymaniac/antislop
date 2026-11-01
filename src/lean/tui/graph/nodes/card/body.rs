use super::super::super::card_content::{Content, Tone};
use super::*;

impl Card<'_> {
    pub(super) fn paint_body(&mut self) {
        let content = Content::new(
            self.view,
            &self.node.key,
            self.node.width,
            self.node.height,
            self.node.collapsed,
        );
        let (_, status) = super::content::status_style(self.theme, self.health);
        for (row, (tone, text)) in content.rows.iter().enumerate() {
            let style = match tone {
                Tone::Text => self.theme.text(),
                Tone::Dim => self.theme.dim(),
                Tone::Status => status,
                Tone::Activity => match self.health {
                    Health::Problem => self.theme.error(),
                    Health::Waiting => self.theme.warning(),
                    _ => self.theme.dim(),
                },
            };
            if row + 2 >= usize::from(self.node.height) {
                break;
            }
            self.canvas.text(
                self.node.x + 2,
                self.node.y + 2 + row as u16,
                &truncate(text, self.inner_width()),
                self.fill.patch(style),
            );
        }
    }
}
