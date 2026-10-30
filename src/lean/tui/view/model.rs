use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Agent {
    /// Actual branch in the latest turn's working directory.
    pub branch: Option<String>,
    pub provider: String,
    /// The model and native effort of its latest turn, when configured.
    pub model: Option<String>,
    pub effort: Option<String>,
    pub running: bool,
    /// Process ID of the running or latest turn, if reported.
    pub pid: Option<u32>,
    /// The current tool call while running, else the last outcome.
    pub activity: String,
    /// Finished turns, and how many of them the CLI reported tokens for.
    pub turns: u32,
    pub reported: u32,
    /// Time spent in finished turns whose start and end times are known,
    /// and the last such turn's.
    pub busy_ms: u64,
    pub last_ms: Option<u64>,
    /// When the running turn started, if known.
    pub started_ms: Option<i64>,
    /// Sums of what was reported; `None` while nothing was.
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
}

impl Agent {
    /// Token use as the CLI reported it; a turn it reported nothing for is
    /// unknown, never zero.
    pub fn tokens(&self) -> String {
        if self.reported == 0 {
            return "unavailable (CLI did not report them)".into();
        }
        let whole = if self.reported >= self.turns {
            String::new()
        } else {
            format!(
                " · reported in {} of {} turns, rest unknown",
                self.reported, self.turns
            )
        };
        let side = |sum: Option<u64>| sum.map_or("unknown".to_owned(), |sum| sum.to_string());
        format!(
            "in {} / out {}{whole}",
            side(self.tokens_in),
            side(self.tokens_out)
        )
    }
}

/// Messages that passed between two agents.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub count: u32,
    pub last: Instant,
}

/// A comment as the daemon stores it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Comment {
    pub id: u64,
    pub plan_id: Option<String>,
    pub target: String,
    pub text: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct CommentProgress {
    pub state: String,
    pub error: Option<String>,
}

/// The conversation's step, shown in the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Fresh,
    Interview,
    Planning,
    Review,
    Executing,
    Blocked,
    Done,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fresh => "new",
            Self::Interview => "interview",
            Self::Planning => "planning",
            Self::Review => "plan review",
            Self::Executing => "executing",
            Self::Blocked => "blocked",
            Self::Done => "done",
        }
    }
}
