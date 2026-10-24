use super::*;

fn normalize_scope_path(path: &str) -> &str {
    path.trim().trim_start_matches("./").trim_end_matches('/')
}

pub fn work_kind_name(kind: WorkKind) -> &'static str {
    match kind {
        WorkKind::Orchestration => "orchestration",
        WorkKind::Context => "context",
        WorkKind::Requirements => "requirements",
        WorkKind::Options => "options",
        WorkKind::Planning => "planning",
        WorkKind::Architecture => "architecture",
        WorkKind::Dependencies => "dependencies",
        WorkKind::Implementation => "implementation",
        WorkKind::Refactoring => "refactoring",
        WorkKind::StaticAnalysis => "static_analysis",
        WorkKind::TestDesign => "test_design",
        WorkKind::Testing => "testing",
        WorkKind::Security => "security",
        WorkKind::Performance => "performance",
        WorkKind::Integration => "integration",
        WorkKind::Research => "research",
        WorkKind::Writing => "writing",
        WorkKind::Analysis => "analysis",
        WorkKind::Delivery => "delivery",
    }
}

/// Whether two task scopes overlap on disk (empty scope means the whole project).
pub fn scopes_overlap(a: &[String], b: &[String]) -> bool {
    if a.is_empty() || b.is_empty() {
        return true;
    }
    for left in a {
        let left = normalize_scope_path(left);
        if left.is_empty() || left == "." {
            return true;
        }
        for right in b {
            let right = normalize_scope_path(right);
            if right.is_empty() || right == "." || left == right {
                return true;
            }
            if left
                .strip_prefix(right)
                .is_some_and(|rest| rest.starts_with('/'))
                || right
                    .strip_prefix(left)
                    .is_some_and(|rest| rest.starts_with('/'))
            {
                return true;
            }
        }
    }
    false
}

/// Whether task `from` depends directly or transitively on `target`.
pub fn depends_transitively(tasks: &[TaskSpec], from: &str, target: &str) -> bool {
    let mut stack: Vec<&str> = tasks
        .iter()
        .find(|t| t.id == from)
        .map(|t| t.depends_on.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let mut visited = std::collections::BTreeSet::new();
    while let Some(curr) = stack.pop() {
        if curr == target {
            return true;
        }
        if visited.insert(curr)
            && let Some(node) = tasks.iter().find(|t| t.id == curr)
        {
            for dep in &node.depends_on {
                stack.push(dep.as_str());
            }
        }
    }
    false
}

/// What the parallelism rule reads of a task, so the core's specs and a
/// frontend's rows share one implementation of it.
pub struct PeerNode<'a> {
    pub id: &'a str,
    pub depends_on: &'a [String],
    pub writes: bool,
    pub scope: &'a [String],
}

/// For each task id, lists the peer task ids with which it can execute in parallel
/// (neither depends on the other, and if both write, their scopes do not overlap).
pub fn parallel_peers(tasks: &[TaskSpec]) -> std::collections::HashMap<String, Vec<String>> {
    let nodes: Vec<PeerNode> = tasks
        .iter()
        .map(|task| PeerNode {
            id: &task.id,
            depends_on: &task.depends_on,
            writes: task.writes,
            scope: &task.scope,
        })
        .collect();
    peers_of(&nodes)
}

/// [`parallel_peers`] over plain nodes. Each task's dependency chain is walked
/// once, so the cost is the plan's own size (tasks x edges), not a walk per pair.
pub fn peers_of(nodes: &[PeerNode]) -> std::collections::HashMap<String, Vec<String>> {
    let index: std::collections::HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(at, node)| (node.id, at))
        .collect();
    // reaches[i][j]: task i depends, through any chain, on task j.
    let reaches: Vec<Vec<bool>> = (0..nodes.len())
        .map(|from| {
            let mut seen = vec![false; nodes.len()];
            let mut stack: Vec<usize> = vec![from];
            while let Some(at) = stack.pop() {
                for dep in nodes[at].depends_on {
                    if let Some(&to) = index.get(dep.as_str())
                        && !std::mem::replace(&mut seen[to], true)
                    {
                        stack.push(to);
                    }
                }
            }
            seen
        })
        .collect();
    let mut map = std::collections::HashMap::new();
    for (i, task) in nodes.iter().enumerate() {
        let peers = nodes
            .iter()
            .enumerate()
            .filter(|&(j, other)| {
                j != i
                    && other.id != task.id
                    && !reaches[i][j]
                    && !reaches[j][i]
                    && !(task.writes && other.writes && scopes_overlap(task.scope, other.scope))
            })
            .map(|(_, other)| other.id.to_owned())
            .collect();
        map.insert(task.id.to_owned(), peers);
    }
    map
}

/// Builds a structured Markdown plan document with sections assigned to tasks.
/// If the orchestrator's text already contains Markdown headings or paragraphs,
/// preserves them and appends structured section blocks for any tasks not yet
/// covered by a `#` heading so every task is mapped to a document section.
pub fn render_plan_document(markdown: &str, tasks: &[TaskSpec]) -> String {
    let peers = parallel_peers(tasks);
    let trimmed = markdown.trim();
    let mut doc = String::new();
    if !trimmed.is_empty() {
        doc.push_str(trimmed);
        doc.push_str("\n\n");
    } else {
        doc.push_str("# Execution Plan\n\n");
    }
    let has_section_headers = trimmed.lines().any(|l| l.trim_start().starts_with('#'));
    if !has_section_headers {
        doc.push_str("## Plan Sections & Task Breakdown\n\n");
    }
    for (idx, task) in tasks.iter().enumerate() {
        let section_title = task
            .section
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&task.title);
        let heading_marker = format!("### {}. {} [{}]", idx + 1, section_title, task.id);
        if !trimmed.contains(&format!("[{}]", task.id)) {
            doc.push_str(&heading_marker);
            doc.push('\n');
            doc.push_str(&format!("- **Task**: `{}` — {}\n", task.id, task.title));
            doc.push_str(&format!("- **Kind**: `{}`", work_kind_name(task.kind)));
            if task.writes {
                doc.push_str(" (writes)");
            } else {
                doc.push_str(" (read-only)");
            }
            doc.push('\n');
            doc.push_str(&format!("- **Goal**: {}\n", task.goal));
            doc.push_str(&format!("- **Scope**: {}\n", scope_text(&task.scope)));
            let deps = if task.depends_on.is_empty() {
                "none (can start immediately)".to_owned()
            } else {
                task.depends_on.join(", ")
            };
            doc.push_str(&format!("- **Depends on**: {deps}\n"));
            let par = peers
                .get(&task.id)
                .filter(|p| !p.is_empty())
                .map(|p| p.join(", "))
                .unwrap_or_else(|| "sequential".to_owned());
            doc.push_str(&format!("- **Parallel with**: {par}\n\n"));
        }
    }
    doc.trim_end().to_owned()
}
