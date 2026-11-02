//! Task-only DAG used by the plan viewer. Geometry scales with task count;
//! the drawing surface is always limited to the visible window.
use super::*;
use std::collections::VecDeque;

pub(in crate::lean::tui) struct PlanGraph {
    pub nodes: Vec<Rect>,
    pub width: u16,
    pub height: u16,
}

impl PlanGraph {
    pub fn new(tasks: &[TaskRow], viewport_width: u16) -> Self {
        let ids: HashMap<_, _> = tasks
            .iter()
            .enumerate()
            .map(|(index, task)| (task.id.as_str(), index))
            .collect();
        let mut pending = vec![0usize; tasks.len()];
        let mut children = vec![Vec::new(); tasks.len()];
        for (index, task) in tasks.iter().enumerate() {
            for dependency in &task.depends_on {
                if let Some(&parent) = ids.get(dependency.as_str()) {
                    pending[index] += 1;
                    children[parent].push(index);
                }
            }
        }
        let mut ready: VecDeque<_> = pending
            .iter()
            .enumerate()
            .filter_map(|(index, count)| (*count == 0).then_some(index))
            .collect();
        let mut levels = vec![0usize; tasks.len()];
        while let Some(parent) = ready.pop_front() {
            for &child in &children[parent] {
                levels[child] = levels[child].max(levels[parent] + 1);
                pending[child] -= 1;
                if pending[child] == 0 {
                    ready.push_back(child);
                }
            }
        }
        let card_width = viewport_width.saturating_sub(2).clamp(6, 30);
        let columns = usize::from(
            viewport_width.saturating_sub(2).saturating_add(4) / card_width.saturating_add(4),
        )
        .max(1);
        let mut counts = vec![0usize; tasks.len()];
        for &level in &levels {
            counts[level] += 1;
        }
        let mut tops = vec![1usize; tasks.len()];
        for level in 1..tops.len() {
            tops[level] = tops[level - 1]
                .saturating_add(counts[level - 1].div_ceil(columns).saturating_mul(9));
        }
        let mut slots = vec![0usize; tasks.len()];
        let nodes: Vec<_> = levels
            .into_iter()
            .map(|level| {
                let slot = slots[level];
                slots[level] += 1;
                let row = slot / columns;
                let column = slot % columns;
                let row_count = (counts[level] - row * columns).min(columns);
                let row_width = row_count * (usize::from(card_width) + 4) - 4;
                let left = usize::from(viewport_width).saturating_sub(row_width) / 2;
                let x = left.saturating_add(column * (usize::from(card_width) + 4));
                let y = tops[level].saturating_add(row.saturating_mul(9));
                Rect::new(
                    x.min(usize::from(u16::MAX - card_width)) as u16,
                    y.min(usize::from(u16::MAX - 6)) as u16,
                    card_width,
                    6,
                )
            })
            .collect();
        Self {
            width: nodes
                .iter()
                .map(|node| node.right().saturating_add(1))
                .max()
                .unwrap_or(0),
            height: nodes
                .iter()
                .map(|node| node.bottom().saturating_add(1))
                .max()
                .unwrap_or(0),
            nodes,
        }
    }

    pub fn render(&self, tasks: &[TaskRow], canvas: &mut Canvas, selected: usize, theme: &Theme) {
        let ids: HashMap<_, _> = tasks
            .iter()
            .enumerate()
            .map(|(index, task)| (task.id.as_str(), index))
            .collect();
        for (index, task) in tasks.iter().enumerate() {
            let target = self.nodes[index];
            for dependency in &task.depends_on {
                let Some(&parent) = ids.get(dependency.as_str()) else {
                    continue;
                };
                let source = self.nodes[parent];
                let from = (source.x.saturating_add(source.width / 2), source.bottom());
                let to = (
                    target.x.saturating_add(target.width / 2),
                    target.y.saturating_sub(1),
                );
                let lane = source.bottom().saturating_add(1);
                canvas.path(
                    &[from, (from.0, lane), (to.0, lane), to],
                    if index == selected || parent == selected {
                        theme.accent()
                    } else {
                        theme.dim()
                    },
                );
                canvas.text(
                    to.0,
                    to.1,
                    if theme.ascii { "v" } else { "▼" },
                    theme.accent(),
                );
            }
        }
        for (index, task) in tasks.iter().enumerate() {
            self.draw_card(task, canvas, self.nodes[index], index == selected, theme);
        }
    }

