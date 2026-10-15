//! Factor repeated reviewer-authored methods/receipt IDs without sharing decisions.
//! The original native JSON remains the authenticated report; expansion is lossless.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Wire {
    #[serde(default)]
    agent_messages: Vec<crate::communication::outbox::Message>,
    contract_version: String,
    output_hash: String,
    spec_revision: u64,
    input_hash: String,
    #[serde(default)]
    proofs: Vec<Proof>,
    checks: Vec<Check>,
    findings: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    id: String,
    method: String,
    evidence_ids: Vec<String>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Check {
    Expanded(CheckResult),
    Referenced(Reference),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    check_id: String,
    status: CheckStatus,
    proof_id: String,
    rationale: String,
}
impl TryFrom<Wire> for ReviewResponse {
    type Error = String;
    fn try_from(wire: Wire) -> std::result::Result<Self, Self::Error> {
        let mut proofs = BTreeMap::new();
        for proof in wire.proofs {
            if proof.id.trim().is_empty()
                || proof.method.trim().is_empty()
                || proof.evidence_ids.is_empty()
                || proofs.contains_key(&proof.id)
            {
                return Err("duplicate or empty shared review proof".into());
            }
            proofs.insert(proof.id.clone(), proof);
        }
        let mut used = BTreeSet::new();
        let checks = wire
            .checks
            .into_iter()
            .map(|check| match check {
                Check::Expanded(check) => Ok(check),
                Check::Referenced(reference) => {
                    let proof = proofs
                        .get(&reference.proof_id)
                        .ok_or_else(|| "unknown shared review proof".to_string())?;
                    used.insert(reference.proof_id);
                    Ok(CheckResult {
                        check_id: reference.check_id,
                        status: reference.status,
                        method: proof.method.clone(),
                        evidence_ids: proof.evidence_ids.clone(),
                        rationale: reference.rationale,
                    })
                }
            })
            .collect::<std::result::Result<Vec<_>, String>>()?;
        if used.len() != proofs.len() {
            return Err("unused shared review proof".into());
        }
        Ok(Self {
            agent_messages: wire.agent_messages,
            contract_version: wire.contract_version,
            output_hash: wire.output_hash,
            spec_revision: wire.spec_revision,
            input_hash: wire.input_hash,
            checks,
            findings: wire.findings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Value {
        json!({"contract_version":"orchestration-review-v2","output_hash":"hash",
            "spec_revision":1,"input_hash":"input","findings":[],
            "proofs":[{"id":"read","method":"Inspect exact current inputs and output","evidence_ids":["input-receipt","output-receipt"]}],
            "checks":[{"check_id":"V01","status":"pass","proof_id":"read","rationale":"Identity matched"},
                {"check_id":"V02","status":"blocked","proof_id":"read","rationale":"A requirement remains unresolved"}]})
    }

    #[test]
    fn factoring_shares_only_method_and_evidence_never_a_decision() {
        let parsed: ReviewResponse = serde_json::from_value(report()).unwrap();
        assert_eq!(parsed.checks[0].status, CheckStatus::Pass);
        assert_eq!(parsed.checks[1].status, CheckStatus::Blocked);
        assert_ne!(parsed.checks[0].rationale, parsed.checks[1].rationale);
        assert_eq!(parsed.checks[0].method, parsed.checks[1].method);
        let legacy = serde_json::to_value(parsed).unwrap();
        let parsed: ReviewResponse = serde_json::from_value(legacy).unwrap();
        assert_eq!(
            parsed.checks[0].evidence_ids,
            ["input-receipt", "output-receipt"]
        );
    }

    #[test]
    fn ambiguous_unused_unknown_and_empty_proofs_are_refused() {
        let mut invalid = vec![];
        let mut r = report();
        r["proofs"][0]["method"] = json!(" ");
        invalid.push(r);
        let mut r = report();
        r["proofs"][0]["id"] = json!("");
        invalid.push(r);
        let mut r = report();
        r["checks"][0]["method"] = json!("override");
        invalid.push(r);
        let mut r = report();
        r["checks"][0]["status"] = json!("unknown");
        invalid.push(r);
        let mut r = report();
        r["checks"][0].as_object_mut().unwrap().remove("rationale");
        invalid.push(r);
        let mut r = report();
        r["checks"] = json!([]);
        invalid.push(r);
        let mut r = report();
        r["extra"] = json!(true);
        invalid.push(r);
        for value in invalid {
            assert!(serde_json::from_value::<ReviewResponse>(value).is_err());
        }
    }
}
