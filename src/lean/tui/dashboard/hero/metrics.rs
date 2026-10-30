use super::super::*;

impl Details {
    pub(super) fn provider_chip(
        &self,
        view: &View,
        theme: &Theme,
        key: &str,
    ) -> Vec<Vec<Span<'static>>> {
        let Some(agent) = view
            .agents
            .get(key)
            .filter(|agent| !agent.provider.is_empty())
        else {
            return Vec::new();
        };
        vec![vec![Span::styled(
            theme.clean(&format!(
                "{} · {} · {}",
                agent.provider,
                agent.model.as_deref().unwrap_or("default model"),
                agent.effort.as_deref().unwrap_or("default effort")
            )),
            theme.dim(),
        )]]
    }

    pub(super) fn check_chips(&self, theme: &Theme) -> Vec<Vec<Span<'static>>> {
        let Some(task) = &self.task else {
            return Vec::new();
        };
        let rows = check_rows(task);
        if rows.is_empty() {
            return Vec::new();
        }
        vec![self.check_summary(theme, &rows)]
    }

    fn check_summary(
        &self,
        theme: &Theme,
        rows: &[crate::lean::tui::details::CheckRow],
    ) -> Vec<Span<'static>> {
        let settled = rows.iter().filter(|row| row.state == "settled").count();
        let issues = rows
            .iter()
            .filter(|row| matches!(row.status.as_deref(), Some("issue" | "blocked")))
            .count();
        let fill = if issues > 0 {
            theme.warning()
        } else {
            theme.status("approved")
        };
        let mut spans = vec![Span::styled("checks ", theme.dim())];
        spans.extend(gauge(theme, settled, rows.len(), 8, fill));
        spans.push(Span::styled(
            format!(" {settled}/{}", rows.len()),
            theme.text(),
        ));
        if issues > 0 {
            spans.push(Span::styled(
                format!(" {issues} issue{}", if issues == 1 { "" } else { "s" }),
                theme.error(),
            ));
        }
        spans
    }

    pub(super) fn activity_chips(&self, view: &View, theme: &Theme) -> Vec<Vec<Span<'static>>> {
        let mut chips = self.obligation_chip(view, theme);
        chips.extend(self.comment_chips(view, theme));
        chips.extend(self.usage_chips(view, theme));
        chips
    }

    fn obligation_chip(&self, view: &View, theme: &Theme) -> Vec<Vec<Span<'static>>> {
        if self
            .task
            .as_ref()
            .is_some_and(|task| !check_rows(task).is_empty())
        {
            return Vec::new();
        }
        self.row(view)
            .and_then(taskinfo::settled_total)
            .map(|(ok, total)| {
                vec![chip(
                    theme,
                    "obligations",
                    format!("{ok}/{total}"),
                    theme.text(),
                )]
            })
            .unwrap_or_default()
    }

    fn comment_chips(&self, view: &View, theme: &Theme) -> Vec<Vec<Span<'static>>> {
        let open = self
            .comments(view)
            .iter()
            .filter(|comment| comment.status == "open")
            .count();
        if open == 0 {
            Vec::new()
        } else {
            vec![chip(
                theme,
                "comments",
                format!("{open} open"),
                theme.warning(),
            )]
        }
    }

    fn usage_chips(&self, view: &View, theme: &Theme) -> Vec<Vec<Span<'static>>> {
        let agents: Vec<_> = self
            .role_keys()
            .iter()
            .filter_map(|key| view.agents.get(key))
            .collect();
        let mut chips = self.token_chip(theme, &agents);
        let busy: u64 = agents.iter().map(|agent| agent.busy_ms).sum();
        if busy > 0 {
            chips.push(chip(theme, "busy", duration(busy), theme.text()));
        }
        if let Some(row) = self.row(view).filter(|row| row.integration.is_some()) {
            chips.push(chip(
                theme,
                "landing",
                taskinfo::integration_label(row),
                theme.text(),
            ));
        }
        chips
    }

    fn token_chip(
        &self,
        theme: &Theme,
        agents: &[&super::super::super::view::Agent],
    ) -> Vec<Vec<Span<'static>>> {
        if !agents.iter().any(|agent| agent.reported > 0) {
            return Vec::new();
        }
        let sum = |field: fn(&super::super::super::view::Agent) -> Option<u64>| {
            agents.iter().filter_map(|agent| field(agent)).sum::<u64>()
        };
        vec![chip(
            theme,
            "tokens",
            format!(
                "{} in / {} out",
                compact(sum(|agent| agent.tokens_in)),
                compact(sum(|agent| agent.tokens_out))
            ),
            theme.text(),
        )]
    }
}
