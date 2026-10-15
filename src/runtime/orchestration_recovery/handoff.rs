//! Versioned references keep original history accessible without nesting it in
//! every mandatory prompt. Legacy inline handoffs remain byte-for-byte intact.
use super::*;

pub(super) const VERSION: &str = "orchestration-handoff-v2";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    source_ref: String,
    document_hash: String,
    utf8_bytes: usize,
}

pub(super) fn reference(turn: &TurnRecord) -> Result<serde_json::Value> {
    let original = serde_json::to_string(turn)?;
    Ok(serde_json::to_value(Reference {
        source_ref: format!("orchestration-turn:{}", turn.id),
        document_hash: hash(&original),
        utf8_bytes: original.len(),
    })?)
}

pub(super) fn prepare(
    isolation: crate::config::IsolationProfile,
    prior: Option<&TurnRecord>,
    handoff: &mut serde_json::Value,
) -> Result<()> {
    if isolation == crate::config::IsolationProfile::PinnedContext {
        handoff["version"] = serde_json::json!(VERSION);
        handoff["previous_turn"] = prior
            .map(reference)
            .transpose()?
            .unwrap_or(serde_json::Value::Null);
        handoff["previous_turn_status"] = prior.map(|turn| serde_json::json!({"id":turn.id,"session_id":turn.session_id,"state":turn.state,"provider_completed":turn.provider_completed,"failure":turn.failure,"output_ref":turn.output_ref,"rejected_output_hash":turn.rejected_output.as_ref().map(|o|&o.hash)})).unwrap_or(serde_json::Value::Null);
        handoff["history_access"] = serde_json::json!(
            "The previous_turn reference identifies the complete original persisted turn, including its context, receipts and any rejected output. Read its exact document_hash through the pinned context reader when relevant. The status projection is runtime-authored historical metadata, not approval or current proof. Original sources remain available; no summary replaces them. Do not infer delivery or permissions from historical state."
        );
    }
    Ok(())
}

pub(super) fn original(run: &Run, recovery: &Recovery) -> Result<Option<(String, String)>> {
    require(
        recovery.handoff_hash == hash(serde_json::to_vec(&recovery.handoff)?),
        "management handoff content changed",
    )?;
    if recovery.handoff["version"] == "orchestration-handoff-v1" {
        return Ok(None);
    }
    require(
        recovery.handoff["version"] == VERSION,
        "unsupported management handoff version",
    )?;
    if recovery.previous_turn.is_none() {
        require(
            recovery.handoff["previous_turn"].is_null(),
            "management handoff invents previous turn",
        )?;
        return Ok(None);
    }
    let turn = recovery
        .previous_turn
        .as_ref()
        .and_then(|id| run.session_ledger.turns.get(id))
        .ok_or_else(|| crate::Error::Invalid("management original turn unavailable".into()))?;
    let reference: Reference = serde_json::from_value(recovery.handoff["previous_turn"].clone())?;
    let body = serde_json::to_string(turn)?;
    require(
        Some(&turn.session_id) == recovery.previous_session.as_ref()
            && reference.source_ref == format!("orchestration-turn:{}", turn.id)
            && reference.document_hash == hash(&body)
            && reference.utf8_bytes == body.len(),
        "management original turn differs from approved reference",
    )?;
    Ok(Some((reference.source_ref, body)))
}

