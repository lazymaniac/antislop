//! Small role-specific schemas keep process control out of task prompts.
use serde_json::{Value, json};

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

fn tagged(kind: &str, mut fields: Value, required: &[&str]) -> Value {
    fields["type"] = json!({"const":kind});
    let mut required = required.to_vec();
    required.push("type");
    object(fields, &required)
}

pub(super) fn result(role: &str) -> Value {
    let pending = tagged(
        "needs_input",
        json!({"request":{"type":"string"},"checkpoint":{"type":"string"}}),
        &["request", "checkpoint"],
    );
    if role.starts_with("verifier:") {
        let assessment = object(
            json!({"status":{"enum":["ok","issue","blocked","n/a"]},"note":{"type":"string"}}),
            &["status", "note"],
        );
        let finding = object(
            json!({"id":{"type":"string"},"check":{"type":"string"},"severity":{"enum":["critical","high","medium","low"]},"issue":{"type":"string"},"where":{"type":"string"}}),
            &["id", "check", "severity", "issue", "where"],
        );
        let verdict = object(
            json!({"approved":{"type":"boolean"},"findings":{"type":"array","items":finding},"coverage":{"type":"object","additionalProperties":assessment}}),
            &["approved", "findings", "coverage"],
        );
        let review = tagged(
            "review",
            json!({"report":{"type":"string"},"verdict":verdict}),
            &["report", "verdict"],
        );
        return json!({"oneOf":[review,pending]});
    }
    if role.starts_with("worker:") {
        let alternative = object(
            json!({"id":{"type":"string"},"approach":{"type":"string"},"evaluations":{"type":"array","items":{"type":"string"}},"eligible":{"type":"boolean"},"tradeoffs":{"type":"string"}}),
            &["id", "approach", "evaluations", "eligible", "tradeoffs"],
        );
        let selection = object(
            json!({"criteria":{"type":"array","items":{"type":"string"}},"alternatives":{"type":"array","minItems":2,"maxItems":4,"items":alternative},"selected":{"type":"string"},"rationale":{"type":"string"},"reconsider_when":{"type":"string"}}),
            &[
                "criteria",
                "alternatives",
                "selected",
                "rationale",
                "reconsider_when",
            ],
        );
        let completion = tagged(
            "completed",
            json!({"summary":{"type":"string"},"report":{"type":"string"},"outputs":{"type":"object","additionalProperties":{"type":"string"}},"selection":selection}),
            &["summary", "report", "outputs"],
        );
        return json!({"oneOf":[completion,pending]});
    }
    tagged("report", json!({"text":{"type":"string"}}), &["text"])
}

pub(super) fn revision() -> Value {
    let strings = json!({"type":"array","items":{"type":"string"}});
    let input = object(
        json!({"name":{"type":"string"},"task":{"type":"string"},"output":{"type":"string"}}),
        &["name", "task", "output"],
    );
    let verifier = object(
        json!({"focus":{"type":"string"},"summary":{"type":["string","null"]},"risks":strings,"risk_ids":strings}),
        &["focus"],
    );
    let contract = object(
        json!({
            "id":{"type":"string"},"title":{"type":"string"},"goal":{"type":"string"},
            "summary":{"type":["string","null"]},"section":{"type":["string","null"]},
            "kind":{"type":"string"},"writes":{"type":"boolean"},"scope":strings,"depends_on":strings,
            "requirement_ids":strings,"criterion_ids":strings,"outputs":strings,
            "inputs":{"type":"array","items":input},"verifier":verifier,
            "quality_gate":{"const":false},"options_for":{"type":"null"}
        }),
        &["id", "title", "goal", "kind", "verifier"],
    );
    object(
        json!({"plan":{"type":"string"},"base_revision":{"type":"integer","minimum":0},"reason":{"type":"string"},"tasks":{"type":"array","items":contract},"retire":strings}),
        &["plan", "base_revision", "reason", "tasks", "retire"],
    )
}
