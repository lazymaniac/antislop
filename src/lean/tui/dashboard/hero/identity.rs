use super::super::*;

impl Details {
    pub(super) fn hero_key(&self) -> String {
        self.session_keys()
            .get(self.active_panel)
            .cloned()
            .unwrap_or_else(|| self.key.clone())
    }

    pub(super) fn identity_line(&self, view: &View, theme: &Theme, key: &str) -> Line<'static> {
        let Some(row) = self.row(view) else {
            return Line::from(vec![
                Span::raw(" "),
                Span::styled(agent_name(key), theme.title()),
            ]);
        };
        let mut head = self.identity_status(theme, row);
        head.extend(self.identity_title(theme, key, row));
        Line::from(head)
    }

    fn identity_status(
        &self,
        theme: &Theme,
        row: &super::super::super::view::TaskRow,
    ) -> Vec<Span<'static>> {
        let state = status_label(&row.status).to_uppercase();
        let round = taskinfo::round_shown(row.round).map_or_default(|round| format!(" {round}"));
        vec![
            Span::raw(" "),
            Span::styled(
                format!("{} {state}{round}", theme.status_glyph(&row.status)),
                theme.status(&row.status).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {} ", theme.glyphs().vertical), theme.faint()),
        ]
    }

    fn identity_title(
        &self,
        theme: &Theme,
        key: &str,
        row: &super::super::super::view::TaskRow,
    ) -> Vec<Span<'static>> {
        vec![
            identity_id(theme, key, row),
            identity_summary(theme, row),
            identity_kind(theme, row),
        ]
    }
}

fn identity_id(
    theme: &Theme,
    key: &str,
    row: &super::super::super::view::TaskRow,
) -> Span<'static> {
    Span::styled(
        theme.clean(&row.id) + " ",
        theme.role_accent(key, &row.kind),
    )
}

fn identity_summary(theme: &Theme, row: &super::super::super::view::TaskRow) -> Span<'static> {
    let summary = theme
        .clean(row.summary.as_deref().unwrap_or(&row.title))
        .replace(['\n', '\r', '\t'], " ");
    Span::styled(summary, theme.title())
}

fn identity_kind(theme: &Theme, row: &super::super::super::view::TaskRow) -> Span<'static> {
    let access = if row.writes {
        "writes files"
    } else {
        "read-only"
    };
    Span::styled(
        format!("  {} · {access}", theme.clean(&row.kind)),
        theme.dim(),
    )
}
