//! The task dashboard of a worker or verifier: the Overview's second face
//! (`m`), beside the live terminals. A hero strip says where the task stands
//! at a glance; below it, cards (Now, Goal, Checks, Findings, …) hold the
//! detail. Up/Down move between cards, Enter folds or expands one, so a long
//! check list or report never hides the rest. Everything is read from the
//! same sources as the graph cards ([`taskinfo`]), never re-derived.

use super::details::{Details, check_rows};
use super::taskinfo;
use super::theme::Theme;
use super::view::{TaskRow, View, agent_name, clean, duration, status_label, wrap};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

/// Below this the two columns would be narrower than a readable line.
pub(super) const TWO_COLUMNS: u16 = 118;
pub(super) const GAP: u16 = 3;

/// How much of a card is shown. Enter cycles Preview → Full → Folded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fold {
    #[default]
    Preview,
    Full,
    Folded,
}

pub(super) struct Section {
    pub(super) id: &'static str,
    pub(super) title: &'static str,
    pub(super) meta: Vec<Span<'static>>,
    pub(super) body: Vec<Line<'static>>,
    /// Body lines shown before "+N more".
    pub(super) preview: usize,
    pub(super) right: bool,
}

/// The hero's gauge, `3/6` of it filled.
fn gauge(
    theme: &Theme,
    done: usize,
    total: usize,
    width: usize,
    fill: Style,
) -> Vec<Span<'static>> {
    let (full, empty) = if theme.ascii {
        ("#", "-")
    } else {
        ("█", "░")
    };
    let on = if total == 0 {
        0
    } else {
        (done * width).div_ceil(total).min(width)
    };
    vec![
        Span::styled(full.repeat(on), fill),
        Span::styled(empty.repeat(width - on), theme.faint()),
    ]
}

/// 1234 → `1.2k`, so a token count fits a chip.
fn compact(n: u64) -> String {
    match n {
        0..1000 => n.to_string(),
        1000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// A paragraph wrapped to `width`, blank source lines kept.
fn para(theme: &Theme, text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    let text = theme.clean(text);
    text.lines()
        .flat_map(|line| {
            if line.trim().is_empty() {
                vec![String::new()]
            } else {
                wrap(line, width)
            }
        })
        .map(|line| Line::styled(line, style))
        .collect()
}

/// `mark text` wrapped, continuation lines hanging under the text.
fn item(
    theme: &Theme,
    mark: Span<'static>,
    lead: Option<String>,
    text: &str,
    width: usize,
    style: Style,
) -> Vec<Line<'static>> {
    let lead = lead.map(|l| format!("{l} ")).unwrap_or_default();
    let indent = 2 + lead.width();
    let pieces = wrap(&theme.clean(text), width.saturating_sub(indent).max(1));
    let mut out = Vec::new();
    for (index, piece) in pieces.into_iter().enumerate() {
        if index == 0 {
            out.push(Line::from(vec![
                mark.clone(),
                Span::raw(" "),
                Span::styled(lead.clone(), theme.dim()),
                Span::styled(piece, style),
            ]));
        } else {
            out.push(Line::from(vec![
                Span::raw(" ".repeat(indent)),
                Span::styled(piece, style),
            ]));
        }
    }
    if out.is_empty() {
        out.push(Line::from(vec![
            mark,
            Span::raw(" "),
            Span::styled(lead, theme.dim()),
        ]));
    }
    out
}

fn chip(theme: &Theme, label: &str, value: String, style: Style) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!("{label} "), theme.dim()),
        Span::styled(value, style),
    ]
}

fn join_chips(theme: &Theme, chips: Vec<Vec<Span<'static>>>) -> Line<'static> {
    let sep = format!(" {} ", theme.glyphs().vertical);
    let mut spans = vec![Span::raw(" ")];
    for (index, chip) in chips.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(sep.clone(), theme.faint()));
        }
        spans.extend(chip);
    }
    Line::from(spans)
}

