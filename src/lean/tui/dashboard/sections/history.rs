use super::*;
use crate::lean::tui::view::{Agent, Comment};

pub(super) fn outcome_and_history(
    this: &Details,
    view: &View,
    theme: &Theme,
    width: usize,
    row: &TaskRow,
    keys: &[String],
) -> Vec<Section> {
    let mut sections = vec![outcome(this, theme, width, row)];
    sections.extend(comments(this, view, theme, width));
    sections.extend(rounds(this, theme, row));
    let agents = agents(view, theme, width, keys);
    if !agents.body.is_empty() {
        sections.push(agents);
    }
    sections
}

fn outcome(this: &Details, theme: &Theme, width: usize, row: &TaskRow) -> Section {
    let mut body = Vec::new();
    if let Some(effect) = taskinfo::effect_line(row) {
        body.extend(para(theme, &effect, width, theme.text()));
    }
    append_revisions(&mut body, this, theme);
    append_coverage(&mut body, theme, width, row);
    append_worker_report(&mut body, this, theme, width);
    Section {
        id: "outcome",
        title: "Outcome",
        meta: vec![],
        body,
        preview: 12,
        right: true,
    }
}

fn append_revisions(body: &mut Vec<Line<'static>>, this: &Details, theme: &Theme) {
    let Some((base, result)) = this
        .task
        .as_ref()
        .and_then(|task| Some((task["base"].as_str()?, task["result"].as_str()?)))
    else {
        return;
    };
    body.push(Line::styled(
        format!(
            "git {}..{}",
            &base[..base.len().min(10)],
            &result[..result.len().min(10)]
        ),
        theme.dim(),
    ));
}

fn append_coverage(body: &mut Vec<Line<'static>>, theme: &Theme, width: usize, row: &TaskRow) {
    body.extend(para(
        theme,
        &format!("Obligations: {}", taskinfo::coverage_full(row)),
        width,
        theme.text(),
    ));
    body.extend(para(
        theme,
        &format!("Integration: {}", taskinfo::integration_line(row)),
        width,
        theme.text(),
    ));
}

fn append_worker_report(
    body: &mut Vec<Line<'static>>,
    this: &Details,
    theme: &Theme,
    width: usize,
) {
    let Some(report) = this
        .task
        .as_ref()
        .and_then(|task| task["report"].as_str())
        .filter(|_| this.tab != crate::lean::tui::details::VERIFIER)
    else {
        return;
    };
    body.push(Line::raw(""));
    body.push(Line::styled(
        "Worker report",
        theme.dim().add_modifier(Modifier::BOLD),
    ));
    body.extend(para(theme, report, width, theme.text()));
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

fn rounds(this: &Details, theme: &Theme, row: &TaskRow) -> Vec<Section> {
    let latest = row
        .round
        .max(this.available_rounds.last().copied().unwrap_or(0));
    if latest == 0 {
        return Vec::new();
    }
    vec![Section {
        id: "rounds",
        title: "Rounds",
        meta: vec![Span::styled(format!("{latest}"), theme.dim())],
        body: vec![
            Line::from(round_spans(this, theme, latest)),
            round_hint(this, theme),
        ],
        preview: 4,
        right: true,
    }]
}

fn round_spans(this: &Details, theme: &Theme, latest: u64) -> Vec<Span<'static>> {
    let selected = this.selected_round.unwrap_or(latest);
    (1..=latest)
        .map(|round| {
            let active = round == selected;
            let label = format!(
                "{}r{round}{} ",
                if active { "[" } else { " " },
                if active { "]" } else { "" }
            );
            Span::styled(label, if active { theme.accent() } else { theme.dim() })
        })
        .collect()
}

fn round_hint(this: &Details, theme: &Theme) -> Line<'static> {
    let hint = if this.selected_round.is_some() {
        "History: ←/→ browse, back on the latest round to send."
    } else {
        "Current round. ←/→ browse earlier rounds."
    };
    Line::styled(hint, theme.dim())
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
    let mut text = theme.clean(&format!(
        "{} · {} · {}",
        agent.provider,
        agent.model.as_deref().unwrap_or("default model"),
        agent.effort.as_deref().unwrap_or("default effort")
    ));
    if let Some(pid) = agent.pid {
        text.push_str(&format!(" · pid {pid}"));
    }
    para(theme, &text, width, theme.dim())
}

fn agent_usage(agent: &Agent, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    if agent.turns == 0 {
        return Vec::new();
    }
    let tokens = if agent.reported == 0 {
        "not reported by the CLI".to_owned()
    } else {
        format!(
            "{} in / {} out",
            compact(agent.tokens_in.unwrap_or(0)),
            compact(agent.tokens_out.unwrap_or(0))
        )
    };
    para(
        theme,
        &format!(
            "{} turn{} · tokens: {tokens}",
            agent.turns,
            if agent.turns == 1 { "" } else { "s" }
        ),
        width,
        theme.dim(),
    )
}

fn agent_activity(agent: &Agent, theme: &Theme, width: usize) -> Vec<Line<'static>> {
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
    if let Some(branch) = &agent.branch {
        lines.extend(para(theme, &format!("branch {branch}"), width, theme.dim()));
    }
    lines
}
