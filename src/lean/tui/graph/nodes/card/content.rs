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

/// The title is drawn in `inner - 2` columns; a folded card marks itself
/// with ` [+]`, or ` +` when that would leave its name under 8 columns.
pub(super) fn card_title(name: &str, inner: usize, collapsed: bool) -> String {
    let room = inner.saturating_sub(2);
    if !collapsed {
        return super::super::super::super::view::truncate(name, room);
    }
    let mark = if room >= 12 { " [+]" } else { " +" };
    let name = super::super::super::super::view::truncate(name, room.saturating_sub(mark.len()));
    format!("{name}{mark}")
}
