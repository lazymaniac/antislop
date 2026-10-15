//! Read-only compatibility probe against an explicitly supplied owned fixture.
#[test]
#[ignore = "requires ANTISLOP_RECOVERY_PROJECTION_INPUT; no provider or mutation"]
fn retained_native_review_prefix_authenticates_with_its_original_prepared_inputs() {
    let path = std::env::var_os("ANTISLOP_RECOVERY_PROJECTION_INPUT")
        .expect("explicit retained fixture required");
    let bytes = std::fs::read(path).unwrap();
    let run: architect::domain::Run = serde_json::from_slice(&bytes).unwrap();
    let before = serde_json::to_vec(&run).unwrap();
    let round = run.orchestration_rounds.last().unwrap();
    let response = architect::transport::decode(round.output.as_ref().unwrap().as_bytes()).unwrap();
    let completed = round
        .review_parts
        .iter()
        .take_while(|p| p.output.is_some())
        .count();
    assert!(completed > 0, "fixture must contain real completed prefix");
    let pending = architect::orchestration::planning::parts::recovery_part(&run, round, &response)
        .unwrap()
        .unwrap();
    assert_eq!(pending.id, round.review_parts[completed].id);
    assert_eq!(serde_json::to_vec(&run).unwrap(), before);
    println!(
        "Authenticated {completed} retained native reports against exact original prepared inputs; no provider launch or state write."
    );
}
