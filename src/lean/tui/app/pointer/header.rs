use super::*;
use crate::lean::tui::view::{
    CANCEL, CANCEL_ICON, CANCEL_SHORT, CONTINUE, CONTINUE_ICON, CONTINUE_SHORT, NEW, NEW_ICON,
    NEW_SHORT, PAUSE, PAUSE_ICON, PAUSE_SHORT, PLAN, RESUME, RESUME_ICON, RESUME_SHORT,
};

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
            "[Mode: autonomous]" | "[auto]" => self.run_workflow(Command::Supervised),
            "[Mode: supervised]" | "[sup]" => self.run_workflow(Command::Autonomous),
            PAUSE | PAUSE_SHORT | PAUSE_ICON if controls.pause => {
                self.run_workflow(Command::Pause);
            }
            CONTINUE | CONTINUE_SHORT | CONTINUE_ICON if controls.resume_paused => {
                self.run_workflow(Command::Continue);
            }
            RESUME | RESUME_SHORT | RESUME_ICON if controls.resume_plan => {
                self.run_workflow(Command::Resume);
            }
            CANCEL | CANCEL_SHORT | CANCEL_ICON if controls.cancel => {
                self.run_workflow(Command::Cancel);
            }
            PLAN => self.run_workflow(Command::Plan),
            NEW | NEW_SHORT | NEW_ICON => self.run_workflow(Command::NewConversation),
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
