use super::*;

impl Graph {
    /// The card of `key`; a verifier sharing its worker's card finds that.
    pub fn node(&self, key: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| node.key == key).or_else(|| {
            let (id, verifier) = crate::lean::task_role(key)?;
            self.nodes
                .iter()
                .find(|node| verifier && node.key.strip_prefix("worker:") == Some(id))
        })
    }

    /// The frame of `group`, `task:{id}`.
    pub(super) fn frame(&self, group: &str) -> Option<&Frame> {
        self.frames.iter().find(|frame| frame.key == group)
    }

    /// Where an edge runs, if both its ends are shown: its own path when
    /// the layout laid one, else a route between its two cards.
    pub(super) fn edge_path(
        &self,
        edge: &Edge,
        nodes: &HashMap<&str, &Node>,
    ) -> Option<Vec<(u16, u16)>> {
        let shown = |key: &str| {
            nodes
                .get(key)
                .map(|node| node.visible)
                .unwrap_or_else(|| self.frame(key).is_some_and(|frame| frame.visible))
        };
        if !shown(&edge.from) || !shown(&edge.to) {
            return None;
        }
        if let Some(path) = &edge.path {
            return Some(path.clone());
        }
        let from = nodes.get(edge.from.as_str())?;
        let to = nodes.get(edge.to.as_str())?;
        Some(Self::route(from, to))
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

    /// Returns actual routes for visible edges of the given relations.
    pub fn edge_routes(&self, relations: Relations) -> Vec<EdgeRoute<'_>> {
        let nodes: HashMap<&str, &Node> = self
            .nodes
            .iter()
            .map(|node| (node.key.as_str(), node))
            .collect();
        self.edges
            .iter()
            .filter(|edge| relations.shows(edge, None))
            .filter_map(|edge| {
                let path = self.edge_path(edge, &nodes)?;
                Some((edge.from.as_str(), edge.to.as_str(), path))
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
}
