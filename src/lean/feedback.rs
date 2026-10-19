//! What the comment agent says about the comments it was given: which ones
//! its revised plan took care of. A comment it only answered stays open for
//! the user to settle.

use serde::Deserialize;

#[derive(Deserialize)]
struct Report {
    #[serde(default)]
    addressed: Vec<Addressed>,
}

#[derive(Deserialize)]
struct Addressed {
    id: u64,
}

/// Splits the ```comments block off an answer. Returns the answer without it
/// and, when the block was there and valid, the ids it lists as addressed.
pub fn split(answer: &str) -> (String, Option<Vec<u64>>) {
    let open = "```comments";
    let Some(start) = answer.rfind(open) else {
        return (answer.to_owned(), None);
    };
    let rest = &answer[start + open.len()..];
    if !rest.starts_with(['\n', '\r', ' ']) {
        return (answer.to_owned(), None);
    }
    let Some(end) = rest.find("```") else {
        return (answer.to_owned(), None);
    };
    let mut text = answer[..start].trim_end().to_owned();
    let after = rest[end + 3..].trim();
    if !after.is_empty() {
        text.push_str("\n\n");
        text.push_str(after);
    }
    let addressed = serde_json::from_str::<Report>(rest[..end].trim())
        .ok()
        .map(|report| report.addressed.into_iter().map(|a| a.id).collect());
    (text, addressed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_is_removed_and_its_ids_are_read() {
        let answer = "Revised.\n```comments\n{\"addressed\":[{\"id\":3,\"how\":\"x\"},{\"id\":5}]}\n```\n```plan\n{}\n```";
        let (text, ids) = split(answer);
        assert_eq!(ids, Some(vec![3, 5]));
        assert!(!text.contains("comments"));
        assert!(text.contains("Revised.") && text.contains("```plan"));
    }

    #[test]
    fn without_a_valid_block_nothing_is_claimed() {
        assert_eq!(split("just text"), ("just text".to_owned(), None));
        let (text, ids) = split("a\n```comments\nnot json\n```");
        assert_eq!((text.as_str(), ids), ("a", None));
        assert_eq!(
            split("x\n```comments\n{\"addressed\":[]}\n```").1,
            Some(vec![])
        );
    }
}
