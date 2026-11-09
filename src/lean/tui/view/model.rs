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
    /// Turns whose prompt and answer sizes are known, and the tokens they
    /// suggest (about four bytes each). Files the agent read are not in it.
    pub estimated: u32,
    pub estimated_in: u64,
    pub estimated_out: u64,
}

/// How much an agent's turns cost, and how that is known.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Usage {
    /// What the CLI reported; `partial` when some turns reported nothing.
    Reported {
        tokens_in: u64,
        tokens_out: u64,
        partial: bool,
    },
    /// Sized from the prompts antislop sent and the answers it got back: a
    /// lower bound, since what the agent read on its own is not counted.
    Estimated {
        tokens_in: u64,
        tokens_out: u64,
    },
    Unavailable,
}

impl Usage {
    /// The usage of several agents together: reported when any reported.
    pub fn total<'a>(agents: impl IntoIterator<Item = &'a Agent>) -> Self {
        let (mut reported, mut estimated) = (None, None);
        for usage in agents.into_iter().map(Agent::usage) {
            match usage {
                Self::Reported {
                    tokens_in,
                    tokens_out,
                    partial,
                } => {
                    let (a, b, p) = reported.get_or_insert((0, 0, false));
                    (*a, *b, *p) = (*a + tokens_in, *b + tokens_out, *p || partial);
                }
                Self::Estimated {
                    tokens_in,
                    tokens_out,
                } => {
                    let (a, b) = estimated.get_or_insert((0, 0));
                    (*a, *b) = (*a + tokens_in, *b + tokens_out);
                }
                Self::Unavailable => {}
            }
        }
        match (reported, estimated) {
            (Some((tokens_in, tokens_out, partial)), estimated) => Self::Reported {
                tokens_in,
                tokens_out,
                partial: partial || estimated.is_some(),
            },
            (None, Some((tokens_in, tokens_out))) => Self::Estimated {
                tokens_in,
                tokens_out,
            },
            (None, None) => Self::Unavailable,
        }
    }

    /// `12.3k in / 4.0k out`, marked when partial or estimated; `None` when
    /// nothing is known.
    pub fn short(self) -> Option<String> {
        match self {
            Self::Reported {
                tokens_in,
                tokens_out,
                partial,
            } => Some(format!(
                "{} in / {} out{}",
                compact(tokens_in),
                compact(tokens_out),
                if partial { " (partial)" } else { "" }
            )),
            Self::Estimated {
                tokens_in,
                tokens_out,
            } => Some(format!(
                "~{} in / ~{} out (estimated)",
                compact(tokens_in),
                compact(tokens_out)
            )),
            Self::Unavailable => None,
        }
    }
}

/// A count in at most five characters: `999`, `12.3k`, `1.2M`.
pub fn compact(n: u64) -> String {
    match n {
        0..1000 => n.to_string(),
        1000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

impl Agent {
    /// What this agent's turns cost, as reported or else as estimated.
    pub fn usage(&self) -> Usage {
        if self.reported > 0 {
            return Usage::Reported {
                tokens_in: self.tokens_in.unwrap_or(0),
                tokens_out: self.tokens_out.unwrap_or(0),
                partial: self.reported < self.turns
                    || self.tokens_in.is_none()
                    || self.tokens_out.is_none(),
            };
        }
        if self.estimated > 0 {
            return Usage::Estimated {
                tokens_in: self.estimated_in,
                tokens_out: self.estimated_out,
            };
        }
        Usage::Unavailable
    }

    /// Token use in words: reported, estimated with what it leaves out, or
    /// unavailable. A turn nothing is known for is unknown, never zero.
    pub fn tokens(&self) -> String {
        match self.usage() {
            Usage::Reported { .. } => {
                let whole = if self.reported >= self.turns {
                    String::new()
                } else {
                    format!(
                        " · reported in {} of {} turns, rest unknown",
                        self.reported, self.turns
                    )
                };
                let side =
                    |sum: Option<u64>| sum.map_or("unknown".to_owned(), |sum| sum.to_string());
                format!(
                    "in {} / out {}{whole}",
                    side(self.tokens_in),
                    side(self.tokens_out)
                )
            }
            Usage::Estimated {
                tokens_in,
                tokens_out,
            } => format!(
                "estimated ~{tokens_in} in / ~{tokens_out} out from prompts and answers \
                 (the CLI did not report usage; files the agent read are not counted)"
            ),
            Usage::Unavailable => "unavailable (CLI did not report them)".into(),
        }
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
    /// Every task approved; the final audit runs.
    Audit,
    Done,
}

impl Stage {
    /// The steps every run goes through, in order.
    pub const STEPS: [Self; 6] = [
        Self::Interview,
        Self::Planning,
        Self::Review,
        Self::Executing,
        Self::Audit,
        Self::Done,
    ];

    /// Which of [`Self::STEPS`] this stage is at; a blocked run is stuck in
    /// execution and a fresh one has not left the interview.
    pub fn step(self) -> usize {
        match self {
            Self::Fresh | Self::Interview => 0,
            Self::Planning => 1,
            Self::Review => 2,
            Self::Executing | Self::Blocked => 3,
            Self::Audit => 4,
            Self::Done => 5,
        }
    }

    /// One mark per step (done, current, still to come), then this stage's
    /// name: `✓✓●○○○ plan review`.
    pub fn strip(self, glyphs: &crate::lean::tui::theme::Glyphs) -> String {
        let step = self.step();
        let marks = Self::STEPS
            .iter()
            .enumerate()
            .map(|(index, _)| match index.cmp(&step) {
                std::cmp::Ordering::Less => glyphs.done,
                std::cmp::Ordering::Equal if self == Self::Done => glyphs.done,
                std::cmp::Ordering::Equal => glyphs.active,
                std::cmp::Ordering::Greater => glyphs.queued,
            })
            .collect::<String>();
        let name = if self == Self::Fresh {
            "interview"
        } else {
            self.label()
        };
        format!("{marks} {name}")
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Fresh => "new",
            Self::Interview => "interview",
            Self::Planning => "planning",
            Self::Review => "plan review",
            Self::Executing => "executing",
            Self::Blocked => "blocked",
            Self::Audit => "audit",
            Self::Done => "done",
        }
    }
}
