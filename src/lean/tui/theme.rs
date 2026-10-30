//! Visual tokens shared by every panel.
//! Cards are separated by background and accents, not heavy borders;
//! NO_COLOR keeps only modifiers, and ASCII mode keeps only ASCII glyphs.

use super::sanitize::sanitize;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

mod tokens;
pub use tokens::{ASCII, Glyphs, Theme, Tone, UNICODE, ayu, tone};

mod ascii;
pub(super) use ascii::ascii_character;
mod palette;

/// Word wrap by display width, respecting grapheme cluster boundaries.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = vec![];
    let sanitized = sanitize(text);
    for paragraph in sanitized.split('\n') {
        let mut line = String::new();
        let mut used = 0;
        for word in paragraph.split_inclusive(' ') {
            let w = word.width();
            if used > 0 && used + w.min(width) > width {
                lines.push(std::mem::take(&mut line));
                used = 0;
            }
            for g in word.graphemes(true) {
                let gw = g.width();
                if used + gw > width && used > 0 {
                    lines.push(std::mem::take(&mut line));
                    used = 0;
                }
                line.push_str(g);
                used += gw;
            }
        }
        lines.push(line);
    }
    lines
}

mod rows;
pub use rows::{CardProps, Row, card, draw_rows, scrollbar, section, split_row, title_row};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_tones_and_glyphs() {
        assert_eq!(tone("running"), Tone::Active);
        assert_eq!(tone("verified"), Tone::Done);
        assert_eq!(tone("failed"), Tone::Failed);
        assert_eq!(tone("paused"), Tone::Waiting);
        assert_eq!(tone("queued"), Tone::Queued);

        let t_uni = Theme::new(true, false);
        assert_eq!(t_uni.glyphs().done, "✓");
        assert_eq!(t_uni.status_glyph("completed"), "✓");

        let t_ascii = Theme::new(false, true);
        assert_eq!(t_ascii.glyphs().done, "x");
        assert_eq!(t_ascii.status_glyph("completed"), "x");
    }

    #[test]
    fn theme_grapheme_wrapping_and_fit() {
        let t = Theme::new(true, false);
        let wrapped = wrap("cat on mat", 7);
        assert_eq!(wrapped, ["cat on ", "mat"]);

        let fit_res = t.fit("verylongsampletext", 10);
        assert!(fit_res.ends_with('…'));
        assert_eq!(fit_res.width(), 10);
    }
}

#[cfg(test)]
mod role_tests {
    use super::*;

    #[test]
    fn role_icons_and_spinner_frames_are_one_column_wide() {
        for glyphs in [&UNICODE, &ASCII] {
            let icons = [
                glyphs.waiting,
                glyphs.problem,
                glyphs.orchestrator,
                glyphs.plan_verifier,
                glyphs.auditor,
                glyphs.worker,
                glyphs.verifier,
            ];
            for icon in icons.iter().chain(glyphs.spinner) {
                assert_eq!(icon.width(), 1, "{icon}");
            }
        }
        assert!(ASCII.spinner.iter().all(|frame| frame.is_ascii()));
    }

    #[test]
    fn spinner_wraps_and_roles_get_distinct_icons() {
        let theme = Theme::new(true, false);
        let frames = UNICODE.spinner.len() as u64;
        assert_eq!(theme.spinner(0), theme.spinner(frames));
        let keys = [
            "orchestrator",
            "plan_verifier",
            "auditor",
            "worker:t1",
            "verifier:t1",
        ];
        let icons: std::collections::BTreeSet<_> =
            keys.iter().map(|key| theme.role_icon(key)).collect();
        assert_eq!(icons.len(), keys.len());
    }
}
