//! `antislop export`: a conversation's history as JSON lines on stdout, for
//! diagnosis the user controls (design §15). Secrets are redacted on a best
//! effort basis; the safer measure is that little is collected (design §13).

use super::cli::Options;
use super::daemon::{Client, Request};
use crate::{Result, require};
use serde_json::{Value, json};
use std::io::Write;

const REDACTED: &str = "[redacted]";

/// Prefixes of well-known token formats.
const TOKEN_PREFIXES: [&str; 12] = [
    "sk-",
    "sk_live_",
    "ghp_",
    "gho_",
    "ghs_",
    "ghu_",
    "github_pat_",
    "glpat-",
    "xoxb-",
    "xoxp-",
    "AKIA",
    "AIza",
];

/// Names that make the value after `=` or `:` a secret.
const SECRET_NAMES: [&str; 6] = ["password", "passwd", "secret", "token", "api_key", "apikey"];

pub fn export(args: &[String]) -> Result<()> {
    let options = Options::parse(args)?;
    require(options.message.is_none(), "export takes no message")?;
    let client = Client::connect(&options.project, &options.config, &options.state)?;
    let conversation = match &options.conversation {
        Some(id) => id.clone(),
        None => options
            .latest(&client)?
            .ok_or_else(|| crate::Error::Invalid("no conversation in this project".into()))?,
    };
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{}",
        json!({
            "export": "antislop",
            "version": env!("CARGO_PKG_VERSION"),
            "project": options.project,
            "conversation": conversation,
        })
    )?;
    let mut after = 0;
    loop {
        let reply = client.call(&Request::Events {
            conversation: conversation.clone(),
            after,
            wait_ms: 0,
        })?;
        let page = reply["events"].as_array().cloned().unwrap_or_default();
        let Some(last) = page.last() else { break };
        after = last["seq"].as_u64().unwrap_or(u64::MAX);
        for event in page {
            writeln!(out, "{}", redact_value(event))?;
        }
    }
    out.flush()?;
    Ok(())
}

fn redact_value(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact(&text)),
        Value::Array(items) => Value::Array(items.into_iter().map(redact_value).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, redact_value(value)))
                .collect(),
        ),
        other => other,
    }
}

/// Text with private keys, bearer tokens, well-known token formats and
/// `password=…`-like values replaced.
pub fn redact(text: &str) -> String {
    let text = private_keys(text);
    let mut out = String::with_capacity(text.len());
    let mut secret_next = false;
    let mut rest = text.as_str();
    while !rest.is_empty() {
        let space = rest.len() - rest.trim_start().len();
        out.push_str(&rest[..space]);
        rest = &rest[space..];
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let word = &rest[..end];
        rest = &rest[end..];
        let (redacted, secret) = redact_word(word, secret_next);
        out.push_str(&redacted);
        secret_next = secret;
    }
    out
}

/// The word redacted, and whether the next one is a secret (`Bearer x`,
/// `password: x`).
fn redact_word(word: &str, secret: bool) -> (String, bool) {
    let quotes: &[char] = &['"', '\'', '`', ',', ';', '(', ')', '[', ']', '{', '}'];
    let core = word.trim_matches(quotes);
    if core.is_empty() {
        return (word.to_owned(), false);
    }
    let start = word.find(core).unwrap_or(0);
    let (before, after) = (&word[..start], &word[start + core.len()..]);
    let token = secret
        || TOKEN_PREFIXES
            .iter()
            .any(|prefix| core.starts_with(prefix) && core.len() >= prefix.len() + 12);
    if token {
        return (format!("{before}{REDACTED}{after}"), false);
    }
    if let Some(at) = core.find(['=', ':']) {
        let name = core[..at].trim_matches(quotes).to_ascii_lowercase();
        let value = core[at + 1..].trim_matches(quotes);
        if SECRET_NAMES.iter().any(|secret| name.ends_with(secret)) {
            if value.is_empty() {
                return (word.to_owned(), true);
            }
            if !value.starts_with("//") {
                return (format!("{before}{}{REDACTED}{after}", &core[..=at]), false);
            }
        }
    }
    (word.to_owned(), core.eq_ignore_ascii_case("bearer"))
}

fn private_keys(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(begin) = rest.find("-----BEGIN ") {
        let Some(end) = rest[begin..].find("-----END ") else {
            break;
        };
        let end = begin + end;
        let close = rest[end + 9..]
            .find("-----")
            .map_or(rest.len(), |at| end + 9 + at + 5);
        out.push_str(&rest[..begin]);
        if rest[begin..close].contains("PRIVATE KEY") {
            out.push_str(REDACTED);
        } else {
            out.push_str(&rest[begin..close]);
        }
        rest = &rest[close..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_redacted_and_ordinary_text_kept() {
        assert_eq!(
            redact("export OPENAI_API_KEY=sk-abcdefghijklmnopqrstu and 'ghp_0123456789abcdefABCD'"),
            "export OPENAI_API_KEY=[redacted] and '[redacted]'"
        );
        assert_eq!(
            redact("Authorization: Bearer abc.def.ghijk\npassword: hunter2"),
            "Authorization: Bearer [redacted]\npassword: [redacted]"
        );
        assert_eq!(
            redact("a\n-----BEGIN RSA PRIVATE KEY-----\nMIIE\n-----END RSA PRIVATE KEY-----\nb"),
            "a\n[redacted]\nb"
        );
        assert_eq!(redact(r#"{"token": "abc"}"#), r#"{"token": "[redacted]"}"#);
        let ordinary = "src/token.rs:12 fn token() at https://example.com, sk-1 too short";
        assert_eq!(redact(ordinary), ordinary);
    }
}
