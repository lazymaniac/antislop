use super::super::super::card_content::{Content, Tone};
use super::*;

impl Card<'_> {
    pub(super) fn paint_body(&mut self) {
        if let Some(tab) = self.expanded_tab {
            self.paint_expanded(tab);
            return;
        }
        let content = Content::new(
            self.view,
            &self.node.key,
            self.node.width,
            self.node.height,
            self.node.collapsed,
        );
        let (_, status) = super::content::status_style(self.theme, self.health);
        for (row, (tone, text)) in content.rows.iter().enumerate() {
            let style = match tone {
                Tone::Text => self.theme.text(),
                Tone::Dim => self.theme.dim(),
                Tone::Status => status,
                Tone::Activity => match self.health {
                    Health::Problem => self.theme.error(),
                    Health::Waiting => self.theme.warning(),
                    _ => self.theme.dim(),
                },
            };
            if row + 2 >= usize::from(self.node.height) {
                break;
            }
            self.canvas.text(
                self.node.x + 2,
                self.node.y + 2 + row as u16,
                &truncate(text, self.inner_width()),
                self.fill.patch(style),
            );
        }
    }

    fn paint_expanded(&mut self, tab: usize) {
        let tabs = if self
            .view
            .agents
            .get(&self.node.key)
            .is_some_and(|agent| agent.branch.is_some())
        {
            "1 Details | 2 PTY | 3 Results | 4 Diff"
        } else {
            "1 Details | 2 PTY | 3 Results"
        };
        let tabs = if tab == 1 {
            format!("{tabs} | Ctrl+] leave PTY")
        } else {
            tabs.to_owned()
        };
        self.canvas.text(
            self.node.x + 2,
            self.node.y + 2,
            &tabs,
            self.fill.patch(self.theme.accent()),
        );
        let mut rows = Vec::new();
        let agent = self.view.agents.get(&self.node.key).or_else(|| {
            card_members(self.view, &self.node.key)
                .into_iter()
                .find_map(|key| self.view.agents.get(key))
        });
        match tab {
            0 => {
                if let Some(agent) = agent {
                    rows.push(format!("CLI: {}", nonempty(&agent.provider)));
                    rows.push(format!(
                        "Model: {}",
                        agent.model.as_deref().unwrap_or("unknown")
                    ));
                    rows.push(format!(
                        "Elapsed: {}",
                        agent
                            .last_ms
                            .map(crate::lean::tui::view::duration)
                            .unwrap_or_else(|| "in progress".into())
                    ));
                    let task = task_card(self.view, &self.node.key);
                    if let Some((task, _)) = task {
                        rows.push(format!("Task: {}", task.title));
                        rows.push(if task.round == 0 {
                            "Round: unknown".into()
                        } else {
                            format!("Round: {}", task.round)
                        });
                    } else {
                        rows.push("Task: none".into());
                        rows.push("Round: none".into());
                    }
                } else {
                    rows.push("No agent turn recorded".into());
                }
            }
            1 => rows.push(agent.map_or_else(
                || "No PTY session is available".to_owned(),
                |agent| {
                    if agent.activity.is_empty() {
                        "PTY session starting".to_owned()
                    } else {
                        agent.activity.clone()
                    }
                },
            )),
            2 => {
                if self.node.key.starts_with("requirements-") {
                    rows.extend(
                        self.view
                            .requirements_summary
                            .lines()
                            .map(str::to_owned)
                            .take(8),
                    );
                    rows.push("p: continue to plan review".into());
                } else if self.node.key == crate::lean::PLANNER {
                    rows.extend(self.view.plan_doc.lines().map(str::to_owned).take(8));
                    if self.view.plan_doc.is_empty() {
                        rows.push("Plan is not available yet".into());
                    }
                    rows.push("p: open plan actions".into());
                } else if let Some((task, role)) = task_card(self.view, &self.node.key) {
                    let result = if role == Role::Reviewer {
                        task.reviewer_summary.as_ref().or(task.summary.as_ref())
                    } else {
                        task.summary.as_ref().or(task.effect.as_ref())
                    };
                    rows.push(
                        result
                            .cloned()
                            .unwrap_or_else(|| "No result reported yet".into()),
                    );
                } else if let Some(agent) = agent {
                    rows.push(if agent.activity.is_empty() {
                        "No result reported yet".into()
                    } else {
                        agent.activity.clone()
                    });
                }
            }
            3 => rows.push("Diff: press d to inspect and comment on changes".into()),
            _ => rows.push("Choose Details, PTY, Results, or Diff".into()),
        }
        for (index, row) in rows.iter().enumerate() {
            let y = self.node.y.saturating_add(4 + index as u16);
            if y + 1 >= self.node.y.saturating_add(self.node.height) {
                break;
            }
            self.canvas.text(
                self.node.x + 2,
                y,
                &truncate(row, self.inner_width()),
                self.fill.patch(self.theme.text()),
            );
        }
    }
}

fn nonempty(value: &str) -> &str {
    if value.is_empty() { "unknown" } else { value }
}
