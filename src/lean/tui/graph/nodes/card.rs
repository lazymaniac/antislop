mod body;
mod content;
mod identity;

use super::super::super::view::TaskRow;
use super::super::*;

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
        self.paint_progress(tick);
        self.paint_identity();
        self.paint_contents();
    }

    fn paint_contents(&mut self) {
        self.paint_body();
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

    fn paint_progress(&mut self, tick: u64) {
        // Idle also covers agents whose turn or task has already ended.
        if self.health == Health::Idle {
            let finished = match self.task {
                Some(task) => matches!(task.status.as_str(), "skipped" | "cancelled" | "revised"),
                None => {
                    current_round(self.view, &card_members(self.view, &self.node.key)).is_some()
                }
            };
            if finished {
                return;
            }
        }
        let last = self.node.height.saturating_sub(1);
        let row = match self.health {
            Health::Working if last > 0 => {
                let phase = tick % (u64::from(last) * 2);
                phase.min(u64::from(last) * 2 - phase) as u16
            }
            Health::Working | Health::Idle => last / 2,
            _ => return,
        };
        let marker = if self.theme.ascii { '#' } else { '█' };
        self.canvas.put(
            self.node.x,
            self.node.y.saturating_add(row),
            marker,
            self.fill.patch(self.theme.progress_marker()),
        );
    }

    fn inner_width(&self) -> usize {
        usize::from(self.node.width.saturating_sub(4))
    }
}
