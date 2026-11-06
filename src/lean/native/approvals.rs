//! Recognizes a provider CLI's approval dialog on its terminal screen and says
//! which keys answer each of its choices.
//!
//! A dialog is recognized only when its whole structure is present at the
//! bottom of the screen: question, choices and the provider's footer hint. An
//! agent talking about permissions, or quoting a dialog in a report, never
//! matches because prose is not followed by the footer on the last line.
use crate::config::ProviderKind;

/// A key typed into the CLI to answer a dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Key {
    Char(char),
    Enter,
    Escape,
    Up,
    Down,
    Right,
}

/// What a choice does, as far as its label says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    Allow,
    AllowAlways,
    Deny,
    /// Anything else, such as switching the CLI to another mode.
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Command,
    /// A tool of an MCP server; see `Approval::mcp_server`.
    Mcp,
    /// The second step of a choice that needs confirming (OpenCode's
    /// "Allow always").
    Confirm,
    Trust,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Choice {
    /// Position in the dialog, counted from 1.
    pub id: String,
    pub label: String,
    pub decision: Decision,
    pub keys: Vec<Key>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Approval {
    pub kind: Kind,
    /// What the CLI asks to do, as it shows it.
    pub summary: String,
    pub choices: Vec<Choice>,
    /// Identifies this dialog; stable while it is redrawn.
    pub fingerprint: String,
    /// For `Kind::Mcp`: the MCP server whose tool is asked for.
    pub server: Option<String>,
}

impl Approval {
    /// The MCP server whose tool is asked for, when the dialog names it.
    pub(crate) fn mcp_server(&self) -> Option<&str> {
        self.server.as_deref()
    }

    pub(crate) fn choice(&self, id: &str) -> Option<&Choice> {
        self.choices.iter().find(|choice| choice.id == id)
    }
}

/// How a dialog was settled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// The id of the choice to press.
    Choose(String),
    /// A person answered in the attached terminal view.
    AnsweredInTerminal,
}

/// Decides a dialog for a session's turn. Blocks until it is settled. The
/// first argument tells whether the dialog is still on screen; the second is
/// set when the turn is cancelled.
pub(crate) type Approver = dyn Fn(&Approval, &std::sync::atomic::AtomicBool, &dyn Fn() -> bool) -> crate::Result<Verdict>
    + Send
    + Sync;

/// The dialog on `screen`, if the provider's CLI shows one. Providers whose
/// dialogs are not recognized yet never match.
pub(crate) fn detect(provider: ProviderKind, screen: &str) -> Option<Approval> {
    let lines: Vec<&str> = screen
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect();
    match provider {
        ProviderKind::Claude => claude(&lines),
        ProviderKind::Codex => codex(&lines),
        ProviderKind::Opencode => opencode(&lines),
        ProviderKind::Agy => agy(&lines),
        ProviderKind::Gemini => gemini(&lines),
    }
}

fn claude(lines: &[&str]) -> Option<Approval> {
    let footer = lines.len().checked_sub(1)?;
    let hint = lines[footer].trim().to_ascii_lowercase();
    if !hint.starts_with("esc to cancel") && !hint.starts_with("enter to confirm") {
        return None;
    }
    if let Some((first, numbered)) = numbered_choices(lines, footer) {
        let question = lines[first.checked_sub(1)?].trim();
        if !question.ends_with('?') {
            return None;
        }
        let mut summary = lines[..first - 1]
            .iter()
            .rev()
            .take_while(|line| !is_solid_rule(line))
            .map(|line| line.trim().trim_start_matches('│').trim().to_owned())
            .filter(|line| !line.is_empty() && !is_rule(line) && !line.starts_with("Tip:"))
            .collect::<Vec<_>>();
        summary.reverse();
        let mut server = None;
        let kind = if summary
            .first()
            .is_some_and(|head| head.eq_ignore_ascii_case("bash command"))
        {
            Kind::Command
        } else if summary.first().is_some_and(|head| head == "Tool use")
            && summary
                .get(1)
                .is_some_and(|line| line.trim_end().ends_with("Tool: (MCP)"))
        {
            server = summary[1]
                .split_once(" — ")
                .map(|(server, _)| server.trim().to_owned());
            Kind::Mcp
        } else {
            Kind::Other
        };
        let choices = numbered
            .into_iter()
            .map(|(number, label, _)| {
                let keys = char::from_digit(number, 10)
                    .filter(|_| number > 0)
                    .map(|digit| vec![Key::Char(digit)])
                    .unwrap_or_default();
                choice(number, label, keys)
            })
            .collect();
        return finish(kind, summary.join("\n"), choices)
            .map(|approval| Approval { server, ..approval });
    }
    let (first, marked) = marked_choices(lines, footer)?;
    let title = lines
        .iter()
        .position(|line| line.trim_start().starts_with("Accessing workspace:"))?;
    if !lines[title..first]
        .iter()
        .any(|line| line.contains("one you trust"))
    {
        return None;
    }
    let path = lines[title + 1..first]
        .iter()
        .take_while(|line| !line.trim_start().starts_with("Quick safety check"))
        .map(|line| line.trim())
        .collect::<String>();
    finish(Kind::Trust, path, navigation_choices(marked))
}

