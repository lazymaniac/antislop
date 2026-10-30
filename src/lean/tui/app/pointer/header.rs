use super::*;

impl App {
    pub(super) fn click_panel(&mut self, col: u16, area: Rect) {
        let chat_width = (area.width / 3).max(34);
        if col < area.x + chat_width {
            self.focus = Focus::Chat;
        } else if col > area.x + chat_width && self.focus == Focus::Chat {
            self.focus = Focus::Graph;
        }
    }

    pub(super) fn activate_header_token(
        &mut self,
        token: &str,
        controls: crate::lean::tui::view::Controls,
    ) {
        match token {
            "[Mode: autonomous]" | "[auto]" => self.command("/mode supervised"),
            "[Mode: supervised]" | "[sup]" => self.command("/mode autonomous"),
            "[/pause]" | "[‖]" if controls.pause => self.command("/pause"),
            "[/continue]" | "[▶]" if controls.resume_paused => self.command("/continue"),
            "[/resume]" | "[↻]" if controls.resume_plan => self.command("/resume"),
            "[/cancel]" | "[✕]" if controls.cancel => self.command("/cancel"),
            "[Plan]" => self.command("/plan"),
            "[Unblock]" | "[Task actions]" => self.command("/unblock"),
            "[Chat]" => self.focus = Focus::Chat,
            "[Team]" => self.focus = Focus::Graph,
            _ => {}
        }
    }
}

pub(super) fn token_at(text: &str, target_col: usize) -> Option<String> {
    let mut column = 0usize;
    for (idx, grapheme) in unicode_segmentation::UnicodeSegmentation::grapheme_indices(text, true) {
        let width = unicode_width::UnicodeWidthStr::width(grapheme);
        if target_col >= column && target_col < column + width {
            let open = text[..idx + grapheme.len()].rfind('[')?;
            let close = text[idx..].find(']')?;
            return Some(text[open..=idx + close].to_owned());
        }
        column += width;
    }
    None
}
