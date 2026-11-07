use crate::config::ProviderKind;

const BRAILLE_SPINNERS: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Default)]
pub(super) struct Tracker {
    saw_work: bool,
    reminded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScreenState {
    Working,
    Ready,
    Unknown,
}

impl Tracker {
    /// True once for each ready state that follows recognized work.
    pub(super) fn observe(
        &mut self,
        provider: ProviderKind,
        screen: &str,
        approval_open: bool,
        paused: bool,
    ) -> bool {
        if approval_open {
            return false;
        }
        match classify(provider, screen) {
            ScreenState::Working => {
                self.saw_work = true;
                self.reminded = false;
                false
            }
            ScreenState::Ready if self.saw_work && !self.reminded && !paused => {
                self.reminded = true;
                true
            }
            ScreenState::Ready | ScreenState::Unknown => false,
        }
    }
}

fn classify(provider: ProviderKind, screen: &str) -> ScreenState {
    let lines: Vec<&str> = screen
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() {
        return ScreenState::Unknown;
    }
    // Provider status and animation rows sit beside the input line. Older
    // assistant messages often remain visible above it and are not activity.
    let recent = lines.iter().rev().take(2).copied().collect::<Vec<_>>();
    if recent.iter().any(|line| is_work_signal(provider, line)) {
        return ScreenState::Working;
    }
    if is_ready_prompt(provider, lines[lines.len() - 1]) {
        ScreenState::Ready
    } else {
        ScreenState::Unknown
    }
}

fn is_ready_prompt(provider: ProviderKind, line: &str) -> bool {
    let line = line.trim();
    match provider {
        ProviderKind::Claude | ProviderKind::Agy | ProviderKind::Opencode => line == "❯",
        ProviderKind::Codex => line == "›",
        ProviderKind::Gemini => line == ">",
    }
}

fn is_work_signal(provider: ProviderKind, line: &str) -> bool {
    let text_signal = is_status_label(provider, line);
    let animation = match provider {
        ProviderKind::Claude => {
            // Claude uses ⏺ and ⎿ to prefix both active tool calls and their
            // completed output. Only explicit status text or actual spinners
            // can distinguish ongoing work from a finished response.
            ["✻", "✶", "✢", "◐", "◓", "◑", "◒", "● "]
                .iter()
                .any(|mark| line.contains(mark))
        }
        ProviderKind::Agy => ["✻", "✶", "✢", "⏺", "⎿", "◐", "◓", "◑", "◒", "● "]
            .iter()
            .any(|mark| line.contains(mark)),
        ProviderKind::Codex => {
            ["• ", "◦"].iter().any(|mark| line.contains(mark)) || has_braille_spinner(line)
        }
        ProviderKind::Gemini => {
            ["✦", "✧"].iter().any(|mark| line.contains(mark)) || has_braille_spinner(line)
        }
        ProviderKind::Opencode => {
            has_braille_spinner(line) || ["◐", "◓", "◑", "◒"].iter().any(|mark| line.contains(mark))
        }
    };
    text_signal || animation
}

fn has_braille_spinner(line: &str) -> bool {
    BRAILLE_SPINNERS.iter().any(|mark| line.contains(mark))
}

fn is_status_label(provider: ProviderKind, line: &str) -> bool {
    let line = line.trim();
    let prefixes: &[&str] = match provider {
        ProviderKind::Claude => &["⏺ "],
        ProviderKind::Agy => &["✻ ", "✶ ", "✢ ", "⏺ "],
        ProviderKind::Codex => &["• ", "◦ "],
        ProviderKind::Gemini => &["✦ ", "✧ "],
        ProviderKind::Opencode => &[],
    };
    const LABELS: &[&str] = &[
        "thinking",
        "working",
        "generating",
        "planning",
        "searching",
        "running",
        "executing",
        "reading",
        "writing",
        "tool call",
        "tool use",
    ];
    prefixes.iter().any(|prefix| {
        line.strip_prefix(prefix).is_some_and(|status| {
            let status = status.trim();
            LABELS
                .iter()
                .any(|known| status.eq_ignore_ascii_case(known))
                || status.to_ascii_lowercase().starts_with("waiting for ")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(name: &str) -> ProviderKind {
        match name {
            "agy" => ProviderKind::Agy,
            "claude" => ProviderKind::Claude,
            "codex" => ProviderKind::Codex,
            "gemini" => ProviderKind::Gemini,
            _ => ProviderKind::Opencode,
        }
    }

    #[test]
    fn every_supported_cli_requires_work_then_its_ready_prompt() {
        let screens = [
            ("agy", "✻ Thinking\n❯"),
            ("claude", "⏺ Tool use\n❯"),
            ("codex", "• Working\n›"),
            ("gemini", "✦ Thinking\n>"),
            ("opencode", "⠋ Running\n❯"),
        ];
        for (name, screen) in screens {
            let mut tracker = Tracker::default();
            assert!(
                !tracker.observe(provider(name), "❯", false, false),
                "{name} initial prompt"
            );
            assert!(
                !tracker.observe(provider(name), screen, false, false),
                "{name} work"
            );
            assert!(
                tracker.observe(provider(name), screen.lines().last().unwrap(), false, false),
                "{name} ready"
            );
        }
    }

    #[test]
    fn braille_spinner_signals_work_for_codex_gemini_and_opencode() {
        for (provider, prompt) in [
            (ProviderKind::Codex, "›"),
            (ProviderKind::Gemini, ">"),
            (ProviderKind::Opencode, "❯"),
        ] {
            let mut tracker = Tracker::default();
            assert!(!tracker.observe(provider, "⠋", false, false));
            assert!(tracker.observe(provider, prompt, false, false));
        }
    }

    #[test]
    fn silence_approval_and_unknown_screens_do_not_wake_or_reset() {
        let mut tracker = Tracker::default();
        assert!(!tracker.observe(ProviderKind::Claude, "", false, false));
        assert!(!tracker.observe(ProviderKind::Claude, "❯", false, false));
        assert!(!tracker.observe(ProviderKind::Claude, "✻ Thinking", false, false));
        assert!(!tracker.observe(ProviderKind::Claude, "", false, false));
        assert!(!tracker.observe(ProviderKind::Claude, "unknown screen", false, false));
        assert!(tracker.observe(ProviderKind::Claude, "❯", false, false));
        assert!(!tracker.observe(ProviderKind::Claude, "✻ Thinking", true, false));
        assert!(!tracker.observe(ProviderKind::Claude, "✻ Thinking", false, false));
        assert!(tracker.observe(ProviderKind::Claude, "❯", false, false));
        assert!(!tracker.observe(ProviderKind::Claude, "unknown screen", false, false));
    }

    #[test]
    fn old_response_markers_do_not_hide_a_ready_prompt() {
        let screen = format!("{}\n{}", "⏺ previous message\ntext\n".repeat(4), "❯");
        let mut tracker = Tracker::default();
        assert!(!tracker.observe(ProviderKind::Claude, "✻ Thinking", false, false));
        assert!(tracker.observe(ProviderKind::Claude, &screen, false, false));
    }

    #[test]
    fn completed_claude_message_marker_does_not_hide_ready_prompt() {
        let mut tracker = Tracker::default();
        assert!(!tracker.observe(ProviderKind::Claude, "✻ Thinking", false, false));
        assert!(tracker.observe(ProviderKind::Claude, "⏺ Done\n❯", false, false));
    }

    #[test]
    fn ordinary_completed_answer_words_do_not_hide_ready_prompt() {
        let mut tracker = Tracker::default();
        assert!(!tracker.observe(ProviderKind::Claude, "✻ Thinking", false, false));
        assert!(tracker.observe(
            ProviderKind::Claude,
            "I finished writing the file\n❯",
            false,
            false
        ));
    }

    #[test]
    fn claude_tool_use_text_still_counts_as_work() {
        let mut tracker = Tracker::default();
        assert!(!tracker.observe(ProviderKind::Claude, "✻ Thinking", false, false));
        assert!(!tracker.observe(ProviderKind::Claude, "⏺ Tool use\n❯", false, false));
        assert!(tracker.observe(ProviderKind::Claude, "❯", false, false));
    }

    #[test]
    fn one_reminder_per_ready_state_and_work_rearms_it() {
        let mut tracker = Tracker::default();
        assert!(!tracker.observe(ProviderKind::Codex, "• Working", false, false));
        assert!(tracker.observe(ProviderKind::Codex, "›", false, false));
        assert!(!tracker.observe(ProviderKind::Codex, "›", false, false));
        assert!(!tracker.observe(ProviderKind::Codex, "◦ Running", false, false));
        assert!(tracker.observe(ProviderKind::Codex, "›", false, false));
    }

    #[test]
    fn pause_suppresses_reminders_until_resumed() {
        let mut tracker = Tracker::default();
        assert!(!tracker.observe(ProviderKind::Claude, "✻ Thinking", false, false));
        assert!(!tracker.observe(ProviderKind::Claude, "❯", false, true));
        assert!(tracker.observe(ProviderKind::Claude, "❯", false, false));
    }

    #[test]
    fn approval_dialog_suppresses_a_ready_reminder_without_consuming_it() {
        let mut tracker = Tracker::default();
        assert!(!tracker.observe(ProviderKind::Claude, "✻ Thinking", false, false));
        assert!(!tracker.observe(ProviderKind::Claude, "❯", true, false));
        assert!(tracker.observe(ProviderKind::Claude, "❯", false, false));
        assert!(!tracker.observe(ProviderKind::Claude, "❯", false, false));
    }
}
