use super::super::super::super::theme::Theme;
use super::super::super::Health;
use ratatui::style::Style;

pub(super) fn status_style(theme: &Theme, health: Health) -> (String, Style) {
    let (icon, tone) = match health {
        Health::Working => ("", theme.status("working")),
        Health::Problem => (theme.glyphs().problem, theme.error()),
        Health::Waiting => (theme.glyphs().waiting, theme.warning()),
        Health::Done => (theme.glyphs().done, theme.status("approved")),
        Health::Idle => (theme.glyphs().queued, theme.dim()),
    };
    (icon.to_owned(), tone)
}

pub(super) fn card_title(name: &str, inner: usize, collapsed: bool) -> String {
    if collapsed {
        format!(
            "{} [+]",
            super::super::super::super::view::truncate(name, inner.saturating_sub(7))
        )
    } else {
        super::super::super::super::view::truncate(name, inner.saturating_sub(2))
    }
}
