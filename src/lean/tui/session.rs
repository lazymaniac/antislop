//! An agent's session as the blocks its CLI printed, drawn the way that CLI
//! draws them: commands and their output, reasoning, answers and prompts.
//! Provider output is data; every piece of text is cleaned before display.

use super::sanitize::sanitize;
use ratatui::style::{Color, Modifier, Style};

/// Longest output shown for one tool call before the rest is folded.
const OUTPUT_LINES: usize = 12;
/// Longest prompt shown before it is folded.
const PROMPT_LINES: usize = 6;

/// One thing that happened in an agent's turns.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// A turn began: the CLI, model and effort it runs with.
    Turn(String),
    /// What the agent was asked.
    Prompt(String),
    /// What the user wrote to the agent.
    User(String),
    Reasoning(String),
    Text(String),
    Tool {
        name: String,
        detail: String,
    },
    Output {
        name: String,
        text: String,
        exit_code: Option<i64>,
    },
    /// A message to or from another agent.
    Mail(String),
    Tokens {
        input: Option<u64>,
        output: Option<u64>,
    },
    /// Process and session bookkeeping.
    Note(String),
    /// The turn ended; `ok` tells whether it succeeded.
    End {
        ok: bool,
        text: String,
    },
}

/// How a CLI marks the parts of its transcript.
struct Look {
    tool: &'static str,
    /// Wraps the call as the CLI shows it.
    call: fn(&str, &str) -> String,
    first_output: &'static str,
    next_output: &'static str,
    answer: &'static str,
    prompt: &'static str,
    thinking: &'static str,
}

fn codex_call(name: &str, detail: &str) -> String {
    match name {
        "shell" => format!("Ran {detail}"),
        "edit" => format!("Edited {detail}"),
        _ => format!("Called {name} {detail}"),
    }
}

fn claude_call(name: &str, detail: &str) -> String {
    format!("{name}({detail})")
}

fn plain_call(name: &str, detail: &str) -> String {
    format!("{name} {detail}")
}

