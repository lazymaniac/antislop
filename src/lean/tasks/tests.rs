use super::*;
use serde_json::json;

fn verdict_with(coverage: serde_json::Map<String, serde_json::Value>) -> Verdict {
    Verdict {
        approved: false,
        findings: vec![],
        coverage,
    }
}

#[path = "tests/group_1.rs"]
mod group_1;
#[path = "tests/group_2.rs"]
mod group_2;
#[path = "tests/group_3.rs"]
mod group_3;
#[path = "tests/group_4.rs"]
mod group_4;
#[path = "tests/group_5.rs"]
mod group_5;
#[path = "tests/group_6.rs"]
mod group_6;
#[path = "tests/group_7.rs"]
mod group_7;
