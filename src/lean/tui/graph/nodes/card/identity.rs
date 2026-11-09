use super::super::super::health::describe;
use super::content::{card_title, status_style};
use super::*;

impl Card<'_> {
    pub(super) fn paint_identity(&mut self) {
        if self.expanded_tab.is_some() {
            // An open card's title row (task, status, round) is drawn over
            // the canvas with the round shown; only its strip and the icon
            // belong here.
            let right = self.node.x + self.node.width.saturating_sub(1);
            let header = self.fill.patch(self.theme.card_header());
            for x in self.node.x + 1..right {
                self.canvas.put(x, self.node.y, ' ', header);
            }
            self.canvas.text(
                self.node.x + 2,
                self.node.y,
                self.theme.role_icon(&self.node.key),
                header.patch(self.accent),
            );
            self.paint_open_comments();
            return;
        }
        let (icon, tone) = status_style(self.theme, self.health);
        self.paint_title();
        self.paint_open_comments();
        self.paint_status(self.inner_width(), &icon, tone);
    }

    fn paint_icon(&mut self) {
        self.canvas.text(
            self.node.x + 2,
            self.node.y,
            self.theme.role_icon(&self.node.key),
            self.fill.patch(self.accent),
        );
    }

    fn paint_title(&mut self) {
        let inner = self.inner_width();
        let title_style = match self.health {
            Health::Problem => self.theme.error(),
            Health::Waiting => self.theme.warning(),
            _ => self.accent,
        }
        .add_modifier(Modifier::BOLD);
        let name = match self.role {
            Some(Role::Worker) => "worker".to_owned(),
            Some(Role::Reviewer) => "reviewer".to_owned(),
            Some(Role::Pair) => "options".to_owned(),
            None => node_name(&self.node.key),
        };
        // The work kind goes after the role while at least a few of its
        // letters fit.
        let title = match (self.role, self.task) {
            (Some(Role::Worker), Some(task)) if !task.kind.is_empty() && task.kind != name => {
                let full = format!("{name} - {}", task.kind);
                let title = card_title(&full, inner, self.node.collapsed);
                let shown = title.split_once(" - ").map_or(0, |(_, kind)| {
                    kind.split('…').next().unwrap_or("").chars().count()
                });
                if title.contains('…') && shown < 4 {
                    card_title(&name, inner, self.node.collapsed)
                } else {
                    title
                }
            }
            _ => card_title(&name, inner, self.node.collapsed),
        };
        let (x, y) = (self.node.x, self.node.y);
        self.paint_icon();
        self.canvas.text(
            x + 4,
            y,
            &truncate(&title, inner.saturating_sub(2)),
            self.fill.patch(title_style),
        );
    }

    fn paint_open_comments(&mut self) {
        let open = self
            .view
            .comments_on(&self.node.key)
            .iter()
            .filter(|comment| comment.status == "open")
            .count();
        if open > 0 {
            let base = if self.expanded_tab.is_some() {
                self.fill.patch(self.theme.card_header())
            } else {
                self.fill
            };
            let right = self.node.x + self.node.width - 1;
            self.canvas.text(
                right - 3,
                self.node.y,
                &format!("✎{}", open.min(9)),
                base.patch(self.theme.accent()),
            );
        }
    }

    fn paint_status(&mut self, inner: usize, icon: &str, tone: Style) {
        let (x, y) = (self.node.x, self.node.y);
        self.canvas.text(x + 2, y + 1, icon, self.fill.patch(tone));
        let description = self.status_description();
        let icon_width = if icon.is_empty() { 0 } else { 2 };
        let style = if self.health == Health::Idle {
            self.theme.dim()
        } else {
            tone
        };
        self.canvas.text(
            x + 2 + icon_width,
            y + 1,
            &truncate(&description, inner.saturating_sub(usize::from(icon_width))),
            self.fill.patch(style),
        );
    }

    fn status_description(&self) -> String {
        if self.health != Health::Working {
            let mut description = describe(self.view, &self.node.key);
            if self.pair_finished()
                && let Some(task) = self.task.filter(|task| task.obligations.is_some())
            {
                description.push_str(&format!(" · {}", taskinfo::coverage_short(task)));
            }
            return description;
        }
        if requirements_round(&self.node.key).is_some() {
            return describe(self.view, &self.node.key);
        }
        self.task
            .map(|task| task.round)
            .filter(|round| *round > 0)
            .map_or_else(|| "working".into(), |round| format!("round {round}"))
    }

    fn pair_finished(&self) -> bool {
        self.task
            .zip(self.role)
            .is_some_and(|(task, role)| card_content::pair_finished(task, role))
    }
}