fn look(provider: &str) -> Look {
    match provider {
        "codex" => Look {
            tool: "• ",
            call: codex_call,
            first_output: "  └ ",
            next_output: "    ",
            answer: "• ",
            prompt: "› ",
            thinking: "• ",
        },
        "claude" => Look {
            tool: "⏺ ",
            call: claude_call,
            first_output: "  ⎿  ",
            next_output: "     ",
            answer: "⏺ ",
            prompt: "> ",
            thinking: "✻ ",
        },
        "gemini" => Look {
            tool: "✓ ",
            call: plain_call,
            first_output: "  ",
            next_output: "  ",
            answer: "✦ ",
            prompt: "> ",
            thinking: "✦ ",
        },
        _ => Look {
            tool: "→ ",
            call: plain_call,
            first_output: "  ",
            next_output: "  ",
            answer: "",
            prompt: "> ",
            thinking: "~ ",
        },
    }
}

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// Splits `text` into cleaned lines, trimming trailing blanks.
fn lines_of(text: &str) -> Vec<String> {
    let clean = sanitize(text);
    let mut lines: Vec<String> = clean.lines().map(|l| l.trim_end().to_owned()).collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

/// Pushes `text` as a block: `first` leads the first line, `next` the rest;
/// more than `limit` lines are folded with a count.
fn block(
    out: &mut Vec<(Style, String)>,
    text: &str,
    first: &str,
    next: &str,
    limit: usize,
    style: Style,
) {
    let lines = lines_of(text);
    for (n, line) in lines.iter().take(limit).enumerate() {
        let lead = if n == 0 { first } else { next };
        out.push((style, format!("{lead}{line}")));
    }
    if lines.len() > limit {
        out.push((
            dim(),
            format!("{next}… +{} more lines", lines.len() - limit),
        ));
    }
}

/// The blocks of one agent as styled lines, in the style of its `provider`.
pub fn render(provider: &str, blocks: &[Block]) -> Vec<(Style, String)> {
    let look = look(provider);
    let mut out = Vec::new();
    for item in blocks {
        match item {
            Block::Turn(header) => {
                if !out.is_empty() {
                    out.push((Style::default(), String::new()));
                }
                out.push((
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                    format!("── {}", sanitize(header)),
                ));
            }
            Block::Prompt(text) => {
                block(&mut out, text, look.prompt, "  ", PROMPT_LINES, dim());
            }
            Block::User(text) => {
                block(
                    &mut out,
                    text,
                    look.prompt,
                    "  ",
                    PROMPT_LINES,
                    Style::default().fg(Color::Cyan),
                );
            }
            Block::Reasoning(text) => {
                block(
                    &mut out,
                    text,
                    look.thinking,
                    "  ",
                    OUTPUT_LINES,
                    dim().add_modifier(Modifier::ITALIC),
                );
            }
            Block::Text(text) => {
                block(
                    &mut out,
                    text,
                    look.answer,
                    "  ",
                    usize::MAX,
                    Style::default(),
                );
            }
            Block::Tool { name, detail } => {
                let call = (look.call)(&sanitize(name), &sanitize(detail));
                out.push((
                    Style::default().add_modifier(Modifier::BOLD),
                    format!("{}{}", look.tool, call.trim_end()),
                ));
            }
            Block::Output {
                name: _,
                text,
                exit_code,
            } => {
                let failed = exit_code.is_some_and(|code| code != 0);
                let style = if failed {
                    Style::default().fg(Color::Red)
                } else {
                    dim()
                };
                if text.trim().is_empty() {
                    out.push((style, format!("{}(no output)", look.first_output)));
                } else {
                    block(
                        &mut out,
                        text,
                        look.first_output,
                        look.next_output,
                        OUTPUT_LINES,
                        style,
                    );
                }
                if let Some(code) = exit_code.filter(|code| *code != 0) {
                    out.push((style, format!("{}exit code {code}", look.next_output)));
                }
            }
            Block::Mail(text) => {
                block(
                    &mut out,
                    text,
                    "✉ ",
                    "  ",
                    PROMPT_LINES,
                    Style::default().fg(Color::Magenta),
                );
            }
            Block::Tokens { input, output } => out.push((
                dim(),
                format!(
                    "  tokens in {} · out {}",
                    input.map_or("?".to_owned(), |n| n.to_string()),
                    output.map_or("?".to_owned(), |n| n.to_string())
                ),
            )),
            Block::Note(text) => out.push((dim(), format!("  {}", sanitize(text)))),
            Block::End { ok, text } => out.push((
                if *ok {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::Red)
                },
                format!("── {}", sanitize(text)),
            )),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[(Style, String)]) -> String {
        lines
            .iter()
            .map(|(_, l)| l.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn sample() -> Vec<Block> {
        vec![
            Block::Turn("codex · gpt · high".into()),
            Block::Prompt("Fix the bug".into()),
            Block::Reasoning("Look at main first".into()),
            Block::Tool {
                name: "shell".into(),
                detail: "cargo check".into(),
            },
            Block::Output {
                name: "shell".into(),
                text: "error: boom\nsecond".into(),
                exit_code: Some(101),
            },
            Block::Text("Done.".into()),
            Block::End {
                ok: true,
                text: "turn finished".into(),
            },
        ]
    }

    #[test]
    fn each_cli_is_drawn_in_its_own_style() {
        let codex = text(&render("codex", &sample()));
        assert!(codex.contains("• Ran cargo check"), "{codex}");
        assert!(codex.contains("  └ error: boom"), "{codex}");
        assert!(codex.contains("exit code 101"));
        assert!(codex.contains("› Fix the bug"));
        let claude = text(&render("claude", &sample()));
        assert!(claude.contains("⏺ shell(cargo check)"), "{claude}");
        assert!(claude.contains("  ⎿  error: boom"), "{claude}");
        assert!(claude.contains("✻ Look at main first"));
        let other = text(&render("opencode", &sample()));
        assert!(other.contains("→ shell cargo check"), "{other}");
    }

    #[test]
    fn long_output_is_folded_and_failures_are_red() {
        let long = (0..40)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let lines = render(
            "codex",
            &[Block::Output {
                name: "shell".into(),
                text: long,
                exit_code: Some(1),
            }],
        );
        let shown = text(&lines);
        assert!(shown.contains("line 11"));
        assert!(!shown.contains("line 12"));
        assert!(shown.contains("… +28 more lines"));
        assert_eq!(lines[0].0.fg, Some(Color::Red));
    }

    #[test]
    fn terminal_escapes_in_agent_text_are_removed() {
        let shown = text(&render(
            "codex",
            &[
                Block::Text("hi \u{1b}[31mred\u{1b}[0m \u{1b}]52;c;AAAA\u{7}".into()),
                Block::Output {
                    name: "shell".into(),
                    text: String::new(),
                    exit_code: None,
                },
            ],
        ));
        assert!(!shown.contains('\u{1b}'));
        assert!(shown.contains("hi red"));
        assert!(shown.contains("(no output)"));
    }
}
