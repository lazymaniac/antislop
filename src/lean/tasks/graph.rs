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
pub fn depends_transitively(tasks: &[TaskContract], from: &str, target: &str) -> bool {
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
pub fn parallel_peers(tasks: &[TaskContract]) -> std::collections::HashMap<String, Vec<String>> {
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
    let index = peer_index(nodes);
    let reaches = dependency_reachability(nodes, &index);
    nodes
        .iter()
        .enumerate()
        .map(|(at, task)| (task.id.to_owned(), peer_ids(at, task, nodes, &reaches)))
        .collect()
}

fn peer_index<'a>(nodes: &[PeerNode<'a>]) -> std::collections::HashMap<&'a str, usize> {
    nodes
        .iter()
        .enumerate()
        .map(|(at, node)| (node.id, at))
        .collect()
}

fn dependency_reachability(
    nodes: &[PeerNode],
    index: &std::collections::HashMap<&str, usize>,
) -> Vec<Vec<bool>> {
    (0..nodes.len())
        .map(|from| reachable_nodes(from, nodes, index))
        .collect()
}

fn reachable_nodes(
    from: usize,
    nodes: &[PeerNode],
    index: &std::collections::HashMap<&str, usize>,
) -> Vec<bool> {
    let mut seen = vec![false; nodes.len()];
    let mut stack = vec![from];
    while let Some(at) = stack.pop() {
        for dependency in nodes[at].depends_on {
            if let Some(&to) = index.get(dependency.as_str())
                && !std::mem::replace(&mut seen[to], true)
            {
                stack.push(to);
            }
        }
    }
    seen
}

fn peer_ids(at: usize, task: &PeerNode, nodes: &[PeerNode], reaches: &[Vec<bool>]) -> Vec<String> {
    nodes
        .iter()
        .enumerate()
        .filter(|&(other_at, other)| peers_can_run(at, task, other_at, other, reaches))
        .map(|(_, other)| other.id.to_owned())
        .collect()
}

fn peers_can_run(
    at: usize,
    task: &PeerNode,
    other_at: usize,
    other: &PeerNode,
    reaches: &[Vec<bool>],
) -> bool {
    other_at != at
        && other.id != task.id
        && !reaches[at][other_at]
        && !reaches[other_at][at]
        && !(task.writes && other.writes && scopes_overlap(task.scope, other.scope))
}

/// Builds a structured Markdown plan document with sections assigned to tasks.
/// If the orchestrator's text already contains Markdown headings or paragraphs,
/// preserves them and appends structured section blocks for any tasks not yet
/// covered by a `#` heading so every task is mapped to a document section.
pub fn render_plan_document(markdown: &str, tasks: &[TaskContract]) -> String {
    let peers = parallel_peers(tasks);
    let trimmed = markdown.trim();
    let mut document = plan_document_start(trimmed);
    for (index, task) in tasks.iter().enumerate() {
        append_plan_task(&mut document, trimmed, index, task, &peers);
    }
    document.trim_end().to_owned()
}

fn plan_document_start(markdown: &str) -> String {
    let mut document = if markdown.is_empty() {
        "# Execution Plan\n\n".to_owned()
    } else {
        format!("{markdown}\n\n")
    };
    if !has_section_headers(markdown) {
        document.push_str("## Plan Sections & Task Breakdown\n\n");
    }
    document
}

fn has_section_headers(markdown: &str) -> bool {
    markdown
        .lines()
        .any(|line| line.trim_start().starts_with('#'))
}

fn append_plan_task(
    document: &mut String,
    markdown: &str,
    index: usize,
    task: &TaskContract,
    peers: &std::collections::HashMap<String, Vec<String>>,
) {
    if markdown.contains(&format!("[{}]", task.id)) {
        return;
    }
    append_task_heading(document, index, task);
    append_task_details(document, task);
    append_task_relationships(document, task, peers);
}

fn append_task_heading(document: &mut String, index: usize, task: &TaskContract) {
    let section = task
        .section
        .as_deref()
        .filter(|section| !section.trim().is_empty())
        .unwrap_or(&task.title);
    document.push_str(&format!("### {}. {} [{}]\n", index + 1, section, task.id));
}

fn append_task_details(document: &mut String, task: &TaskContract) {
    document.push_str(&format!("- **Task**: `{}` — {}\n", task.id, task.title));
    document.push_str(&format!("- **Kind**: `{}`", work_kind_name(task.kind)));
    document.push_str(if task.writes {
        " (writes)\n"
    } else {
        " (read-only)\n"
    });
    document.push_str(&format!("- **Goal**: {}\n", task.goal));
    document.push_str(&format!("- **Scope**: {}\n", scope_text(&task.scope)));
}

fn append_task_relationships(
    document: &mut String,
    task: &TaskContract,
    peers: &std::collections::HashMap<String, Vec<String>>,
) {
    let dependencies = dependency_text(task);
    let parallel = peer_text(task, peers);
    document.push_str(&format!("- **Depends on**: {dependencies}\n"));
    document.push_str(&format!("- **Parallel with**: {parallel}\n\n"));
}

fn dependency_text(task: &TaskContract) -> String {
    if task.depends_on.is_empty() {
        "none (can start immediately)".into()
    } else {
        task.depends_on.join(", ")
    }
}

fn peer_text(
    task: &TaskContract,
    peers: &std::collections::HashMap<String, Vec<String>>,
) -> String {
    peers
        .get(&task.id)
        .filter(|peers| !peers.is_empty())
        .map(|peers| peers.join(", "))
        .unwrap_or_else(|| "sequential".to_owned())
}
