use super::super::super::health::describe;
use super::content::{card_title, status_style};
use super::*;

impl Card<'_> {
    pub(super) fn paint_identity(&mut self) {
        let (icon, tone) = status_style(self.theme, self.health);
        self.paint_title();
        self.paint_open_comments();
        self.paint_status(self.inner_width(), &icon, tone);
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
            Some(Role::Verifier) => "verifier".to_owned(),
            Some(Role::Pair) => "options".to_owned(),
            None => node_name(&self.node.key),
        };
        let name = match (self.role, self.task) {
            (Some(Role::Worker), Some(task)) if !task.kind.is_empty() && task.kind != name => {
                format!("{name} - {}", task.kind)
            }
            _ => name,
        };
        let title = card_title(&name, inner, self.node.collapsed);
        let (x, y) = (self.node.x, self.node.y);
        self.canvas.text(
            x + 2,
            y,
            self.theme.role_icon(&self.node.key),
            self.fill.patch(self.accent),
        );
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
            let right = self.node.x + self.node.width - 1;
            self.canvas.text(
                right - 3,
                self.node.y,
                &format!("✎{}", open.min(9)),
                self.fill.patch(self.theme.accent()),
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
        let round = self
            .task
            .map(|task| task.round)
            .filter(|round| *round > 0)
            .or_else(|| {
                let members = card_members(self.view, &self.node.key);
                current_round(self.view, &members)
                    .and_then(requirements_round)
                    .map(|index| index as u64 + 1)
            })
            .or_else(|| {
                self.view
                    .agents
                    .get(&self.node.key)
                    .map(|agent| u64::from(agent.turns) + 1)
            });
        round.map_or_default(|round| format!("round {round}"))
    }

    fn pair_finished(&self) -> bool {
        self.task
            .zip(self.role)
            .is_some_and(|(task, role)| card_content::pair_finished(task, role))
    }
}
