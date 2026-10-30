use super::super::*;

pub(super) fn task_columns(
    view: &View,
    index: &HashMap<&str, usize>,
    grouping: GroupingMode,
) -> Vec<u16> {
    match grouping {
        GroupingMode::Dependency => dependency_columns(view, index),
        GroupingMode::Status => status_columns(view),
    }
}

fn dependency_columns(view: &View, index: &HashMap<&str, usize>) -> Vec<u16> {
    let mut graph = DependencyGraph::new(view.tasks.len());
    graph.add_dependencies(view, index);
    let visited = graph.assign_depths();
    if visited < graph.columns.len() {
        graph.repair_cycles();
    }
    graph.columns
}

fn status_columns(view: &View) -> Vec<u16> {
    view.tasks
        .iter()
        .map(|task| match task.status.as_str() {
            "proposed" | "queued" => 0,
            "working" | "verifying" => 1,
            "approved" => 2,
            _ => 3,
        })
        .collect()
}

struct DependencyGraph {
    columns: Vec<u16>,
    adjacent: Vec<Vec<usize>>,
    parents: Vec<Vec<usize>>,
    in_degree: Vec<usize>,
}

impl DependencyGraph {
    fn new(size: usize) -> Self {
        Self {
            columns: vec![0; size],
            adjacent: vec![Vec::new(); size],
            parents: vec![Vec::new(); size],
            in_degree: vec![0; size],
        }
    }

    fn add_dependencies(&mut self, view: &View, index: &HashMap<&str, usize>) {
        for (child, task) in view.tasks.iter().enumerate() {
            for parent_id in &task.depends_on {
                self.add_dependency(child, parent_id, index);
            }
        }
    }

    fn add_dependency(&mut self, child: usize, parent_id: &str, index: &HashMap<&str, usize>) {
        let Some(&parent) = index.get(parent_id) else {
            return;
        };
        if parent == child {
            return;
        }
        self.adjacent[parent].push(child);
        self.parents[child].push(parent);
        self.in_degree[child] += 1;
    }

    fn assign_depths(&mut self) -> usize {
        let mut queue = self.roots();
        let mut visited = 0;
        while let Some(parent) = queue.pop_front() {
            visited += 1;
            self.visit_children(parent, &mut queue);
        }
        visited
    }

    fn roots(&self) -> std::collections::VecDeque<usize> {
        self.in_degree
            .iter()
            .enumerate()
            .filter_map(|(index, degree)| (*degree == 0).then_some(index))
            .collect()
    }

    fn visit_children(&mut self, parent: usize, queue: &mut std::collections::VecDeque<usize>) {
        for &child in &self.adjacent[parent] {
            self.columns[child] = self.columns[child].max(self.columns[parent] + 1);
            self.in_degree[child] = self.in_degree[child].saturating_sub(1);
            if self.in_degree[child] == 0 {
                queue.push_back(child);
            }
        }
    }

    fn repair_cycles(&mut self) {
        for child in 0..self.columns.len() {
            if self.in_degree[child] > 0 {
                self.columns[child] = self.cycle_parent_depth(child);
            }
        }
    }

    fn cycle_parent_depth(&self, child: usize) -> u16 {
        self.parents[child]
            .iter()
            .filter(|&&parent| self.in_degree[parent] == 0)
            .map(|&parent| self.columns[parent] + 1)
            .max()
            .unwrap_or(0)
    }
}