    fn draw_card(
        &self,
        task: &TaskRow,
        canvas: &mut Canvas,
        rect: Rect,
        selected: bool,
        theme: &Theme,
    ) {
        // Clear edge segments beneath cards, including long edges crossing a layer.
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                canvas.put(x, y, ' ', theme.text());
            }
        }
        let border = if selected {
            theme.accent()
        } else {
            theme.dim()
        };
        let horizontal = theme
            .glyphs()
            .horizontal
            .repeat(usize::from(rect.width.saturating_sub(2)));
        let corners = if theme.ascii {
            ["+", "+", "+", "+"]
        } else {
            ["╭", "╮", "╰", "╯"]
        };
        canvas.text(
            rect.x,
            rect.y,
            &format!("{}{horizontal}{}", corners[0], corners[1]),
            border,
        );
        canvas.text(
            rect.x,
            rect.bottom() - 1,
            &format!("{}{horizontal}{}", corners[2], corners[3]),
            border,
        );
        for y in rect.y + 1..rect.bottom() - 1 {
            canvas.text(rect.x, y, theme.glyphs().vertical, border);
            canvas.text(rect.right() - 1, y, theme.glyphs().vertical, border);
        }
        let width = usize::from(rect.width.saturating_sub(4));
        let header = format!("{} {}", theme.status_glyph(&task.status), task.id);
        let style = theme.status(&task.status);
        canvas.text(
            rect.x + 2,
            rect.y + 1,
            &theme.fit(&header, width),
            if selected {
                style.add_modifier(Modifier::REVERSED)
            } else {
                style
            },
        );
        for (row, text) in wrap(&task.title, width).iter().take(2).enumerate() {
            canvas.text(
                rect.x + 2,
                rect.y + 2 + row as u16,
                &theme.fit(text, width),
                theme.text(),
            );
        }
        canvas.text(
            rect.x + 2,
            rect.y + 4,
            &theme.fit(
                &format!(
                    "{} · {}",
                    task.kind,
                    super::super::view::status_label(&task.status)
                ),
                width,
            ),
            style,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_parallel_layers_wrap_vertically_with_centered_rows() {
        let tasks: Vec<_> = (0..9)
            .map(|index| TaskRow {
                id: format!("t{index}"),
                ..TaskRow::default()
            })
            .collect();
        for width in [20, 60, 80] {
            let graph = PlanGraph::new(&tasks, width);
            assert!(graph.width <= width);
            assert!(graph.nodes.last().unwrap().y > graph.nodes[0].y);
            for node in &graph.nodes {
                let row: Vec<_> = graph
                    .nodes
                    .iter()
                    .filter(|other| other.y == node.y)
                    .collect();
                let left = row.iter().map(|node| node.x).min().unwrap();
                let right = row.iter().map(|node| node.right()).max().unwrap();
                assert!(left.abs_diff(width - right) <= 1);
            }
        }
    }

    #[test]
    fn unordered_diamond_places_parallel_work_in_the_same_layer() {
        let tasks: Vec<_> = [
            ("join", vec!["a", "b"]),
            ("b", vec!["root"]),
            ("root", vec![]),
            ("a", vec!["root"]),
        ]
        .into_iter()
        .map(|(id, deps)| TaskRow {
            id: id.into(),
            depends_on: deps.into_iter().map(str::to_owned).collect(),
            ..TaskRow::default()
        })
        .collect();
        let graph = PlanGraph::new(&tasks, 80);
        assert!(graph.nodes[2].bottom() < graph.nodes[1].y);
        assert_eq!(graph.nodes[1].y, graph.nodes[3].y);
        assert_ne!(graph.nodes[1].x, graph.nodes[3].x);
        assert!(graph.nodes[1].bottom() < graph.nodes[0].y);
        assert_eq!(graph.nodes[2].x, graph.nodes[0].x);
        assert_eq!(graph.nodes[2].x * 2 + graph.nodes[2].width, 80);
        let mut canvas = Canvas::new_window(0, 0, 80, 20);
        graph.render(&tasks, &mut canvas, 0, &Theme::new(false, false));
        assert_eq!(canvas.cells.len(), 80 * 20);
    }
}
