use super::*;
use crate::lean::store::now_ms;
use crate::lean::tui::view::{Agent, Comment, clean};

pub(super) fn comments_and_agents(
    this: &Details,
    view: &View,
    theme: &Theme,
    width: usize,
    keys: &[String],
) -> Vec<Section> {
    let mut sections = comments(this, view, theme, width);
    let agents = agents(view, theme, width, keys);
    if !agents.body.is_empty() {
        sections.push(agents);
    }
    sections
}

/// Whether each agent runs and what it is doing; agents without a task card
/// have no card title to carry their state.
pub(super) fn now(view: &View, theme: &Theme, width: usize, keys: &[String]) -> Vec<Section> {
    let body: Vec<Line<'static>> = keys
        .iter()
        .filter_map(|key| {
            view.agents
                .get(key)
                .map(|agent| now_lines(view, theme, width, key, agent))
        })
        .flatten()
        .collect();
    if body.is_empty() {
        return Vec::new();
    }
    vec![Section {
        id: "now",
        title: "Now",
        meta: vec![],
        body,
        preview: 14,
        right: false,
    }]
}

fn now_lines(
    view: &View,
    theme: &Theme,
    width: usize,
    key: &str,
    agent: &Agent,
) -> Vec<Line<'static>> {
    let state = if agent.running {
        let since = agent.started_ms.map_or_default(|at| {
            format!(
                " for {}",
                duration(now_ms().saturating_sub(at).max(0) as u64)
            )
        });
        Span::styled(format!("running{since}"), theme.status("working"))
    } else {
        Span::styled("idle", theme.dim())
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!("{} {} ", theme.role_icon(key), agent_name(key)),
            theme.role_accent(key, ""),
        ),
        state,
    ])];
    if let Some(doing) = taskinfo::doing(view, key)
        .or_else(|| (!agent.activity.is_empty()).then(|| clean(&agent.activity)))
    {
        lines.extend(
            wrap(&theme.clean(&doing), width.saturating_sub(4).max(1))
                .into_iter()
                .take(2)
                .map(|line| Line::styled(format!("    {line}"), theme.text())),
        );
    }
    lines
}

fn comments(this: &Details, view: &View, theme: &Theme, width: usize) -> Vec<Section> {
    let comments = this.comments(view);
    if comments.is_empty() {
        return Vec::new();
    }
    let open = comments
        .iter()
        .filter(|comment| comment.status == "open")
        .count();
    let body = comments
        .iter()
        .flat_map(|comment| comment_lines(comment, view, theme, width))
        .collect();
    vec![Section {
        id: "comments",
        title: "Comments",
        meta: vec![Span::styled(format!("{open} open"), theme.dim())],
        body,
        preview: 6,
        right: false,
    }]
}

fn comment_lines(
    comment: &Comment,
    view: &View,
    theme: &Theme,
    width: usize,
) -> Vec<Line<'static>> {
    let open = comment.status == "open";
    let state = view.comment_state(comment.id).to_owned();
    let glyphs = theme.glyphs();
    item(
        theme,
        Span::styled(
            if open { glyphs.active } else { glyphs.done },
            if open { theme.warning() } else { theme.dim() },
        ),
        Some(format!("#{} {state}", comment.id)),
        &comment.text,
        width,
        if open { theme.text() } else { theme.dim() },
    )
}

fn agents(view: &View, theme: &Theme, width: usize, keys: &[String]) -> Section {
    let body = keys
        .iter()
        .filter_map(|key| {
            view.agents
                .get(key)
                .filter(|agent| !agent.provider.is_empty())
                .map(|agent| agent_lines(key, agent, theme, width))
        })
        .flatten()
        .collect();
    Section {
        id: "agents",
        title: "Agents",
        meta: vec![],
        body,
        preview: 10,
        right: true,
    }
}

fn agent_lines(key: &str, agent: &Agent, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::styled(agent_name(key), theme.title())];
    lines.extend(agent_identity(agent, theme, width));
    lines.extend(agent_usage(agent, theme, width));
    lines.extend(agent_activity(agent, theme, width));
    lines
}

fn agent_identity(agent: &Agent, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let text = theme.clean(&format!(
        "{} · {} · {}",
        agent.provider,
        agent.model.as_deref().unwrap_or("default model"),
        agent.effort.as_deref().unwrap_or("default effort")
    ));
    para(theme, &text, width, theme.dim())
}

fn agent_usage(agent: &Agent, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    // Rounds are counted in the title; only what the turns cost is shown.
    let Some(usage) = agent.usage().short() else {
        return Vec::new();
    };
    let text = format!("tokens: {usage}");
    para(theme, &text, width, theme.dim())
}

fn agent_activity(agent: &Agent, theme: &Theme, _width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(last) = agent.last_ms {
        lines.push(Line::styled(
            format!(
                "busy {} (last turn {})",
                duration(agent.busy_ms),
                duration(last)
            ),
            theme.dim(),
        ));
    }
    lines
}
