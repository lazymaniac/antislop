use crate::config::ProviderKind;

/// Stable causes in persisted diagnostics. Raw provider text follows the marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopCause {
    ProviderLimit,
    LoginRequired,
    UnexpectedDialog,
    Timeout,
    ProcessExit,
    Cancelled,
}
impl StopCause {
    pub(crate) fn marker(self) -> &'static str {
        match self {
            Self::ProviderLimit => "provider_limit",
            Self::LoginRequired => "login_required",
            Self::UnexpectedDialog => "unexpected_dialog",
            Self::Timeout => "timeout",
            Self::ProcessExit => "process_exit",
            Self::Cancelled => "cancelled",
        }
    }
    pub(crate) fn diagnostic(self, detail: impl std::fmt::Display) -> crate::Error {
        super::invalid(format!("[{}] {detail}", self.marker()))
    }
}

/// Recognize operational UI lines, not an agent's discussion of limits or permissions.
/// Prefix checks deliberately reject quoted reports, code and prose around a marker.
pub(crate) fn screen_stop(provider: ProviderKind, screen: &str) -> Option<StopCause> {
    for line in screen.lines() {
        let line = line
            .trim()
            .trim_start_matches(['│', '┃', '╭', '╰', '❯', '⎿'])
            .trim();
        let line = line.to_ascii_lowercase();
        let limit = match provider {
            ProviderKind::Claude => {
                line.starts_with("you've hit your 5-hour limit")
                    || line.starts_with("you’ve hit your 5-hour limit")
                    || line.starts_with("you've hit your usage limit")
                    || line.starts_with("you've hit your weekly limit")
            }
            ProviderKind::Codex => {
                line.starts_with("you've hit your usage limit")
                    || line.starts_with("you have hit your usage limit")
            }
            _ => {
                line.starts_with("error: quota exceeded")
                    || line.starts_with("error: rate limit exceeded")
                    || line.starts_with("error: 429")
            }
        };
        if limit {
            return Some(StopCause::ProviderLimit);
        }
        if line.starts_with("not logged in")
            || line.starts_with("error: not logged in")
            || line.starts_with("authentication required")
            || line.starts_with("please sign in to continue")
            || line.starts_with("select login method")
            || line.starts_with("please log in to ")
            || line.starts_with("how would you like to authenticate")
        {
            return Some(StopCause::LoginRequired);
        }
        if line.starts_with("do you want to allow ")
            || line.starts_with("would you like to run the following command?")
            || line.starts_with("do you trust the files in this folder?")
            || line.starts_with("permission required")
        {
            return Some(StopCause::UnexpectedDialog);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn operational_limits_are_distinct_from_agent_prose() {
        assert_eq!(
            screen_stop(
                ProviderKind::Claude,
                "You've hit your 5-hour limit · resets 4pm (Europe/Warsaw)"
            ),
            Some(StopCause::ProviderLimit)
        );
        assert_eq!(
            screen_stop(
                ProviderKind::Claude,
                "⎿  You've hit your 5-hour limit · resets 4pm"
            ),
            Some(StopCause::ProviderLimit)
        );
        for text in [
            "The code mentions permission and rate limits.",
            "The error was: You've hit your 5-hour limit",
            "`You've hit your 5-hour limit`",
            "No permission required for this command.",
        ] {
            assert_eq!(screen_stop(ProviderKind::Claude, text), None);
        }
        assert_eq!(
            screen_stop(ProviderKind::Claude, "Do you want to allow Bash?"),
            Some(StopCause::UnexpectedDialog)
        );
    }
}
