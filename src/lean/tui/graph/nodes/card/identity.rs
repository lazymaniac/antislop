use super::super::super::health::{describe, provider_of};
use super::content::{card_title, status_style};
use super::*;

impl Card<'_> {
    pub(super) fn paint_identity(&mut self) {
        let (icon, tone) = status_style(self.theme, self.health);
        self.paint_title();
        self.paint_open_comments();
        self.paint_provider(self.inner_width());
        self.paint_status(self.inner_width(), &icon, tone);
        self.paint_verifier(self.inner_width(), tone);
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

    fn paint_provider(&mut self, inner: usize) {
        let kind = self.task.map_or("orchestration", |task| {
            if self.role == Some(Role::Verifier) {
                "verification"
            } else if task.kind.is_empty() {
                "unknown"
            } else {
                task.kind.as_str()
            }
        });
        let provider = provider_of(self.view, &self.node.key).unwrap_or("-");
        self.canvas.text(
            self.node.x + 2,
            self.node.y + 1,
            &truncate(&format!("{provider} - {kind}"), inner),
            self.fill.patch(self.accent),
        );
    }

    fn paint_status(&mut self, inner: usize, icon: &str, tone: Style) {
        let (x, y) = (self.node.x, self.node.y);
        self.canvas.text(x + 2, y + 2, icon, self.fill.patch(tone));
        let description = self.status_description();
        let icon_width = if icon.is_empty() { 0 } else { 2 };
        let style = if self.health == Health::Idle {
            self.theme.dim()
        } else {
            tone
        };
        self.canvas.text(
            x + 2 + icon_width,
            y + 2,
            &truncate(&description, inner.saturating_sub(usize::from(icon_width))),
            self.fill.patch(style),
        );
    }

    fn status_description(&self) -> String {
        if self.health != Health::Working {
            return describe(self.view, &self.node.key);
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

    fn paint_verifier(&mut self, inner: usize, tone: Style) {
        if let Some(text) = self.verifier_text() {
            self.canvas.text(
                self.node.x + 2,
                self.node.y + 3,
                &truncate(&text, inner),
                self.fill.patch(tone),
            );
        }
    }

    pub(super) fn verifier_text(&self) -> Option<String> {
        let task = self.task?;
        let coverage = task
            .obligations
            .as_ref()
            .map(|_| taskinfo::coverage_short(task));
        match (self.role, coverage) {
            (Some(Role::Pair), coverage) => Some(format!(
                "V: {}{}",
                taskinfo::verifier_status(task),
                coverage.map_or_default(|coverage| format!(" · {coverage}"))
            )),
            (Some(Role::Verifier), Some(coverage)) => Some(format!("coverage {coverage}")),
            _ => None,
        }
    }
}
