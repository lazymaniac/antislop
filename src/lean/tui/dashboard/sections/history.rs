use super::*;
use crate::lean::tui::view::{Agent, Comment};

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