impl Details {
    /// The task row this details screen is about.
    fn row<'a>(&self, view: &'a View) -> Option<&'a TaskRow> {
        self.task_id()
            .and_then(|id| view.tasks.iter().find(|task| task.id == id))
    }

    /// The dashboard replaces the terminals while `m` is on and nothing
    /// narrows the text with a search.
    pub fn dashboard_visible(&self) -> bool {
        self.overview()
            && (self.show_info || self.split)
            && self.key != "conversation"
            && self.query.is_empty()
    }

    /// Two lines that stay above both faces of the Overview: where the task
    /// stands, and the numbers that matter.
    pub(super) fn hero(&self, view: &View, theme: &Theme) -> Vec<Line<'static>> {
        let key = self
            .session_keys()
            .get(self.active_panel)
            .cloned()
            .unwrap_or_else(|| self.key.clone());
        let agent = view.agents.get(&key);
        let row = self.row(view);
        let g = theme.glyphs();
        let mut head = vec![Span::raw(" ")];
        match row {
            Some(row) => {
                let state = status_label(&row.status).to_uppercase();
                let round =
                    taskinfo::round_shown(row.round).map_or_default(|round| format!(" {round}"));
                head.push(Span::styled(
                    format!("{} {state}{round}", theme.status_glyph(&row.status)),
                    theme.status(&row.status).add_modifier(Modifier::BOLD),
                ));
                head.push(Span::styled(format!(" {} ", g.vertical), theme.faint()));
                head.push(Span::styled(
                    format!("{} ", theme.clean(&row.id)),
                    theme.role_accent(&key, &row.kind),
                ));
                head.push(Span::styled(
                    theme
                        .clean(row.summary.as_deref().unwrap_or(&row.title))
                        .replace(['\n', '\r', '\t'], " "),
                    theme.title(),
                ));
                head.push(Span::styled(
                    format!(
                        "  {} · {}",
                        theme.clean(&row.kind),
                        if row.writes {
                            "writes files"
                        } else {
                            "read-only"
                        }
                    ),
                    theme.dim(),
                ));
            }
            None => head.push(Span::styled(agent_name(&key), theme.title())),
        }
        let mut chips = Vec::new();
        if let Some(agent) = agent.filter(|agent| !agent.provider.is_empty()) {
            chips.push(vec![Span::styled(
                theme.clean(&format!(
                    "{} · {} · {}",
                    agent.provider,
                    agent.model.as_deref().unwrap_or("default model"),
                    agent.effort.as_deref().unwrap_or("default effort"),
                )),
                theme.dim(),
            )]);
        }
        let (rows, settled, issues) = match &self.task {
            Some(task) => {
                let rows = check_rows(task);
                let settled = rows.iter().filter(|r| r.state == "settled").count();
                let issues = rows
                    .iter()
                    .filter(|r| matches!(r.status.as_deref(), Some("issue" | "blocked")))
                    .count();
                (rows.len(), settled, issues)
            }
            None => (0, 0, 0),
        };
        if rows > 0 {
            let fill = if issues > 0 {
                theme.warning()
            } else {
                theme.status("approved")
            };
            let mut spans = vec![Span::styled("checks ", theme.dim())];
            spans.extend(gauge(theme, settled, rows, 8, fill));
            spans.push(Span::styled(format!(" {settled}/{rows}"), theme.text()));
            if issues > 0 {
                spans.push(Span::styled(
                    format!(" {issues} issue{}", if issues == 1 { "" } else { "s" }),
                    theme.error(),
                ));
            }
            chips.push(spans);
        } else if let Some((ok, total)) = row.and_then(taskinfo::settled_total) {
            chips.push(chip(
                theme,
                "obligations",
                format!("{ok}/{total}"),
                theme.text(),
            ));
        }
        let open = self
            .comments(view)
            .iter()
            .filter(|c| c.status == "open")
            .count();
        if open > 0 {
            chips.push(chip(
                theme,
                "comments",
                format!("{open} open"),
                theme.warning(),
            ));
        }
        // The task spends tokens and time in both of its sessions.
        let agents: Vec<_> = self
            .role_keys()
            .iter()
            .filter_map(|key| view.agents.get(key))
            .collect();
        if agents.iter().any(|agent| agent.reported > 0) {
            let sum = |f: fn(&super::view::Agent) -> Option<u64>| {
                agents.iter().filter_map(|agent| f(agent)).sum::<u64>()
            };
            chips.push(chip(
                theme,
                "tokens",
                format!(
                    "{} in / {} out",
                    compact(sum(|a| a.tokens_in)),
                    compact(sum(|a| a.tokens_out))
                ),
                theme.text(),
            ));
        }
        let busy: u64 = agents.iter().map(|agent| agent.busy_ms).sum();
        if busy > 0 {
            chips.push(chip(theme, "busy", duration(busy), theme.text()));
        }
        if let Some(row) = row.filter(|row| row.integration.is_some()) {
            chips.push(chip(
                theme,
                "landing",
                taskinfo::integration_label(row),
                theme.text(),
            ));
        }
        vec![Line::from(head), join_chips(theme, chips), Line::raw("")]
    }

    pub(super) fn sections(&self, view: &View, theme: &Theme, width: usize) -> Vec<Section> {
        let g = theme.glyphs();
        let row = self.row(view);
        let keys = self.session_keys();
        let mut out = Vec::new();
        let rows = self.task.as_ref().map(check_rows).unwrap_or_default();

        // Now: what each agent does this moment, and what happens next.
        let mut now = Vec::new();
        for key in &keys {
            let Some(agent) = view.agents.get(key) else {
                continue;
            };
            let role = key.split(':').next().unwrap_or(key);
            let state = if agent.running {
                let since = agent
                    .started_ms
                    .map(|at| {
                        format!(
                            " for {}",
                            duration(now_ms().saturating_sub(at).max(0) as u64)
                        )
                    })
                    .unwrap_or_default();
                Span::styled(format!("running{since}"), theme.status("working"))
            } else {
                Span::styled("idle", theme.dim())
            };
            now.push(Line::from(vec![
                Span::styled(
                    format!("{} {role:<9}", theme.role_icon(key)),
                    theme.role_accent(key, row.map_or("", |r| r.kind.as_str())),
                ),
                state,
            ]));
            let doing = taskinfo::doing(view, key)
                .or_else(|| (!agent.activity.is_empty()).then(|| clean(&agent.activity)));
            if let Some(doing) = doing {
                for line in wrap(&theme.clean(&doing), width.saturating_sub(4).max(1))
                    .into_iter()
                    .take(2)
                {
                    now.push(Line::styled(format!("    {line}"), theme.text()));
                }
            }
        }
        if let Some(row) = row {
            if let Some(cause) = taskinfo::cause(row) {
                let reason = row
                    .reason
                    .as_deref()
                    .map(theme_clean_one(theme))
                    .unwrap_or(cause);
                now.extend(item(
                    theme,
                    Span::styled(g.problem, theme.error()),
                    Some("stopped:".into()),
                    &reason,
                    width,
                    theme.error(),
                ));
            }
            now.push(Line::raw(""));
            now.extend(para(
                theme,
                &taskinfo::next_step(view, row),
                width,
                theme.accent(),
            ));
        }
        if !now.is_empty() {
            out.push(Section {
                id: "now",
                title: "Now",
                meta: vec![],
                body: now,
                preview: 14,
                right: false,
            });
        }

        let Some(row) = row else {
            return out;
        };

        out.push(Section {
            id: "goal",
            title: "Goal",
            meta: vec![],
            body: para(theme, &row.goal, width, theme.text()),
            preview: 8,
            right: false,
        });
        if !row.focus.trim().is_empty() {
            out.push(Section {
                id: "verify",
                title: "Verification",
                meta: vec![],
                body: para(theme, &row.focus, width, theme.text()),
                preview: 6,
                right: false,
            });
        }
        if !row.risks.is_empty() {
            let mut body = Vec::new();
            for (index, risk) in row.risks.iter().enumerate() {
                let id = format!("R{}", index + 1);
                let (mark, style) = rows
                    .iter()
                    .find(|r| r.id == id)
                    .map_or((g.queued, theme.dim()), |r| (r.mark, r.style));
                body.extend(item(
                    theme,
                    Span::styled(mark, if theme.color { style } else { Style::default() }),
                    Some(id),
                    risk,
                    width,
                    theme.text(),
                ));
            }
            out.push(Section {
                id: "risks",
                title: "Risks",
                meta: vec![Span::styled(row.risks.len().to_string(), theme.dim())],
                body,
                preview: 8,
                right: false,
            });
        }
        let mut scope = Vec::new();
        for path in &row.scope {
            scope.extend(item(
                theme,
                Span::styled(g.dot, theme.dim()),
                None,
                path,
                width,
                theme.text(),
            ));
        }
        for dep in &row.depends_on {
            let state = view.tasks.iter().find(|t| &t.id == dep);
            let (mark, style, label) = match state {
                Some(t) => (
                    theme.status_glyph(&t.status),
                    theme.status(&t.status),
                    status_label(&t.status).to_owned(),
                ),
                None => (g.queued, theme.dim(), "unknown".to_owned()),
            };
            scope.push(Line::from(vec![
                Span::styled(mark, style),
                Span::styled(format!(" after {dep} "), theme.text()),
                Span::styled(label, theme.dim()),
            ]));
        }
        let spec = self.task.as_ref().map(|task| &task["spec"]);
        let listed = |field: &str| -> Vec<String> {
            spec.and_then(|spec| spec[field].as_array())
                .map(|ids| {
                    ids.iter()
                        .filter_map(|id| id.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        if let Some(section) = &row.section {
            scope.extend(para(
                theme,
                &format!("section: {section}"),
                width,
                theme.dim(),
            ));
        }
        let requirements = listed("requirement_ids");
        if !requirements.is_empty() {
            scope.extend(para(
                theme,
                &format!("serves requirements {}", requirements.join(", ")),
                width,
                theme.dim(),
            ));
        }
        if let Some(resolves) = spec.and_then(|spec| spec["resolves"].as_str()) {
            scope.extend(para(
                theme,
                &format!("clears the blocker of {resolves}"),
                width,
                theme.dim(),
            ));
        }
        if !row.parallel_with.is_empty() {
            scope.push(Line::styled(
                format!("{} parallel with {}", g.dot, row.parallel_with.join(", ")),
                theme.dim(),
            ));
        }
        if !scope.is_empty() {
            out.push(Section {
                id: "scope",
                title: "Scope & dependencies",
                meta: vec![],
                body: scope,
                preview: 8,
                right: false,
            });
        }

        // Checks: every obligation of the verifier and how far it is.
        if !rows.is_empty() {
            let settled = rows.iter().filter(|r| r.state == "settled").count();
            let issues = rows
                .iter()
                .filter(|r| matches!(r.status.as_deref(), Some("issue" | "blocked")))
                .count();
            let mut meta = gauge(theme, settled, rows.len(), 8, theme.status("approved"));
            meta.push(Span::styled(
                format!(" {settled}/{}", rows.len()),
                theme.dim(),
            ));
            if issues > 0 {
                meta.push(Span::styled(
                    format!(" · {issues} with issues"),
                    theme.error(),
                ));
            }
            let mut body = Vec::new();
            // Trouble first: what a reader must look at leads the list.
            let rank = |r: &super::details::CheckRow| match (r.state, r.status.as_deref()) {
                (_, Some("issue")) => 0,
                (_, Some("blocked")) => 1,
                ("checking", _) => 2,
                ("unaccounted", _) => 3,
                ("pending", _) => 4,
                _ => 5,
            };
            let mut ordered: Vec<_> = rows.iter().collect();
            ordered.sort_by_key(|r| rank(r));
            for r in ordered {
                if r.state == "pending" {
                    // Not started: one dim line each, so they never bury the rest.
                    let lead = format!("{} {} ", r.mark, r.id);
                    let room = width.saturating_sub(lead.width()).max(1);
                    body.push(Line::from(vec![
                        Span::styled(lead, theme.dim()),
                        Span::styled(theme.fit(&theme.clean(&r.text), room), theme.dim()),
                    ]));
                    continue;
                }
                body.extend(item(
                    theme,
                    Span::styled(
                        r.mark,
                        if theme.color {
                            r.style
                        } else {
                            Style::default()
                        },
                    ),
                    Some(r.id.clone()),
                    &r.text,
                    width,
                    theme.text(),
                ));
                if let Some(note) = &r.note {
                    for line in wrap(&theme.clean(note), width.saturating_sub(4).max(1)) {
                        body.push(Line::styled(format!("    {line}"), theme.dim()));
                    }
                }
            }
            out.push(Section {
                id: "checks",
                title: "Checks",
                meta,
                body,
                preview: 14,
                right: true,
            });
        }

        // Findings of the verdict, severest first.
        if let Some(stored) = &self.task {
            let verdict = &stored["verdict"];
            if !verdict.is_null() {
                let mut findings: Vec<&Value> = verdict["findings"]
                    .as_array()
                    .map(|f| f.iter().collect())
                    .unwrap_or_default();
                let sev = |f: &Value| match f["severity"].as_str().unwrap_or("note") {
                    "critical" | "high" | "blocker" => 0,
                    "medium" | "major" => 1,
                    _ => 2,
                };
                findings.sort_by_key(|f| sev(f));
                let mut body = Vec::new();
                if findings.is_empty() {
                    body.push(Line::styled(
                        "No issues: the verifier approved without notes.",
                        theme.status("approved"),
                    ));
                }
                for f in &findings {
                    let level = f["severity"].as_str().unwrap_or("note");
                    let style = match sev(f) {
                        0 => theme.error(),
                        1 => theme.warning(),
                        _ => theme.dim(),
                    };
                    body.extend(item(
                        theme,
                        Span::styled(g.problem, style),
                        Some(format!("[{level}]")),
                        f["issue"].as_str().unwrap_or_default(),
                        width,
                        theme.text(),
                    ));
                    if let Some(place) = f["where"].as_str().filter(|w| !w.is_empty()) {
                        body.push(Line::styled(
                            format!("    at {}", theme.clean(place)),
                            theme.dim(),
                        ));
                    }
                }
                out.push(Section {
                    id: "findings",
                    title: "Findings",
                    meta: vec![Span::styled(
                        findings.len().to_string(),
                        if findings.is_empty() {
                            theme.dim()
                        } else {
                            theme.warning()
                        },
                    )],
                    body,
                    preview: 10,
                    right: true,
                });
            }
        }

        // Outcome: what the round changed and how it landed.
        let mut outcome = Vec::new();
        if let Some(effect) = taskinfo::effect_line(row) {
            outcome.extend(para(theme, &effect, width, theme.text()));
        }
        if let Some(stored) = &self.task
            && let (Some(base), Some(result)) = (stored["base"].as_str(), stored["result"].as_str())
        {
            outcome.push(Line::styled(
                format!(
                    "git {}..{}",
                    &base[..base.len().min(10)],
                    &result[..result.len().min(10)]
                ),
                theme.dim(),
            ));
        }
        outcome.extend(para(
            theme,
            &format!("Obligations: {}", taskinfo::coverage_full(row)),
            width,
            theme.text(),
        ));
        outcome.extend(para(
            theme,
            &format!("Integration: {}", taskinfo::integration_line(row)),
            width,
            theme.text(),
        ));
        if let Some(report) = self
            .task
            .as_ref()
            .and_then(|t| t["report"].as_str())
            .filter(|_| self.tab != super::details::VERIFIER)
        {
            outcome.push(Line::raw(""));
            outcome.push(Line::styled(
                "Worker report",
                theme.dim().add_modifier(Modifier::BOLD),
            ));
            outcome.extend(para(theme, report, width, theme.text()));
        }
        out.push(Section {
            id: "outcome",
            title: "Outcome",
            meta: vec![],
            body: outcome,
            preview: 12,
            right: true,
        });

        let comments = self.comments(view);
        if !comments.is_empty() {
            let open = comments.iter().filter(|c| c.status == "open").count();
            let mut body = Vec::new();
            for c in &comments {
                let state = view.comment_state(c.id).to_owned();
                body.extend(item(
                    theme,
                    Span::styled(
                        if c.status == "open" { g.active } else { g.done },
                        if c.status == "open" {
                            theme.warning()
                        } else {
                            theme.dim()
                        },
                    ),
                    Some(format!("#{} {state}", c.id)),
                    &c.text,
                    width,
                    if c.status == "open" {
                        theme.text()
                    } else {
                        theme.dim()
                    },
                ));
            }
            out.push(Section {
                id: "comments",
                title: "Comments",
                meta: vec![Span::styled(format!("{open} open"), theme.dim())],
                body,
                preview: 6,
                right: false,
            });
        }

        // Rounds: where this view sits in the task's history.
        let latest = row
            .round
            .max(self.available_rounds.last().copied().unwrap_or(0));
        if latest > 0 {
            let selected = self.selected_round.unwrap_or(latest);
            let mut spans = Vec::new();
            for round in 1..=latest {
                let current = round == selected;
                spans.push(Span::styled(
                    format!(
                        "{}r{round}{} ",
                        if current { "[" } else { " " },
                        if current { "]" } else { "" }
                    ),
                    if current { theme.accent() } else { theme.dim() },
                ));
            }
            let body = vec![
                Line::from(spans),
                Line::styled(
                    if self.selected_round.is_some() {
                        "History: ←/→ browse, back on the latest round to send."
                    } else {
                        "Current round. ←/→ browse earlier rounds."
                    },
                    theme.dim(),
                ),
            ];
            out.push(Section {
                id: "rounds",
                title: "Rounds",
                meta: vec![Span::styled(format!("{latest}"), theme.dim())],
                body,
                preview: 4,
                right: true,
            });
        }

        // Agents: provider, model, spend and time of each session.
        let mut agents = Vec::new();
        for key in &keys {
            let Some(agent) = view.agents.get(key).filter(|a| !a.provider.is_empty()) else {
                continue;
            };
            agents.push(Line::styled(agent_name(key), theme.title()));
            let mut line = theme.clean(&format!(
                "{} · {} · {}",
                agent.provider,
                agent.model.as_deref().unwrap_or("default model"),
                agent.effort.as_deref().unwrap_or("default effort")
            ));
            if let Some(pid) = agent.pid {
                line.push_str(&format!(" · pid {pid}"));
            }
            agents.extend(para(theme, &line, width, theme.dim()));
            if agent.turns > 0 {
                agents.extend(para(
                    theme,
                    &format!(
                        "{} turn{} · tokens: {}",
                        agent.turns,
                        if agent.turns == 1 { "" } else { "s" },
                        if agent.reported == 0 {
                            "not reported by the CLI".to_owned()
                        } else {
                            format!(
                                "{} in / {} out",
                                compact(agent.tokens_in.unwrap_or(0)),
                                compact(agent.tokens_out.unwrap_or(0))
                            )
                        }
                    ),
                    width,
                    theme.dim(),
                ));
            }
            if let Some(last) = agent.last_ms {
                agents.push(Line::styled(
                    format!(
                        "busy {} (last turn {})",
                        duration(agent.busy_ms),
                        duration(last)
                    ),
                    theme.dim(),
                ));
            }
            if let Some(branch) = &agent.branch {
                agents.extend(para(theme, &format!("branch {branch}"), width, theme.dim()));
            }
        }
        if !agents.is_empty() {
            out.push(Section {
                id: "agents",
                title: "Agents",
                meta: vec![],
                body: agents,
                preview: 10,
                right: true,
            });
        }
        // Each section shows what its role cares about: the worker its
        // assignment and result, the verifier what it checks and finds.
        let verifier = self.tab == super::details::VERIFIER;
        out.retain(|section| match section.id {
            "goal" | "scope" => !verifier,
            "verify" | "risks" | "checks" | "findings" => verifier,
            _ => true,
        });
        out
    }

    pub(super) fn fold_of(&self, id: &str) -> Fold {
        self.dash_fold.get(id).copied().unwrap_or_default()
    }

    /// Moves the card focus by `delta`; clamped at both ends.
    pub fn dash_move(&mut self, delta: isize) {
        let last = self.dash_ids.len().saturating_sub(1);
        self.dash_focus = (self.dash_focus as isize + delta).clamp(0, last as isize) as usize;
        self.dash_reveal = true;
    }

    pub fn dash_jump(&mut self, last: bool) {
        self.dash_focus = if last {
            self.dash_ids.len().saturating_sub(1)
        } else {
            0
        };
        self.dash_reveal = true;
    }

    /// Enter on a card: preview → full → folded → preview.
    pub fn dash_cycle(&mut self) {
        if let Some(id) = self.dash_ids.get(self.dash_focus).copied() {
            let next = match self.fold_of(id) {
                Fold::Preview => Fold::Full,
                Fold::Full => Fold::Folded,
                Fold::Folded => Fold::Preview,
            };
            self.dash_fold.insert(id, next);
            self.dash_reveal = true;
        }
    }

    /// `e`: everything open in full, or, when it already is, back to previews.
    pub fn dash_expand_all(&mut self) {
        let all_full = self
            .dash_ids
            .iter()
            .all(|id| self.fold_of(id) == Fold::Full);
        let next = if all_full { Fold::Preview } else { Fold::Full };
        for id in self.dash_ids.clone() {
            self.dash_fold.insert(id, next);
        }
    }

    pub fn dash_scroll(&mut self, delta: isize) {
        self.dash_scroll = (self.dash_scroll as isize + delta).max(0) as usize;
    }
}

/// The raw reason as one whole line, never cut.
fn theme_clean_one(theme: &Theme) -> impl Fn(&str) -> String + '_ {
    move |text| {
        theme
            .clean(text)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
}