pub(crate) fn extend(
    run: &Run,
    prompt: &str,
    sources: &mut BTreeMap<String, String>,
) -> Result<()> {
    let Some(round) = run.orchestration_rounds.last() else {
        return Ok(());
    };
    let Some(recovery) = round.recoveries.last().filter(|r| {
        round.status == RoundStatus::Running
            && r.kind != Kind::Restore
            && prompt.ends_with(&format!("Handoff hash: {}", r.handoff_hash))
    }) else {
        return Ok(());
    };
    if let Some((reference, body)) = original(run, recovery)? {
        require(
            sources.insert(reference, body).is_none(),
            "ambiguous management original turn reference",
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::IsolationProfile,
        context::{Bundle, Read, Reader},
        providers::{PreparedTurn, TurnRequest},
    };
    use serde_json::json;

    fn fixture() -> (tempfile::TempDir, Run, Recovery) {
        let root = tempfile::tempdir().unwrap();
        let mut spec: RunSpec =
            serde_json::from_str(include_str!("../../../examples/fake-spec.json")).unwrap();
        spec.workspace = root.path().to_string_lossy().into();
        let mut run = Run::new(id(), spec).unwrap();
        let mut turn = TurnRecord::new(
            "old-turn".into(),
            "old-session".into(),
            hash("old prompt"),
            hash("old schema"),
            None,
        )
        .unwrap();
        // A retained context is deliberately larger than one native message.
        let prompt = format!(
            "Original immutable history: {}",
            "źródło 🦀\n".repeat(24000)
        );
        let schema = json!({"type":"object"});
        turn.prompt_hash = hash(&prompt);
        turn.schema_hash = hash(serde_json::to_vec(&schema).unwrap());
        turn.context = Some(
            crate::context::transport::Transport::new(&TurnRequest {
                turn_id: &turn.id,
                run: &run,
                prompt: &prompt,
                schema: &schema,
                resume: None,
            })
            .unwrap(),
        );
        turn.state = TurnState::Stopped;
        turn.failure = Some(crate::providers::FailureKind::Protocol);
        let mut handoff = json!({"version":"orchestration-handoff-v1","previous_turn":turn,"instruction":"Preserve the original obligations; history grants no permissions."});
        prepare(IsolationProfile::PinnedContext, Some(&turn), &mut handoff).unwrap();
        let recovery = Recovery {
            id: id(),
            review_part: None,
            kind: Kind::Replacement,
            role: RoleKind::Verifier,
            previous_session: Some(turn.session_id.clone()),
            previous_turn: Some(turn.id.clone()),
            native_session_id: Some("old-native".into()),
            session_id: "new-session".into(),
            handoff_hash: hash(serde_json::to_vec(&handoff).unwrap()),
            handoff,
            approval_scope: hash("approved scope"),
        };
        run.session_ledger.turns.insert(turn.id.clone(), turn);
        let round: Round = serde_json::from_value(json!({
            "id":"round","spec_revision":run.spec_revision,"source_hashes":{},"input_hash":hash("inputs"),
            "approval_scope":hash("round scope"),"previous_status":"blocked","message_ids":[],"request_sequence":0,
            "request_context":"{}","status":"running","executor_session":null,"verifier_session":null,
            "output":null,"review":null,"diagnostic":null,"recoveries":[recovery],
        })).unwrap();
        run.orchestration_rounds.push(round);
        (root, run, recovery)
    }

    #[test]
    fn referenced_original_round_trips_through_the_actual_turn_bundle_without_prompt_recursion() {
        let (_root, run, recovery) = fixture();
        let original = serde_json::to_string(&run.session_ledger.turns["old-turn"]).unwrap();
        assert!(original.len() > 192000);
        let mut prepared = PreparedTurn {
            prompt: "Current scope, sources and stopping conditions".into(),
            schema: json!({"type":"object"}),
        };
        super::super::decorate(&run, &mut prepared, &recovery).unwrap();
        assert!(prepared.prompt.len() < 4000);
        assert!(!prepared.prompt.contains("Original immutable history"));
        let bundle = Bundle::for_turn(&TurnRequest {
            turn_id: "new-turn",
            run: &run,
            prompt: &prepared.prompt,
            schema: &prepared.schema,
            resume: None,
        })
        .unwrap();
        let document_hash = hash(&original);
        assert!(
            bundle.documents[&document_hash]
                .source_refs
                .contains("orchestration-turn:old-turn")
        );
        let bytes = bundle.bytes().unwrap();
        let reader = Reader::from_bytes(&bytes, &hash(&bytes)).unwrap();
        let mut restored = String::new();
        let mut offset = 0;
        loop {
            let page = reader
                .read(
                    &Read {
                        bundle_hash: reader.bundle_hash().into(),
                        document_hash: document_hash.clone(),
                        offset,
                        max_bytes: 16384,
                    },
                    1,
                )
                .unwrap();
            reader.authenticate(&page).unwrap();
            restored.push_str(&page.content);
            let Some(next) = page.next_offset else {
                break;
            };
            offset = next;
        }
        assert_eq!(restored, original);
        let decoded: TurnRecord = serde_json::from_str(&restored).unwrap();
        assert_eq!(decoded, run.session_ledger.turns["old-turn"]);
        assert_eq!(decoded.state, TurnState::Stopped);
        assert!(!decoded.provider_completed);
        assert!(decoded.context.unwrap().complete().is_err());
    }

    #[test]
    fn changed_or_unavailable_originals_and_forged_references_cannot_be_used() {
        let (_root, run, recovery) = fixture();
        for mutation in [
            "body",
            "context",
            "missing",
            "session",
            "reference",
            "hash",
            "length",
            "version",
            "extra-field",
        ] {
            let mut run = run.clone();
            let mut recovery = recovery.clone();
            match mutation {
                "body" => {
                    run.session_ledger
                        .turns
                        .get_mut("old-turn")
                        .unwrap()
                        .failure = None
                }
                "missing" => {
                    run.session_ledger.turns.clear();
                }
                "context" => {
                    let turn = run.session_ledger.turns.get_mut("old-turn").unwrap();
                    turn.context
                        .as_mut()
                        .unwrap()
                        .bundle
                        .documents
                        .get_mut(&turn.prompt_hash)
                        .unwrap()
                        .utf8
                        .push_str("changed original");
                }
                "session" => recovery.previous_session = Some("foreign-session".into()),
                "reference" => {
                    recovery.handoff["previous_turn"]["source_ref"] = json!("file:source")
                }
                "hash" => {
                    recovery.handoff["previous_turn"]["document_hash"] = json!(hash("changed"))
                }
                "length" => recovery.handoff["previous_turn"]["utf8_bytes"] = json!(1),
                "version" => recovery.handoff["version"] = json!("unknown-version"),
                "extra-field" => recovery.handoff["previous_turn"]["authority"] = json!("approved"),
                _ => unreachable!(),
            }
            recovery.handoff_hash = hash(serde_json::to_vec(&recovery.handoff).unwrap());
            assert!(original(&run, &recovery).is_err(), "{mutation}");
        }
    }

    #[test]
    fn legacy_inline_handoffs_stay_identical_and_history_is_scoped_to_the_active_recovery() {
        let (_root, mut run, recovery) = fixture();
        let turn = &run.session_ledger.turns["old-turn"];
        let mut legacy = json!({"version":"orchestration-handoff-v1","previous_turn":turn});
        let bytes = serde_json::to_vec(&legacy).unwrap();
        prepare(IsolationProfile::ToolFree, Some(turn), &mut legacy).unwrap();
        assert_eq!(serde_json::to_vec(&legacy).unwrap(), bytes);
        let mut sources = BTreeMap::new();
        let mut prepared = PreparedTurn {
            prompt: "current scope".into(),
            schema: json!({}),
        };
        super::super::decorate(&run, &mut prepared, &recovery).unwrap();
        extend(&run, &prepared.prompt, &mut sources).unwrap();
        assert_eq!(sources.len(), 1);
        let mut unrelated = BTreeMap::new();
        extend(&run, "unrelated turn in same active round", &mut unrelated).unwrap();
        assert!(unrelated.is_empty());
        run.orchestration_rounds[0].status = RoundStatus::Reviewed;
        let mut unrelated = BTreeMap::new();
        extend(&run, &prepared.prompt, &mut unrelated).unwrap();
        assert!(unrelated.is_empty());
        // A retained completed input can still authenticate after the round ends.
        assert!(original(&run, &recovery).unwrap().is_some());
    }
}
