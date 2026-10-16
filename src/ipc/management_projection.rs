//! Derived, process-local management recovery documents. No cached result
//! authorizes an action; admission checks the entire canonical state, live
//! sources and explicit approval against an internally authenticated package.
use crate::{
    Result,
    domain::Run,
    hash,
    runtime::{orchestration_recovery, team_acceptance},
};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};

pub(crate) struct Document {
    pub text: String,
    pub hash: String,
    pub count: usize,
    authenticated: Option<Arc<orchestration_recovery::authenticated::Set>>,
    pub prepared_team: Option<Arc<team_acceptance::Prepared>>,
}
pub(crate) type View = super::projection::View<Document>;
pub(crate) type Cache = super::projection::Cache<Document>;

impl Cache {
    pub fn view(
        self: &Arc<Self>,
        run: Run,
        run_document_hash: String,
        fresh_sources: bool,
    ) -> Result<View> {
        let reviewed_team = team_acceptance::reviewed_round(&run).is_some();
        let eligible = !run.reconciliation_required
            && !matches!(
                run.status,
                crate::domain::Status::Running | crate::domain::Status::Paused
            )
            && (reviewed_team
                || run.orchestration_rounds.last().is_some_and(|r| {
                    matches!(
                        r.status,
                        crate::orchestration::planning::RoundStatus::Failed
                            | crate::orchestration::planning::RoundStatus::Interrupted
                            | crate::orchestration::planning::RoundStatus::Rejected
                    )
                }));
        if !eligible {
            self.clear(&run.id);
            return Ok(View::Ready(Arc::new(document(
                &run_document_hash,
                BTreeMap::new(),
            )?)));
        }
        // Store compares actual bytes, including out-of-band edits which left
        // the stored state_hash unchanged. Sealed proofs retain the full run.
        let key = hash(serde_json::to_vec(&(&run, &run_document_hash))?);
        let captured = run.clone();
        let view = self.prepare(run.id.clone(), key.clone(), move || {
            if reviewed_team {
                let prepared = Arc::new(team_acceptance::Prepared::build(&captured)?);
                orchestration_recovery::sources_current(&captured)?;
                let mut document = document(&run_document_hash, BTreeMap::new())?;
                document.prepared_team = Some(prepared);
                return Ok(document);
            }
            let authenticated = Arc::new(orchestration_recovery::authenticated::Set::build(
                &captured,
            )?);
            orchestration_recovery::sources_current(&captured)?;
            let mut document = document(&run_document_hash, authenticated.packages().clone())?;
            document.authenticated = Some(authenticated);
            Ok(document)
        });
        if matches!(view, View::Unavailable)
            || matches!(&view, View::Ready(_) if fresh_sources && orchestration_recovery::sources_current(&run).is_err())
        {
            self.remove(&run.id, &key);
            return Ok(View::Unavailable);
        }
        Ok(view)
    }
    pub fn authenticated(
        &self,
        run: &Run,
        run_document_hash: &str,
    ) -> Result<Option<Arc<orchestration_recovery::authenticated::Set>>> {
        let key = hash(serde_json::to_vec(&(run, run_document_hash))?);
        Ok(self
            .peek(&run.id, &key)
            .and_then(|d| d.authenticated.clone()))
    }
    pub fn prepared_team(
        &self,
        run: &Run,
        run_document_hash: &str,
    ) -> Result<Option<Arc<team_acceptance::Prepared>>> {
        let key = hash(serde_json::to_vec(&(run, run_document_hash))?);
        Ok(self
            .peek(&run.id, &key)
            .and_then(|d| d.prepared_team.clone()))
    }
}
fn document(
    run_document_hash: &str,
    packages: BTreeMap<String, orchestration_recovery::Pending>,
) -> Result<Document> {
    let count = packages.len();
    let text =
        serde_json::to_string(&json!({"run_document_hash":run_document_hash,"packages":packages}))?;
    Ok(Document {
        hash: hash(&text),
        text,
        count,
        authenticated: None,
        prepared_team: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{domain::RunSpec, orchestration::planning::Round};
    use std::time::{Duration, Instant};

    fn fixture() -> (tempfile::TempDir, Run) {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("source"), "original").unwrap();
        let mut spec: RunSpec =
            serde_json::from_str(include_str!("../../examples/fake-spec.json")).unwrap();
        spec.workspace = root.path().to_string_lossy().into();
        spec.input_files = vec!["source".into()];
        let mut run = Run::new(crate::id(), spec).unwrap();
        run.status = crate::domain::Status::Blocked;
        let round: Round = serde_json::from_value(json!({
            "id":"round","spec_revision":run.spec_revision,"source_hashes":{},"input_hash":hash("inputs"),
            "approval_scope":hash("scope"),"previous_status":"blocked","message_ids":[],"request_sequence":0,
            "request_context":"{}","status":"failed","executor_session":null,"verifier_session":null,
            "output":null,"review":null,"diagnostic":null,
        })).unwrap();
        run.orchestration_rounds.push(round);
        (root, run)
    }
    fn ready(cache: &Arc<Cache>, run: &Run) -> Arc<Document> {
        let start = Instant::now();
        loop {
            match cache
                .view(run.clone(), "retained-db-hash".into(), true)
                .unwrap()
            {
                View::Ready(document) => return document,
                View::Preparing => {}
                View::Unavailable => panic!("fixture sources unexpectedly unavailable"),
            }
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn immutable_pages_reuse_one_document_but_canonical_changes_with_the_same_db_hash_do_not() {
        let (_root, run) = fixture();
        let cache = Arc::new(Cache::default());
        assert!(matches!(
            cache
                .view(run.clone(), "retained-db-hash".into(), true)
                .unwrap(),
            View::Preparing
        ));
        let first = ready(&cache, &run);
        let next = ready(&cache, &run);
        assert!(Arc::ptr_eq(&first, &next));
        let mut changed = run.clone();
        changed.diagnostic = Some("a canonical change without updating database state_hash".into());
        assert!(matches!(
            cache
                .view(changed.clone(), "retained-db-hash".into(), true)
                .unwrap(),
            View::Preparing
        ));
        let after = ready(&cache, &changed);
        assert!(!Arc::ptr_eq(&first, &after));
        assert_eq!(first.hash, hash(&first.text));
    }
    #[test]
    fn publication_reobserves_live_sources_and_a_later_restore_can_rebuild() {
        let (root, run) = fixture();
        let cache = Arc::new(Cache::default());
        let before = serde_json::to_vec(&run).unwrap();
        let first = ready(&cache, &run);
        std::fs::write(root.path().join("source"), "changed").unwrap();
        assert!(matches!(
            cache
                .view(run.clone(), "retained-db-hash".into(), true)
                .unwrap(),
            View::Unavailable
        ));
        std::fs::write(root.path().join("source"), "original").unwrap();
        let restored = ready(&cache, &run);
        assert!(!Arc::ptr_eq(&first, &restored));
        assert_eq!(first.text, restored.text);
        assert_eq!(serde_json::to_vec(&run).unwrap(), before);
    }
}
