use super::nodes::draw_node;
use super::*;

impl Graph {
    pub fn node(&self, key: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| {
            node.key == key
                || crate::lean::task_role(key).is_some_and(|(id, verifier)| {
                    verifier && node.key.strip_prefix("worker:") == Some(id)
                })
        })
    }

    /// The nearest node from `key` in a direction (dx, dy), each -1, 0 or 1.
    /// Only navigates between visible nodes.
    pub fn neighbour(&self, key: &str, dx: i32, dy: i32) -> Option<String> {
        let from = self.node(key)?;
        let center = |node: &Node| {
            (
                i32::from(node.x) + i32::from(node.width) / 2,
                i32::from(node.y) + i32::from(node.height) / 2,
            )
        };
        let (fx, fy) = center(from);
        self.nodes
            .iter()
            .filter(|node| node.visible && node.key != from.key)
            .filter_map(|node| {
                let (x, y) = center(node);
                let (along, across) = if dx != 0 {
                    ((x - fx) * dx, (y - fy).abs())
                } else {
                    ((y - fy) * dy, (x - fx).abs())
                };
                // Rows are closer together than columns; weigh them equally.
                let (along, across) = if dx != 0 {
                    (along, across * 5)
                } else {
                    (along * 5, across)
                };
                (along > 0).then_some((along + 2 * across, node))
            })
            .min_by_key(|(cost, _)| *cost)
            .map(|(_, node)| node.key.clone())
    }

    /// The orthogonal path of an edge, through the gaps between boxes only.
    pub fn route(from: &Node, to: &Node) -> Vec<(u16, u16)> {
        Self::route_same_column(from, to)
            .or_else(|| Self::route_same_row(from, to))
            .unwrap_or_else(|| Self::route_between_rows(from, to))
    }

    fn route_same_column(from: &Node, to: &Node) -> Option<Vec<(u16, u16)>> {
        if from.x != to.x {
            return None;
        }
        let half = GAP_X / 2;
        let adjacent = if from.y < to.y {
            from.y.saturating_add(from.height).saturating_add(GAP_Y) == to.y
        } else {
            to.y.saturating_add(to.height).saturating_add(GAP_Y) == from.y
        };
        let x = to.x + to.width / 2;
        if adjacent && from.y > to.y {
            return Some(vec![(x, from.y - 1), (x, to.y.saturating_add(to.height))]);
        }
        if adjacent && from.y < to.y {
            return Some(vec![
                (x, from.y.saturating_add(from.height)),
                (x, to.y.saturating_sub(1)),
            ]);
        }
        let lane = from.right() + half;
        Some(vec![
            (from.right(), from.mid_y()),
            (lane, from.mid_y()),
            (lane, to.mid_y()),
            (to.right(), to.mid_y()),
        ])
    }

    fn route_same_row(from: &Node, to: &Node) -> Option<Vec<(u16, u16)>> {
        if from.y != to.y {
            return None;
        }
        if from.right() + GAP_X == to.x {
            return Some(vec![(from.right(), from.mid_y()), (to.x - 1, to.mid_y())]);
        }
        if to.right() + GAP_X == from.x {
            return Some(vec![(from.x - 1, from.mid_y()), (to.right(), to.mid_y())]);
        }
        None
    }

    fn route_between_rows(from: &Node, to: &Node) -> Vec<(u16, u16)> {
        let half = GAP_X / 2;
        let out = from.right() + half;
        let into = to.x.saturating_sub(half);
        let mut path = vec![(from.right(), from.mid_y()), (out, from.mid_y())];
        if out != into {
            let lane = if to.y == 0 {
                to.height
            } else if to.y > from.y {
                to.y.saturating_sub(1)
            } else {
                to.y.saturating_add(to.height)
            };
            path.push((out, lane));
            path.push((into, lane));
        }
        path.push((into, to.mid_y()));
        path.push((to.x.saturating_sub(1), to.mid_y()));
        path
    }

    /// Returns actual routes for visible edges under the given filter.
    pub fn edge_routes(&self, filter: Filter) -> Vec<EdgeRoute<'_>> {
        let nodes: HashMap<&str, &Node> = self
            .nodes
            .iter()
            .map(|node| (node.key.as_str(), node))
            .collect();
        self.edges
            .iter()
            .filter(|edge| filter.shows(edge.kind))
            .filter_map(|edge| {
                let from = nodes.get(edge.from.as_str())?;
                let to = nodes.get(edge.to.as_str())?;
                if !from.visible || !to.visible {
                    return None;
                }
                Some((edge.from.as_str(), edge.to.as_str(), Self::route(from, to)))
            })
            .collect()
    }

    /// Checks whether any segment of a route penetrates the interior or border of a node card.
    pub fn route_intersects_card(path: &[(u16, u16)], node: &Node) -> bool {
        if node.height == 0 {
            return false;
        }
        for pair in path.windows(2) {
            let (p1, p2) = (pair[0], pair[1]);
            let x_min = p1.0.min(p2.0);
            let x_max = p1.0.max(p2.0);
            let y_min = p1.1.min(p2.1);
            let y_max = p1.1.max(p2.1);
            if x_max >= node.x
                && x_min < node.x.saturating_add(node.width)
                && y_max >= node.y
                && y_min < node.y.saturating_add(node.height)
            {
                return true;
            }
        }
        false
    }

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

    pub fn render(&self, view: &View, canvas: &mut Canvas, filter: Filter, selected: &str) {
        self.render_with(view, canvas, filter, selected, &Theme::new(true, false), 0);
    }

    pub fn render_with(
        &self,
        view: &View,
        canvas: &mut Canvas,
        filter: Filter,
        selected: &str,
        theme: &Theme,
        tick: u64,
    ) {
        let nodes: HashMap<&str, &Node> = self
            .nodes
            .iter()
            .map(|node| (node.key.as_str(), node))
            .collect();
        let selected = task_node_key(selected);
        self.render_edges(canvas, filter, &nodes);
        self.render_nodes(view, canvas, selected, theme, tick);
    }

    fn render_edges(&self, canvas: &mut Canvas, filter: Filter, nodes: &HashMap<&str, &Node>) {
        for edge in self.edges.iter().filter(|edge| filter.shows(edge.kind)) {
            let Some((from, to)) = nodes
                .get(edge.from.as_str())
                .zip(nodes.get(edge.to.as_str()))
            else {
                continue;
            };
            if from.visible && to.visible {
                self.render_edge(canvas, edge, from, to);
            }
        }
    }

    fn render_edge(&self, canvas: &mut Canvas, edge: &Edge, from: &Node, to: &Node) {
        let path = Self::route(from, to);
        canvas.path(&path, edge.kind.style());
        if let Some(&(x, y)) = path.last() {
            canvas.put(x, y, Self::arrow(&path), edge.kind.style());
        }
        self.render_edge_label(canvas, edge, &path);
    }

    fn render_edge_label(&self, canvas: &mut Canvas, edge: &Edge, path: &[(u16, u16)]) {
        let (Some(label), [(x, y), (next_x, _), ..]) = (&edge.label, path) else {
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
        for node in self.nodes.iter().filter(|n| n.visible) {
            draw_node(view, canvas, node, node.key == selected, theme, tick);
        }
    }
}