/// Agy draws a status line under its dialogs, so the footer is not the last line.
fn agy(lines: &[&str]) -> Option<Approval> {
    let footer = lines
        .iter()
        .rposition(|line| line.trim().starts_with("↑/↓ Navigate · enter Confirm"))?;
    if footer + 2 < lines.len() {
        return None;
    }
    let lines = &lines[..=footer];
    let (first, marked) = marked_choices(lines, footer)?;
    let title = lines
        .iter()
        .position(|line| line.trim_start().starts_with("Accessing workspace:"))?;
    if !lines[title..first]
        .iter()
        .any(|line| line.contains("Do you trust the contents of this project?"))
    {
        return None;
    }
    let path = lines[title + 1..first]
        .iter()
        .take_while(|line| !line.trim_start().starts_with("Do you trust"))
        .map(|line| line.trim())
        .collect::<String>();
    finish(Kind::Trust, path, navigation_choices(marked))
}

/// Gemini draws its tool confirmation in a bordered box: a question, then
/// `● 1. Allow once` style choices (the dot marks the selected one). The
/// layout comes from the published bundle (see the fixtures); it has not been
/// seen on a live Gemini yet.
fn gemini(lines: &[&str]) -> Option<Approval> {
    let rows: Vec<String> = lines
        .iter()
        .map(|line| {
            line.trim()
                .trim_start_matches('│')
                .trim_end_matches('│')
                .trim()
                .to_owned()
        })
        .collect();
    let numbered = |row: &str| -> Option<(bool, u32, String)> {
        let (cursor, rest) = match row.strip_prefix('●') {
            Some(rest) => (true, rest.trim_start()),
            None => (false, row),
        };
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 || digits > 2 {
            return None;
        }
        let text = rest[digits..].strip_prefix(". ")?;
        Some((cursor, rest[..digits].parse().ok()?, text.trim().to_owned()))
    };
    let last = rows.iter().rposition(|row| numbered(row).is_some())?;
    // Only the box's bottom border and a short footer may follow.
    if rows[last + 1..]
        .iter()
        .filter(|row| !row.is_empty() && !row.starts_with('╰'))
        .count()
        > 3
    {
        return None;
    }
    let mut first = last;
    while first > 0 && numbered(&rows[first - 1]).is_some() {
        first -= 1;
    }
    let entries: Vec<(bool, u32, String)> = (first..=last)
        .filter_map(|index| numbered(&rows[index]))
        .collect();
    let sequential = entries
        .iter()
        .enumerate()
        .all(|(index, (_, number, _))| *number as usize == index + 1);
    let cursor = entries.iter().position(|(marked, _, _)| *marked)?;
    if entries.len() < 2 || !sequential {
        return None;
    }
    // The trust question is followed by a paragraph before its choices.
    let trust_at = rows[..first]
        .iter()
        .rposition(|row| row == "Do you trust the files in this folder?");
    let trust = trust_at.is_some();
    let question_at = match trust_at {
        Some(at) => at,
        None => rows[..first].iter().rposition(|row| !row.is_empty())?,
    };
    let question = rows[question_at].as_str();
    // A tool confirmation always ends with the refusal ("No, suggest changes (esc)").
    if !trust
        && !entries
            .last()?
            .2
            .to_ascii_lowercase()
            .starts_with("no, suggest changes")
    {
        return None;
    }
    let mcp = question
        .strip_prefix("Allow execution of MCP tool ")
        .and_then(|rest| rest.split_once(" from server \""))
        .map(|(_, server)| server.trim_end_matches("\"?").to_owned());
    if mcp.is_none()
        && !trust
        && !(question.starts_with("Allow execution of [")
            || question == "Do you want to proceed?"
            || question == "Apply this change?")
    {
        return None;
    }
    let kind = if mcp.is_some() {
        Kind::Mcp
    } else if trust {
        Kind::Trust
    } else if question.starts_with("Allow execution of [Shell]") {
        Kind::Command
    } else {
        Kind::Other
    };
    let from = rows[..question_at]
        .iter()
        .rposition(|row| row.starts_with('╭'))
        .map_or(0, |at| at + 1);
    let summary = rows[from..=question_at]
        .iter()
        .filter(|row| !row.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let choices = entries
        .into_iter()
        .enumerate()
        .map(|(position, (_, number, label))| choice(number, label, navigate(cursor, position)))
        .collect();
    finish(kind, summary, choices).map(|approval| Approval {
        server: mcp,
        ..approval
    })
}

fn codex(lines: &[&str]) -> Option<Approval> {
    let footer = lines.len().checked_sub(1)?;
    let hint = lines[footer].to_ascii_lowercase();
    if !hint.contains("enter to confirm or esc to cancel")
        && !hint.contains("enter to submit | esc to cancel")
        && !(hint.contains("enter continue")
            && (hint.contains("esc back") || hint.contains("esc quit")))
    {
        return None;
    }
    let (first, numbered) = numbered_choices(lines, footer)?;
    let question = lines[..first].iter().rposition(|line| {
        let line = line.trim();
        line.starts_with("Would you like to")
            || line.starts_with("Trust this folder?")
            || mcp_heading(line).is_some()
    })?;
    let heading = lines[question].trim();
    let server = mcp_heading(heading).map(str::to_owned);
    let kind = if server.is_some() {
        Kind::Mcp
    } else if heading.starts_with("Trust this folder?") {
        Kind::Trust
    } else if heading.contains("run the following command") {
        Kind::Command
    } else {
        Kind::Other
    };
    let from = match kind {
        Kind::Trust => question.saturating_sub(2),
        Kind::Mcp => question,
        _ => question + 1,
    };
    let summary = lines[from..first]
        .iter()
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join("\n");
    let choices = numbered
        .into_iter()
        .enumerate()
        .map(|(position, (number, label, cursor))| {
            // A choice may carry a description after a wide gap.
            let label = label.split("  ").next().unwrap_or(&label).trim().to_owned();
            let (label, shortcut) = split_shortcut(&label);
            let keys = match shortcut {
                Some(key) => vec![key],
                None => navigate(cursor, position),
            };
            choice(number, label, keys)
        })
        .collect();
    finish(kind, summary, choices).map(|approval| Approval { server, ..approval })
}

/// The server named by `Allow the <server> MCP server to run tool "<tool>"?`.
fn mcp_heading(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix("Allow the ")?;
    let (server, tail) = rest.split_once(" MCP server to run tool")?;
    tail.trim_end().ends_with('?').then_some(server)
}

fn opencode(lines: &[&str]) -> Option<Approval> {
    let rows: Vec<String> = lines
        .iter()
        .map(|line| line.trim().trim_start_matches(['┃', '│']).trim().to_owned())
        .filter(|line| !line.is_empty())
        .collect();
    let footer = rows.len().checked_sub(1)?;
    let hint = &rows[footer];
    if !hint.contains("enter confirm") {
        return None;
    }
    let confirming = rows
        .iter()
        .rposition(|row| row.contains("Always allow"))
        .filter(|&at| {
            rows[at + 1..footer]
                .iter()
                .any(|row| row.starts_with("This will allow the following patterns"))
        });
    let heading = confirming.or_else(|| {
        rows.iter()
            .rposition(|row| row.contains("Permission required"))
    })?;
    let options = hint.split("ctrl+f").next()?.split('⇆').next()?.trim();
    let labels: Vec<&str> = options
        .split("  ")
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .collect();
    if labels.len() < 2 || labels.iter().any(|label| label.split(' ').count() > 3) {
        return None;
    }
    let summary = rows[heading + 1..footer]
        .iter()
        .take_while(|row| !row.starts_with("Patterns"))
        .map(|row| row.trim_start_matches('←').trim())
        .collect::<Vec<_>>()
        .join("");
    // The dialog opens on its first choice; the others are reached to the right.
    let choices = labels
        .iter()
        .enumerate()
        .map(|(position, label)| {
            let mut keys = vec![Key::Right; position];
            keys.push(Key::Enter);
            choice(position as u32 + 1, (*label).to_owned(), keys)
        })
        .collect();
    let kind = if confirming.is_some() {
        Kind::Confirm
    } else {
        Kind::Other
    };
    finish(kind, summary, choices)
}

/// `(position, label, cursor_on_it)` of numbered choices like `❯ 1. Yes`.
type Numbered = Vec<(u32, String, usize)>;

/// The numbered choices that end just above `footer`, as `(first_line, choices)`.
/// Lines between two numbers continue the label of the choice above them. The
/// cursor is the index of the choice carrying the marker, shared by all entries.
fn numbered_choices(lines: &[&str], footer: usize) -> Option<(usize, Numbered)> {
    let mut first = None;
    for index in (footer.saturating_sub(24)..footer).rev() {
        if let Some((_, 1, _)) = number(lines[index]) {
            first = Some(index);
            break;
        }
    }
    let first = first?;
    let mut entries: Vec<(u32, String, bool)> = Vec::new();
    for line in &lines[first..footer] {
        match number(line) {
            Some((cursor, number, text)) => entries.push((number, text.to_owned(), cursor)),
            None => {
                let last = entries.last_mut()?;
                last.1.push(' ');
                last.1.push_str(line.trim());
            }
        }
    }
    let sequential = entries
        .iter()
        .enumerate()
        .all(|(index, (number, _, _))| *number as usize == index + 1);
    if entries.len() < 2 || !sequential {
        return None;
    }
    // A real dialog always shows which choice is selected.
    let cursor = entries.iter().position(|(_, _, marked)| *marked)?;
    Some((
        first,
        entries
            .into_iter()
            .map(|(number, text, _)| (number, text, cursor))
            .collect(),
    ))
}

/// A line like `❯ 2. text` or `3. text`: `(has_cursor, number, text)`.
fn number(line: &str) -> Option<(bool, u32, &str)> {
    let line = line.trim_start();
    let (cursor, line) = match line.strip_prefix(['❯', '›', '>']) {
        Some(rest) => (true, rest.trim_start()),
        None => (false, line),
    };
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 2 {
        return None;
    }
    let rest = line[digits..].strip_prefix(". ")?;
    Some((cursor, line[..digits].parse().ok()?, rest.trim()))
}

/// Unnumbered choices where a marker (`❯` or `>`) shows the selected one and
/// the others are indented under it, ending just above `footer`:
/// `(first_line, (labels, cursor))`.
fn marked_choices(lines: &[&str], footer: usize) -> Option<(usize, (Vec<String>, usize))> {
    let marker = (footer.saturating_sub(8)..footer).find(|&index| {
        let line = lines[index].trim_start();
        line.starts_with(['❯', '>']) && line.chars().nth(1) == Some(' ')
    })?;
    let column = lines[marker].len() - lines[marker].trim_start().len();
    let indented = |index: usize| {
        let line = lines[index];
        line.len() - line.trim_start().len() == column + 2
    };
    let mut first = marker;
    while first > 0 && indented(first - 1) {
        first -= 1;
    }
    if (marker + 1..footer).any(|index| !indented(index)) {
        return None;
    }
    let labels: Vec<String> = (first..footer)
        .map(|index| {
            lines[index]
                .trim_start()
                .trim_start_matches(['❯', '>'])
                .trim()
                .to_owned()
        })
        .collect();
    (labels.len() >= 2).then_some((first, (labels, marker - first)))
}

fn navigation_choices((labels, cursor): (Vec<String>, usize)) -> Vec<Choice> {
    labels
        .into_iter()
        .enumerate()
        .map(|(position, label)| choice(position as u32 + 1, label, navigate(cursor, position)))
        .collect()
}

/// Moves the cursor from `cursor` to `target` and confirms.
fn navigate(cursor: usize, target: usize) -> Vec<Key> {
    let mut keys = if target >= cursor {
        vec![Key::Down; target - cursor]
    } else {
        vec![Key::Up; cursor - target]
    };
    keys.push(Key::Enter);
    keys
}

/// `label (y)` / `label (esc)` → the label and the key the dialog names.
fn split_shortcut(text: &str) -> (String, Option<Key>) {
    if let Some(open) = text.rfind(" (")
        && let Some(inner) = text[open + 2..].strip_suffix(')')
    {
        let key = match inner {
            "esc" => Some(Key::Escape),
            single if single.chars().count() == 1 => single.chars().next().map(Key::Char),
            _ => None,
        };
        if key.is_some() {
            return (text[..open].trim().to_owned(), key);
        }
    }
    (text.to_owned(), None)
}

fn choice(number: u32, label: String, keys: Vec<Key>) -> Choice {
    Choice {
        id: number.to_string(),
        decision: decision(&label),
        label,
        keys,
    }
}

fn decision(label: &str) -> Decision {
    let label = label.to_ascii_lowercase();
    if label.contains("switch to") {
        Decision::Other
    } else if label.starts_with("no")
        || label.starts_with("reject")
        || label.starts_with("deny")
        || label.starts_with("don't")
        || label.starts_with("back")
        || label.starts_with("cancel")
        || label.starts_with("quit")
    {
        Decision::Deny
    } else if label.contains("always") || label.contains("don't ask again") {
        Decision::AllowAlways
    } else if label.starts_with("yes")
        || label.starts_with("allow")
        || label.starts_with("confirm")
        || label.starts_with("trust")
        || label.starts_with("proceed")
    {
        Decision::Allow
    } else {
        Decision::Other
    }
}

fn finish(kind: Kind, summary: String, choices: Vec<Choice>) -> Option<Approval> {
    if choices.len() < 2 || choices.iter().any(|choice| choice.keys.is_empty()) {
        return None;
    }
    let mut identity = summary.split_whitespace().collect::<Vec<_>>().join(" ");
    for choice in &choices {
        identity.push('\n');
        identity.push_str(&choice.label);
    }
    Some(Approval {
        kind,
        summary,
        choices,
        fingerprint: crate::hash(identity.as_bytes()),
        server: None,
    })
}

fn is_rule(line: &str) -> bool {
    let line = line.trim();
    line.chars().count() >= 10 && line.chars().all(|ch| matches!(ch, '─' | '╌' | '━'))
}

/// The unbroken rule that opens a Claude dialog, not the dashed one around a command.
fn is_solid_rule(line: &str) -> bool {
    is_rule(line) && !line.contains('╌')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(text: &str) -> &str {
        text.split_once('\n').map_or("", |(_, rest)| rest)
    }
    const CLAUDE_COMMAND: &str =
        include_str!("../../../tests/fixtures/approvals/claude/command.txt");
    const CLAUDE_TRUST: &str = include_str!("../../../tests/fixtures/approvals/claude/trust.txt");
    const CLAUDE_MCP: &str = include_str!("../../../tests/fixtures/approvals/claude/mcp.txt");
    const CODEX_MCP: &str = include_str!("../../../tests/fixtures/approvals/codex/mcp.txt");
    const CODEX_COMMAND: &str = include_str!("../../../tests/fixtures/approvals/codex/command.txt");
    const CODEX_TRUST: &str = include_str!("../../../tests/fixtures/approvals/codex/trust.txt");
    const CODEX_TRUST_QUIT: &str =
        include_str!("../../../tests/fixtures/approvals/codex/trust_quit.txt");
    const OPENCODE_CONFIRM: &str =
        include_str!("../../../tests/fixtures/approvals/opencode/always_confirm.txt");
    const AGY_TRUST: &str = include_str!("../../../tests/fixtures/approvals/agy/trust.txt");
    const GEMINI_SHELL: &str = include_str!("../../../tests/fixtures/approvals/gemini/shell.txt");
    const GEMINI_LIVE: &str =
        include_str!("../../../tests/fixtures/approvals/gemini/shell_live.txt");
    const GEMINI_TRUST: &str = include_str!("../../../tests/fixtures/approvals/gemini/trust.txt");
    const GEMINI_MCP_LIVE: &str =
        include_str!("../../../tests/fixtures/approvals/gemini/mcp_live.txt");
    const GEMINI_MCP: &str = include_str!("../../../tests/fixtures/approvals/gemini/mcp.txt");
    const OPENCODE: &str =
        include_str!("../../../tests/fixtures/approvals/opencode/external_directory.txt");

    fn labels(approval: &Approval) -> Vec<&str> {
        approval.choices.iter().map(|c| c.label.as_str()).collect()
    }

    #[test]
    fn codex_trust_dialog_whose_second_choice_quits_is_recognized() {
        let approval = detect(ProviderKind::Codex, fixture(CODEX_TRUST_QUIT)).unwrap();
        assert_eq!(approval.kind, Kind::Trust);
        assert_eq!(labels(&approval), ["Trust and continue", "Quit"]);
        assert_eq!(approval.choices[0].decision, Decision::Allow);
        assert_eq!(approval.choices[0].keys, [Key::Enter]);
    }

    #[test]
    fn codex_mcp_dialog_names_the_server_and_is_answered_by_moving_the_cursor() {
        let approval = detect(ProviderKind::Codex, fixture(CODEX_MCP)).unwrap();
        assert_eq!(approval.kind, Kind::Mcp);
        assert_eq!(approval.mcp_server(), Some("antislop"));
        assert_eq!(
            labels(&approval),
            ["Allow", "Allow for this session", "Always allow", "Cancel"]
        );
        assert_eq!(approval.choices[0].decision, Decision::Allow);
        assert_eq!(approval.choices[0].keys, [Key::Enter]);
        assert_eq!(
            approval.choices[3].keys,
            [Key::Down, Key::Down, Key::Down, Key::Enter]
        );
        assert_eq!(approval.choices[3].decision, Decision::Deny);
        assert!(approval.summary.contains("text: probe passed"));
    }

    #[test]
    fn opencode_allow_always_asks_for_a_confirmation_that_is_its_own_dialog() {
        let approval = detect(ProviderKind::Opencode, fixture(OPENCODE_CONFIRM)).unwrap();
        assert_eq!(approval.kind, Kind::Confirm);
        assert_eq!(labels(&approval), ["Confirm", "Cancel"]);
        assert_eq!(approval.choices[0].decision, Decision::Allow);
        assert_eq!(approval.choices[0].keys, [Key::Enter]);
        assert_eq!(approval.choices[1].keys, [Key::Right, Key::Enter]);
        let first = detect(ProviderKind::Opencode, fixture(OPENCODE)).unwrap();
        assert_ne!(first.kind, Kind::Confirm);
        assert_ne!(first.fingerprint, approval.fingerprint);
    }

    #[test]
    fn agy_trust_dialog_is_answered_by_moving_the_cursor() {
        let approval = detect(ProviderKind::Agy, fixture(AGY_TRUST)).unwrap();
        assert_eq!(approval.kind, Kind::Trust);
        assert_eq!(labels(&approval), ["Yes, I trust this folder", "No, exit"]);
        assert_eq!(approval.choices[0].keys, [Key::Enter]);
        assert_eq!(approval.choices[1].keys, [Key::Down, Key::Enter]);
        assert!(approval.summary.starts_with("/private/tmp"));
    }

    #[test]
    fn gemini_dialogs_are_answered_by_moving_the_cursor() {
        let shell = detect(ProviderKind::Gemini, fixture(GEMINI_SHELL)).unwrap();
        assert_eq!(shell.kind, Kind::Command);
        assert_eq!(
            labels(&shell),
            [
                "Allow once",
                "Allow for this session",
                "No, suggest changes (esc)"
            ]
        );
        assert_eq!(shell.choices[0].keys, [Key::Enter]);
        assert_eq!(shell.choices[2].keys, [Key::Down, Key::Down, Key::Enter]);
        assert_eq!(shell.choices[2].decision, Decision::Deny);
        assert!(shell.summary.contains("touch /tmp/outside/probe.txt"));
        let mcp = detect(ProviderKind::Gemini, fixture(GEMINI_MCP)).unwrap();
        assert_eq!(mcp.kind, Kind::Mcp);
        assert_eq!(mcp.mcp_server(), Some("other"));
        assert_eq!(mcp.choices[2].decision, Decision::AllowAlways);
        let live = detect(ProviderKind::Gemini, fixture(GEMINI_LIVE)).unwrap();
        assert_eq!(live.kind, Kind::Command);
        assert!(live.summary.contains("touch /tmp/gem-probe.txt"));
        assert_eq!(live.choices.len(), 3);
        let live_mcp = detect(ProviderKind::Gemini, fixture(GEMINI_MCP_LIVE)).unwrap();
        assert_eq!(live_mcp.kind, Kind::Mcp);
        assert_eq!(live_mcp.mcp_server(), Some("probe"));
        assert_eq!(live_mcp.choices.len(), 4);
        assert_eq!(live_mcp.choices[3].decision, Decision::Deny);
        let trust = detect(ProviderKind::Gemini, fixture(GEMINI_TRUST)).unwrap();
        assert_eq!(trust.kind, Kind::Trust);
        assert_eq!(trust.choices[0].decision, Decision::Allow);
        assert_eq!(trust.choices[2].decision, Decision::Deny);
        assert_eq!(trust.choices[2].keys, [Key::Down, Key::Down, Key::Enter]);
        // Prose that quotes the question is not a dialog.
        assert!(detect(ProviderKind::Gemini, "Allow execution of [Shell]?\nok").is_none());
    }

    #[test]
    fn claude_names_the_mcp_server_whose_tool_it_asks_about() {
        let approval = detect(ProviderKind::Claude, fixture(CLAUDE_MCP)).unwrap();
        assert_eq!(approval.kind, Kind::Mcp);
        assert_eq!(approval.mcp_server(), Some("antislop"));
        assert_eq!(approval.choices[0].decision, Decision::Allow);
        assert_eq!(approval.choices[0].keys, [Key::Char('1')]);
        // A command that merely mentions the server is not an MCP tool.
        let command = detect(ProviderKind::Claude, fixture(CLAUDE_COMMAND)).unwrap();
        assert_eq!(command.mcp_server(), None);
    }

    #[test]
    fn claude_command_dialog_is_answered_by_number() {
        let approval = detect(ProviderKind::Claude, fixture(CLAUDE_COMMAND)).expect("dialog");
        assert_eq!(approval.kind, Kind::Command);
        assert!(approval.summary.starts_with("Bash command"));
        assert!(approval.summary.contains("touch /private/tmp"));
        assert!(!approval.summary.contains("Tip:"));
        assert_eq!(approval.choices.len(), 4);
        assert_eq!(approval.choices[0].label, "Yes");
        assert_eq!(approval.choices[0].decision, Decision::Allow);
        assert_eq!(approval.choices[0].keys, vec![Key::Char('1')]);
        assert_eq!(approval.choices[1].decision, Decision::AllowAlways);
        assert!(approval.choices[1].label.ends_with("from this project"));
        assert_eq!(approval.choices[2].decision, Decision::Other);
        assert_eq!(approval.choices[3].decision, Decision::Deny);
        assert_eq!(approval.choices[3].keys, vec![Key::Char('4')]);
    }

    #[test]
    fn claude_trust_dialog_moves_the_cursor_to_the_choice() {
        let approval = detect(ProviderKind::Claude, fixture(CLAUDE_TRUST)).expect("dialog");
        assert_eq!(approval.kind, Kind::Trust);
        assert!(approval.summary.ends_with("scratchpad/work/claude"));
        assert_eq!(labels(&approval), ["No, exit", "Yes, I trust this folder"]);
        assert_eq!(approval.choices[0].keys, vec![Key::Enter]);
        assert_eq!(approval.choices[1].keys, vec![Key::Down, Key::Enter]);
        assert_eq!(approval.choices[1].decision, Decision::Allow);
    }

    #[test]
    fn codex_command_dialog_uses_the_shortcuts_it_names() {
        let approval = detect(ProviderKind::Codex, fixture(CODEX_COMMAND)).expect("dialog");
        assert_eq!(approval.kind, Kind::Command);
        assert!(approval.summary.contains("Reason: Do you approve"));
        assert!(approval.summary.contains("$ touch"));
        assert_eq!(approval.choices.len(), 3);
        assert_eq!(approval.choices[0].label, "Yes, proceed");
        assert_eq!(approval.choices[0].keys, vec![Key::Char('y')]);
        assert_eq!(approval.choices[1].decision, Decision::AllowAlways);
        assert_eq!(approval.choices[1].keys, vec![Key::Char('p')]);
        assert_eq!(approval.choices[2].decision, Decision::Deny);
        assert_eq!(approval.choices[2].keys, vec![Key::Escape]);
    }

    #[test]
    fn codex_trust_dialog_moves_the_cursor() {
        let approval = detect(ProviderKind::Codex, fixture(CODEX_TRUST)).expect("dialog");
        assert_eq!(approval.kind, Kind::Trust);
        assert!(approval.summary.contains("/scratchpad/work/co"));
        assert_eq!(
            labels(&approval),
            ["Trust and continue", "Back to Agent Command Center"]
        );
        assert_eq!(approval.choices[0].keys, vec![Key::Enter]);
        assert_eq!(approval.choices[0].decision, Decision::Allow);
        assert_eq!(approval.choices[1].keys, vec![Key::Down, Key::Enter]);
        assert_eq!(approval.choices[1].decision, Decision::Deny);
    }

    #[test]
    fn opencode_dialog_selects_to_the_right() {
        let approval = detect(ProviderKind::Opencode, fixture(OPENCODE)).expect("dialog");
        assert_eq!(approval.kind, Kind::Other);
        assert!(approval.summary.starts_with("Access external directory"));
        assert!(approval.summary.ends_with("scratchpad/outside"));
        assert_eq!(labels(&approval), ["Allow once", "Allow always", "Reject"]);
        assert_eq!(approval.choices[0].keys, vec![Key::Enter]);
        assert_eq!(approval.choices[1].keys, vec![Key::Right, Key::Enter]);
        assert_eq!(
            approval.choices[2].keys,
            vec![Key::Right, Key::Right, Key::Enter]
        );
        assert_eq!(approval.choices[2].decision, Decision::Deny);
    }

    #[test]
    fn dialogs_are_not_taken_for_other_providers_or_unsupported_ones() {
        assert!(detect(ProviderKind::Codex, fixture(CLAUDE_COMMAND)).is_none());
        assert!(detect(ProviderKind::Claude, fixture(CODEX_COMMAND)).is_none());
        assert!(detect(ProviderKind::Opencode, fixture(CODEX_COMMAND)).is_none());
        assert!(detect(ProviderKind::Agy, fixture(CLAUDE_COMMAND)).is_none());
        assert!(detect(ProviderKind::Gemini, fixture(CLAUDE_COMMAND)).is_none());
    }

    #[test]
    fn a_dialog_that_is_not_at_the_bottom_is_not_one() {
        for (provider, text) in [
            (ProviderKind::Claude, CLAUDE_COMMAND),
            (ProviderKind::Codex, CODEX_COMMAND),
            (ProviderKind::Opencode, OPENCODE),
        ] {
            let after = format!("{}\nThe agent went on to say more.\n", fixture(text));
            assert!(detect(provider, &after).is_none(), "{provider:?}");
        }
    }

    #[test]
    fn prose_about_permissions_is_not_a_dialog() {
        for provider in [
            ProviderKind::Claude,
            ProviderKind::Codex,
            ProviderKind::Opencode,
        ] {
            for text in [
                "Do you want to proceed?\n1. Yes\n2. No\nEsc to cancel",
                "Do you want to proceed?\n❯ 1. Yes\n2. No\nPress Esc to cancel it.",
                "Would you like to run the following command?\n$ ls",
                "Permission required: Allow once  Reject",
                "I will ask: Do you want to proceed? Then press Esc to cancel.",
                "",
            ] {
                assert!(detect(provider, text).is_none(), "{provider:?}: {text}");
            }
        }
    }

    #[test]
    fn redrawing_keeps_the_fingerprint_and_other_commands_change_it() {
        let first = detect(ProviderKind::Codex, fixture(CODEX_COMMAND)).unwrap();
        let again = detect(
            ProviderKind::Codex,
            &format!("{}\n\n", fixture(CODEX_COMMAND)),
        )
        .unwrap();
        assert_eq!(first.fingerprint, again.fingerprint);
        let other = detect(
            ProviderKind::Codex,
            &fixture(CODEX_COMMAND).replace("touch", "rm"),
        )
        .unwrap();
        assert_ne!(first.fingerprint, other.fingerprint);
    }

    #[test]
    fn choices_are_looked_up_by_id() {
        let approval = detect(ProviderKind::Claude, fixture(CLAUDE_COMMAND)).unwrap();
        assert_eq!(approval.choice("4").map(|c| c.label.as_str()), Some("No"));
        assert!(approval.choice("9").is_none());
    }
}
