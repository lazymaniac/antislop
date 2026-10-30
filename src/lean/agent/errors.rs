/// Longest tool output or reasoning kept in an event.
pub const OUTPUT: usize = 2000;

/// Redact credentials before retaining the actionable CLI failure in the UI,
/// reports and events. This function has no diagnostic-output side effects.
/// Unknown, unlabelled secrets cannot be inferred reliably from arbitrary prose.
pub(crate) fn persisted_error(text: &str) -> String {
    redact_error(
        text,
        std::env::vars_os().filter_map(|(key, value)| {
            let key = key.to_string_lossy().to_ascii_uppercase();
            (key.contains("TOKEN")
                || key.contains("SECRET")
                || key.contains("PASSWORD")
                || key.contains("API_KEY")
                || key.contains("PRIVATE_KEY"))
            .then(|| value.into_string().ok())
            .flatten()
        }),
    )
}

pub(super) fn redact_error(text: &str, secrets: impl Iterator<Item = String>) -> String {
    let mut secrets: Vec<_> = secrets.filter(|secret| !secret.is_empty()).collect();
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    let mut text = text.to_owned();
    for secret in secrets {
        text = text.replace(&secret, "[REDACTED]");
    }
    let mut result = String::new();
    let mut credential_next = false;
    for part in text.split_inclusive(char::is_whitespace) {
        let word = part.trim_end_matches(char::is_whitespace);
        let whitespace = &part[word.len()..];
        let (visible, next) = redact_word(word, credential_next);
        result.push_str(&visible);
        credential_next = next;
        result.push_str(whitespace);
    }
    clip(&result, OUTPUT)
}

fn redact_word(word: &str, credential_next: bool) -> (String, bool) {
    if word.is_empty() {
        return (String::new(), credential_next);
    }
    let lower = word.to_ascii_lowercase();
    let assignment = lower.split_once('=').or_else(|| lower.split_once(':'));
    let sensitive = assignment.is_some_and(|(key, _)| {
        [
            "token",
            "secret",
            "password",
            "api_key",
            "apikey",
            "authorization",
        ]
        .iter()
        .any(|label| key.contains(label))
    });
    let label = lower == "bearer"
        || sensitive
            && assignment.is_some_and(|(_, value)| value.trim_matches(['"', '\'']).is_empty());
    let visible = if credential_next || word.contains("://") || sensitive && !label {
        "[REDACTED]"
    } else {
        word
    };
    (visible.to_owned(), label)
}

/// The start of `text`, at most `limit` characters, marked when cut.
pub fn clip(text: &str, limit: usize) -> String {
    let text = text.trim_end();
    match text.char_indices().nth(limit) {
        Some((end, _)) => format!("{}\n…[truncated]", &text[..end]),
        None => text.to_owned(),
    }
}

/// The end of `text`, at most `limit` characters, marked when cut: of a
/// prompt, the last part is what differs from turn to turn.
pub fn clip_tail(text: &str, limit: usize) -> String {
    let text = text.trim();
    let count = text.chars().count();
    if count <= limit {
        return text.to_owned();
    }
    let start = text
        .char_indices()
        .nth(count - limit)
        .map_or(0, |(start, _)| start);
    format!("[truncated]…\n{}", &text[start..])
}
