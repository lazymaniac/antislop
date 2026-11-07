use super::super::nodes::draw_node;
use super::*;

impl Graph {
    fn arrow(path: &[(u16, u16)]) -> char {
        let [.., (ax, ay), (bx, by)] = path else {
            return '▶';
        };
        match (bx.cmp(ax), by.cmp(ay)) {
            (std::cmp::Ordering::Less, _) => '◀',
            (std::cmp::Ordering::Greater, _) => '▶',
            (_, std::cmp::Ordering::Less) => '▲',
            (_, std::cmp::Ordering::Greater) => '▼',
            _ => '▶',
        }
    }

    /// The arrow pointing into `to` from an end beside it: a route that
    /// runs along under a card and stops below it enters it upwards.
    fn entry_arrow(to: &Node, (x, y): (u16, u16)) -> Option<char> {
        let across = (to.x..to.right()).contains(&x);
        let along = (to.y..to.y + to.height).contains(&y);
        if across && y == to.y + to.height {
            Some('▲')
        } else if across && y + 1 == to.y {
            Some('▼')
        } else if along && x == to.right() {
            Some('◀')
        } else if along && x + 1 == to.x {
            Some('▶')
        } else {
            None
        }
    }

    pub fn render(&self, view: &View, canvas: &mut Canvas, relations: Relations, selected: &str) {
        self.render_with(
            view,
            canvas,
            relations,
            selected,
            &Theme::new(true, false),
            0,
        );
    }

    pub fn render_with(
        &self,
        view: &View,
        canvas: &mut Canvas,
        relations: Relations,
        selected: &str,
        theme: &Theme,
        tick: u64,
    ) {
        let nodes: HashMap<&str, &Node> = self
            .nodes
            .iter()
            .map(|node| (node.key.as_str(), node))
            .collect();
        let selected = card_key(view, selected);
        self.render_frames(view, canvas, &selected, theme);
        self.render_edges(canvas, relations, &selected, &nodes);
        self.render_nodes(view, canvas, selected, theme, tick);
    }

    fn render_edges(
        &self,
        canvas: &mut Canvas,
        relations: Relations,
        selected: &str,
        nodes: &HashMap<&str, &Node>,
    ) {
        let shown = |edge: &&Edge| relations.shows(edge, Some(selected));
        for edge in self.edges.iter().filter(shown) {
            if let Some(path) = self.edge_path(edge, nodes) {
                self.render_edge(canvas, edge, &path);
            }
        }
    }

    /// Each task's frame, titled with the task and its state; the frame
    /// around the selected card stands out.
    fn render_frames(&self, view: &View, canvas: &mut Canvas, selected: &str, theme: &Theme) {
        let group = self
            .nodes
            .iter()
            .find(|node| node.key == selected)
            .and_then(|node| node.group.as_deref());
        for frame in self.frames.iter().filter(|frame| frame.visible) {
            Self::draw_frame(view, canvas, frame, group, theme);
        }
    }

    fn draw_frame(
        view: &View,
        canvas: &mut Canvas,
        frame: &Frame,
        group: Option<&str>,
        theme: &Theme,
    ) {
        let selected = group == Some(frame.key.as_str());
        Self::draw_frame_border(
            canvas,
            frame,
            if selected {
                theme.accent().add_modifier(Modifier::BOLD)
            } else {
                theme.faint()
            },
        );
        let Some(task) = view.tasks.iter().find(|task| task.id == frame.task) else {
            return;
        };
        Self::draw_frame_title(canvas, frame, task, selected, theme);
    }

    fn draw_frame_border(canvas: &mut Canvas, frame: &Frame, style: Style) {
        let (right, bottom) = (
            frame.x + frame.width - 1,
            frame.y + frame.height.saturating_sub(1),
        );
        canvas.path(
            &[
                (frame.x, frame.y),
                (right, frame.y),
                (right, bottom),
                (frame.x, bottom),
                (frame.x, frame.y),
            ],
            style,
        );
    }

    fn draw_frame_title(
        canvas: &mut Canvas,
        frame: &Frame,
        task: &TaskRow,
        selected: bool,
        theme: &Theme,
    ) {
        let (mark, _) = health::mark(&task.status);
        let title = format!(" {mark} {} ", clean(&task.title));
        let style = theme.status(&task.status).patch(if selected {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        });
        canvas.text(
            frame.x + 2,
            frame.y,
            &truncate(&title, usize::from(frame.width.saturating_sub(4))),
            style,
        );
    }

