//! In-process proof of full prefix authentication. There is deliberately no
//! deserializer or public field constructor: IPC packages are untrusted data.
use super::*;

pub(crate) struct Set {
    run_hash: String,
    packages_hash: String,
    packages: BTreeMap<String, Pending>,
}
impl Set {
    pub fn build(run: &Run) -> Result<Self> {
        let packages = pending(run);
        sources_current(run)?;
        Ok(Self {
            run_hash: hash(serde_json::to_vec(run)?),
            packages_hash: hash(serde_json::to_vec(&packages)?),
            packages,
        })
    }
    pub fn packages(&self) -> &BTreeMap<String, Pending> {
        &self.packages
    }
    /// Package equality only; this does not establish a run binding or current
    /// sources. Callers must retain the immutable owning view or validate_current.
    pub fn packages_match(&self, packages: &BTreeMap<String, Pending>) -> Result<bool> {
        Ok(self.packages_hash == hash(serde_json::to_vec(packages)?))
    }
    pub fn validate_current(&self, run: &Run) -> Result<()> {
        require(
            self.run_hash == hash(serde_json::to_vec(run)?),
            "authenticated management state changed",
        )?;
        sources_current(run)
    }
    pub fn build_for_packages(run: &Run, packages: &BTreeMap<String, Pending>) -> Result<Self> {
        let proof = Self::build(run)?;
        require(
            proof.packages_match(packages)?,
            "management recovery packages differ from captured scope",
        )?;
        Ok(proof)
    }
    pub(super) fn select(
        &self,
        run: &Run,
        round_id: &str,
        kind: Kind,
        scope: &str,
    ) -> Result<Pending> {
        self.validate_current(run)?;
        self.packages
            .values()
            .find(|p| p.round_id == round_id && p.kind == kind && p.scope_hash == scope)
            .cloned()
            .ok_or_else(|| crate::Error::Invalid("management recovery scope mismatch".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> (tempfile::TempDir, Run) {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("source"), "original").unwrap();
        let mut spec: RunSpec =
            serde_json::from_str(include_str!("../../../examples/fake-spec.json")).unwrap();
        spec.workspace = root.path().to_string_lossy().into();
        spec.input_files = vec!["source".into()];
        let mut run = Run::new(id(), spec).unwrap();
        let round: Round = serde_json::from_value(json!({"id":"round","spec_revision":run.spec_revision,"source_hashes":{},"input_hash":hash("inputs"),"approval_scope":hash("scope"),"previous_status":"blocked","message_ids":[],"request_sequence":0,"request_context":"{}","status":"failed","executor_session":null,"verifier_session":null,"output":null,"review":null,"diagnostic":null})).unwrap();
        run.orchestration_rounds.push(round);
        (root, run)
    }
    #[test]
    fn authenticated_view_is_invalidated_by_any_canonical_change_or_live_source_change() {
        let (root, run) = fixture();
        let proof = Set::build(&run).unwrap();
        proof.validate_current(&run).unwrap();
        assert!(proof.packages_match(proof.packages()).unwrap());
        Set::build_for_packages(&run, proof.packages()).unwrap();
        for mutation in ["objective", "status", "history", "diagnostic"] {
            let mut changed = run.clone();
            match mutation {
                "objective" => changed.spec.objective.push_str(" different obligation"),
                "status" => changed.reconciliation_required = true,
                "history" => {
                    changed.orchestration_rounds[0].request_context = "changed original".into()
                }
                "diagnostic" => changed.diagnostic = Some("changed checkpoint".into()),
                _ => unreachable!(),
            }
            assert!(proof.validate_current(&changed).is_err(), "{mutation}");
        }
        std::fs::write(root.path().join("source"), "changed").unwrap();
        assert!(proof.validate_current(&run).is_err());
        std::fs::write(root.path().join("source"), "original").unwrap();
        proof.validate_current(&run).unwrap();
    }
    #[test]
    fn untrusted_package_data_cannot_turn_an_authenticated_empty_set_into_an_approval() {
        let (_root, run) = fixture();
        let proof = Set::build(&run).unwrap();
        assert!(proof.packages().is_empty());
        let mut forged = BTreeMap::new();
        forged.insert(
            "round:Replacement".into(),
            Pending {
                review_part: None,
                round_id: "round".into(),
                kind: Kind::Replacement,
                role: RoleKind::Verifier,
                previous_session: None,
                previous_turn: None,
                native_session_id: None,
                retained_executor: None,
                retained_output: None,
                retained_review: None,
                handoff: json!({"approved":true}),
                handoff_hash: hash("forged"),
                scope_hash: hash("forged scope"),
            },
        );
        assert!(!proof.packages_match(&forged).unwrap());
        assert!(Set::build_for_packages(&run, &forged).is_err());
        assert!(
            proof
                .select(&run, "round", Kind::Replacement, &hash("forged scope"))
                .is_err()
        );
    }
}
