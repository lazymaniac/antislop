use super::*;

pub(super) fn fenced<'a>(text: &'a str, tag: &str) -> Option<(String, &'a str)> {
    let open = format!("```{tag}");
    let start = text.rfind(&open)?;
    let body_start = start + open.len();
    let rest = &text[body_start..];
    if !rest.starts_with(['\n', '\r', ' ']) {
        return None;
    }
    // The closing fence stands on its own line; a ``` inside the body (in a
    // JSON string, say) does not end it. A one-line block closes at the first.
    let end = rest
        .find("\n```")
        .map_or_else(|| rest.find("```"), |at| Some(at + 1))?;
    let mut remaining = text[..start].trim_end().to_owned();
    let after = rest[end + 3..].trim();
    if !after.is_empty() {
        remaining.push_str("\n\n");
        remaining.push_str(after);
    }
    Some((remaining, rest[..end].trim()))
}

/// A conflict resolver's explicit request for a user decision.
pub fn integration_question(answer: &str) -> Option<String> {
    let (_, question) = fenced(answer, "integration_question")?;
    (!question.trim().is_empty()).then(|| question.trim().to_owned())
}

/// A worker's optional own one-line effect: a ```effect block in the same
/// answer it reports in, so no extra model call is spent asking for it.
/// The last such block wins (same rule as [`fenced`]); its closing fence
/// must stand on its own line, so a stray inline backtick cannot cut the
/// block short. An empty or damaged block is left in the report whole —
/// it yields `None` and the caller keeps the diffstat or report-line
/// fallback, so nothing the worker wrote is lost. A kept effect is bounded
/// to [`EFFECT_LIMIT`] with [`bound_cut`], which also says whether it cut;
/// the caller stores that as the card's `effect_cut` metadata.
pub fn parse_effect(answer: &str) -> (String, Option<(String, bool)>) {
    let Some((start, close)) = effect_block_bounds(answer) else {
        return (answer.to_owned(), None);
    };
    let rest = &answer[start + "```effect".len()..];
    let body = rest[..close].trim();
    if body.is_empty() {
        return (answer.to_owned(), None);
    }
    let effect = one_line_effect(body);
    let remaining = effect_remainder(answer, start, close);
    (remaining, Some(bound_cut(&effect, EFFECT_LIMIT)))
}

fn effect_block_bounds(answer: &str) -> Option<(usize, usize)> {
    let start = answer.rfind("```effect")?;
    let rest = &answer[start + "```effect".len()..];
    if !rest.starts_with(['\n', '\r', ' ']) {
        return None;
    }
    find_fence_line(rest).map(|close| (start, close))
}

fn find_fence_line(rest: &str) -> Option<usize> {
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\n', '\r', ' ', '\t']) == "```" {
            return Some(offset);
        }
        offset += line.len();
    }
    None
}

fn one_line_effect(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn effect_remainder(answer: &str, start: usize, close: usize) -> String {
    let rest = &answer[start + "```effect".len()..];
    let mut remaining = answer[..start].trim_end().to_owned();
    let after = rest[close + 3..].trim();
    if !after.is_empty() {
        append_after_block(&mut remaining, after);
    }
    remaining
}

fn append_after_block(remaining: &mut String, after: &str) {
    if !remaining.is_empty() {
        remaining.push_str("\n\n");
    }
    remaining.push_str(after);
}

/// A message an agent addresses to another agent in a ```message block.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Outgoing {
    /// A task id of the plan, `orchestrator`, or `all` (every unfinished task).
    pub to: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    pub text: String,
}

fn default_kind() -> String {
    "status".into()
}

/// Every ```message block of an answer, and the answer without them.
/// Unreadable blocks stay in the text, so nothing the agent said is lost.
pub fn parse_messages(answer: &str) -> (String, Vec<Outgoing>) {
    let (text, messages, _) = parse_messages_checked(answer);
    (text, messages)
}

/// Like `parse_messages`, and also the bodies of the blocks it could not read.
pub fn parse_messages_checked(answer: &str) -> (String, Vec<Outgoing>, Vec<String>) {
    let mut unreadable = vec![];
    let mut text = answer.to_owned();
    let mut messages = vec![];
    let mut kept = vec![];
    while let Some((rest, body)) = fenced(&text, "message") {
        match serde_json::from_str::<Outgoing>(body) {
            Ok(message) if !message.text.trim().is_empty() => messages.push(message),
            _ => {
                unreadable.push(body.to_owned());
                kept.push(format!("```message\n{body}\n```"));
            }
        }
        text = rest;
    }
    messages.reverse();
    for block in kept.into_iter().rev() {
        text.push_str("\n\n");
        text.push_str(&block);
    }
    (text.trim().to_owned(), messages, unreadable)
}

pub const MESSAGE_FORMAT: &str = "To tell another agent something it needs \
     (a finding about its area, a handoff, a question), add a block \
     ```message\n{\"to\": \"t2\", \"kind\": \"finding|handoff|question|answer|decision|status\", \
     \"text\": \"...\"}\n``` addressed to a task id of the plan, \"orchestrator\" or \"all\". \
     It reaches that agent in its next turn. Keep messages short and only \
     when they matter to the recipient. Never write three backticks inside \
     the message text, and keep the JSON valid (escape newlines as \\n).";

/// Messages delivered into a turn, before its own prompt.
pub fn inbox_text(mail: &[crate::lean::store::Mail]) -> String {
    let lines: Vec<String> = mail
        .iter()
        .map(|mail| format!("- from {} ({}): {}", mail.sender, mail.kind, mail.text))
        .collect();
    format!("Messages for you:\n{}\n\n", lines.join("\n"))
}

/// The prompt of a turn that answers messages for a task that is done.
pub fn answer_prompt(title: &str, first: bool) -> String {
    let head = if first {
        format!(
            "You have follow-up mail about task \"{title}\". This reply changes neither its status nor its approval. "
        )
    } else {
        String::new()
    };
    format!(
        "{head}Messages arrived for you (above). Answer them read-only: change no \
         files. Reply to each sender with a message block addressed to them \
         (`{{\"to\": \"<sender>\", \"kind\": \"answer\", \"text\": \"...\"}}`); \
         Do not reply to acknowledgements or answers unless they contain a new \
         question or request. Say plainly when you have nothing to add."
    )
}

pub const INBOX_FOLLOWUP: &str = "Messages arrived while you worked (above). \
     Take them into account where they matter and give your complete, \
     updated report.";

/// How a worker may offer the one line its task card shows, from the same
/// answer it reports in. The runtime keeps the worker's own line when it is
/// there, falls back to the diffstat (writers) or the report's first text
/// line (read-only) when it is not, and bounds what it keeps to
/// `EFFECT_LIMIT` bytes, recording a cut in the card's `effect_cut` flag.
pub const EFFECT_FORMAT: &str = "Optionally, end your report with the one line \
     your task card shows, in a block like this:\n```effect\nwhat you changed or \
     found, on one line, no backticks\n```\nWrite the effect as a single line \
     (no backticks inside), keep it short, and let the closing ``` stand on its \
     own line. Without the block the card falls back to the diff's summary or \
     your first report line; a longer line is cut with a mark, never kept whole.";