    fn render_edge(&self, canvas: &mut Canvas, edge: &Edge, path: &[(u16, u16)]) {
        let mut path = path.to_vec();
        path.dedup();
        canvas.path(&path, edge.kind.style());
        if let Some(&(x, y)) = path.last() {
            let arrow = self
                .node(&edge.to)
                .and_then(|to| Self::entry_arrow(to, (x, y)))
                .unwrap_or_else(|| Self::arrow(&path));
            canvas.put(x, y, arrow, edge.kind.style());
        }
        // A worker and its reviewer answer each other: their arrow points
        // both ways.
        if edge.from.starts_with("worker:")
            && edge.to.starts_with("reviewer:")
            && let [(x, y), (next_x, _), ..] = path.as_slice()
            && next_x > x
        {
            canvas.put(*x, *y, '◀', edge.kind.style());
        }
        self.render_edge_label(canvas, edge, &path);
    }

    fn render_edge_label(&self, canvas: &mut Canvas, edge: &Edge, path: &[(u16, u16)]) {
        let (Some(label), [(x, y), (next_x, next_y), ..]) = (&edge.label, path) else {
            return;
        };
        let lx = if next_x == x {
            x + 1
        } else if next_x < x {
            x.saturating_sub(unicode_width::UnicodeWidthStr::width(label.as_str()) as u16)
        } else {
            x + 1
        };
        let ly = if next_x == x { *y } else { y.saturating_sub(1) };
        let width = unicode_width::UnicodeWidthStr::width(label.as_str()) as u16;
        // A label goes where no card covers it and over no other edge, such
        // as the arrow between two cards of a task: beside its start, on the
        // other side of the line, else beside the first free cell further on.
        let covered = |lx: u16, ly: u16| {
            self.nodes.iter().any(|node| {
                node.visible
                    && (node.y..node.y + node.height).contains(&ly)
                    && lx < node.x + node.width
                    && node.x < lx + width
            }) || self.frames.iter().any(|frame| {
                let bottom = frame.y + frame.height.saturating_sub(1);
                let right = frame.x + frame.width.saturating_sub(1);
                let across = lx <= right && frame.x < lx + width;
                let down = (frame.y..=bottom).contains(&ly);
                frame.visible
                    && ((across && (ly == frame.y || ly == bottom))
                        || (down && (lx..lx + width).any(|x| x == frame.x || x == right)))
            }) || self.edges.iter().any(|other| {
                other != edge
                    && other.path.as_ref().is_some_and(|path| {
                        path.windows(2).any(|pair| {
                            let (x0, x1) = (pair[0].0.min(pair[1].0), pair[0].0.max(pair[1].0));
                            let (y0, y1) = (pair[0].1.min(pair[1].1), pair[0].1.max(pair[1].1));
                            (y0..=y1).contains(&ly) && lx <= x1 && x0 < lx + width
                        })
                    })
            })
        };
        let other_side = if next_x == x {
            (lx, *next_y)
        } else {
            (lx, y + 1)
        };
        let further = path.windows(2).flat_map(|pair| {
            let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
            let xs = x0.min(x1)..=x0.max(x1);
            xs.flat_map(move |x| (y0.min(y1)..=y0.max(y1)).map(move |y| (x + 1, y)))
        });
        let (lx, ly) = [(lx, ly), other_side]
            .into_iter()
            .chain(further)
            .find(|&(lx, ly)| !covered(lx, ly))
            .unwrap_or(other_side);
        canvas.text(lx, ly, label, edge.kind.style());
    }

    fn render_nodes(
        &self,
        view: &View,
        canvas: &mut Canvas,
        selected: String,
        theme: &Theme,
        tick: u64,
    ) {
        // The expanded card goes last so nothing shows through it.
        let expanded = self.expanded.as_deref();
        let ordered = self
            .nodes
            .iter()
            .filter(|n| n.visible && Some(n.key.as_str()) != expanded)
            .chain(
                self.nodes
                    .iter()
                    .filter(|n| n.visible && Some(n.key.as_str()) == expanded),
            );
        for node in ordered {
            draw_node(
                view,
                canvas,
                node,
                node.key == selected,
                (self.expanded.as_deref() == Some(node.key.as_str())).then_some(self.expanded_tab),
                theme,
                tick,
            );
        }
    }
}
