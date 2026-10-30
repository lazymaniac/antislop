mod body;
mod content;

use super::super::super::view::TaskRow;
use super::super::health::{activity_line, describe, effect_lines, provider_of};
use super::super::*;
use content::{card_title, status_style, summary_lines};

struct Card<'a> {
    view: &'a View,
    canvas: &'a mut Canvas,
    node: &'a Node,
    theme: &'a Theme,
    task: Option<&'a TaskRow>,
    role: Option<Role>,
    health: Health,
    fill: Style,
    accent: Style,
}

pub(super) fn draw(
    view: &View,
    canvas: &mut Canvas,
    node: &Node,
    selected: bool,
    theme: &Theme,
    tick: u64,
) {
    if outside_viewport(canvas, node) {
        return;
    }
    Card::new(view, canvas, node, selected, theme).paint(selected, tick);
}

fn outside_viewport(canvas: &Canvas, node: &Node) -> bool {
    node.x >= canvas.origin_x.saturating_add(canvas.width)
        || node.x.saturating_add(node.width) <= canvas.origin_x
        || node.y >= canvas.origin_y.saturating_add(canvas.height)
        || node.y.saturating_add(node.height) <= canvas.origin_y
}

impl Card<'_> {
    fn new<'a>(
        view: &'a View,
        canvas: &'a mut Canvas,
        node: &'a Node,
        selected: bool,
        theme: &'a Theme,
    ) -> Card<'a> {
        let card = task_card(view, &node.key);
        let task = card.map(|(task, _)| task);
        Card {
            view,
            canvas,
            node,
            theme,
            task,
            role: card.map(|(_, role)| role),
            health: health(view, &node.key),
            fill: if selected {
                theme.raised()
            } else {
                theme.card()
            },
            accent: theme.role_accent(&node.key, task.map_or("", |task| task.kind.as_str())),
        }
    }

    fn paint(&mut self, selected: bool, tick: u64) {
        self.paint_background(selected);
        self.paint_identity(tick);
        if !self.node.collapsed {
            self.paint_contents();
        }
    }

    fn paint_contents(&mut self) {
        if is_admin(&self.node.key) {
            self.paint_body();
            return;
        }
        self.paint_activity();
        self.paint_branch();
        self.paint_body();
        self.paint_metadata();
    }

    fn paint_background(&mut self, selected: bool) {
        let right = self.node.x + self.node.width - 1;
        let bottom = self
            .node
            .y
            .saturating_add(self.node.height)
            .saturating_sub(1);
        for y in self.node.y..=bottom {
            for x in self.node.x..=right {
                self.canvas.put(x, y, ' ', self.fill);
            }
            let bar = if selected { '█' } else { '▌' };
            self.canvas
                .put(self.node.x, y, bar, self.fill.patch(self.accent));
        }
    }

    fn paint_identity(&mut self, tick: u64) {
        let (icon, tone) = status_style(self.theme, self.health, tick);
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
        // The frame carries the task's title; a card names its agent.
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
        let description = describe(self.view, &self.node.key);
        self.canvas.text(
            x + 4,
            y + 2,
            &truncate(&description, inner.saturating_sub(2)),
            self.fill.patch(if self.health == Health::Idle {
                self.theme.dim()
            } else {
                tone
            }),
        );
    }

    /// A shared options card keeps the verifier's state on a line of its
    /// own; a verifier's card shows the coverage it has settled.
    fn paint_verifier(&mut self, inner: usize, tone: Style) {
        if let Some(task) = self.task {
            let coverage = task
                .obligations
                .as_ref()
                .map(|_| taskinfo::coverage_short(task));
            let text = match (self.role, coverage) {
                (Some(Role::Pair), coverage) => format!(
                    "V: {}{}",
                    taskinfo::verifier_status(task),
                    coverage.map_or_default(|coverage| format!(" · {coverage}"))
                ),
                (Some(Role::Verifier), Some(coverage)) => format!("coverage {coverage}"),
                _ => return,
            };
            self.canvas.text(
                self.node.x + 2,
                self.node.y + 3,
                &truncate(&text, inner),
                self.fill.patch(tone),
            );
        }
    }

    fn paint_activity(&mut self) {
        if let Some(line) = activity_line(self.view, &self.node.key) {
            let style = match self.health {
                Health::Problem => self.theme.error(),
                Health::Waiting => self.theme.warning(),
                _ => self.theme.dim(),
            };
            self.canvas.text(
                self.node.x + 2,
                self.node.y + ACTIVITY_ROW,
                &truncate(&line, self.inner_width()),
                self.fill.patch(style),
            );
        }
    }

    fn paint_branch(&mut self) {
        if let Some(branch) = self.branch() {
            self.paint_branch_at(BRANCH_ROW, &branch);
        }
    }

    fn branch(&self) -> Option<String> {
        let branch_key = branch_key(self.view, self.node, self.task, self.role);
        self.view
            .agents
            .get(&branch_key)
            .and_then(|agent| agent.branch.clone())
    }

    fn paint_branch_at(&mut self, row: u16, branch: &str) {
        self.canvas.text(
            self.node.x + 2,
            self.node.y + row,
            &truncate(&format!("branch: {branch}"), self.inner_width()),
            self.fill.patch(self.theme.dim()),
        );
    }

    fn inner_width(&self) -> usize {
        usize::from(self.node.width - 4)
    }
}

fn branch_key(view: &View, node: &Node, task: Option<&TaskRow>, role: Option<Role>) -> String {
    if role != Some(Role::Pair) && task.is_some() {
        return node.key.clone();
    }
    if let Some(task) = task {
        format!(
            "{}:{}",
            if task.status == "verifying" {
                "verifier"
            } else {
                "worker"
            },
            task.id
        )
    } else {
        let members = card_members(view, &node.key);
        current_round(view, &members)
            .unwrap_or(&node.key)
            .to_owned()
    }
}
