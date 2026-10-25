use antislop::lean::ledger::parse_block;
use serde_json::{Value, json};

#[test]
fn tagged_json_fences_accept_the_format_requested_by_requirements_prompts() {
    for tag in ["ledger", "compliance", "compliance_verdict", "options"] {
        for suffix in ["", " JSON", " json", "\tJson"] {
            for newline in ["\n", "\r\n"] {
                let answer = format!(
                    "Result:{newline}  ```{tag}{suffix}  {newline}{{\"entries\":[]}}{newline}  ```{newline}"
                );
                assert_eq!(
                    parse_block::<Value>(&answer, tag).unwrap(),
                    json!({"entries": []})
                );
            }
        }
    }
}

#[test]
fn tagged_json_fences_require_the_requested_tag_and_a_complete_block() {
    for answer in [
        "```json\n{}\n```",
        "```ledger_extra\n{}\n```",
        "```ledger YAML\n{}\n```",
        "```ledger JSON\n{}",
    ] {
        assert!(parse_block::<Value>(answer, "ledger").is_err(), "{answer}");
    }
    assert!(parse_block::<Value>("```ledger JSON\ninvalid\n```", "ledger").is_err());
}

#[test]
fn tagged_json_fences_use_the_last_response_block() {
    let answer =
        "```ledger\n{\"revision\":1}\n```\nCorrection:\n```ledger JSON\n{\"revision\":2}\n```";
    assert_eq!(
        parse_block::<Value>(answer, "ledger").unwrap(),
        json!({"revision": 2})
    );
}
