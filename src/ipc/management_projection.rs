//! Derived, process-local management recovery documents. No cached result
//! authorizes an action; admission always rebuilds and validates its scope.
use crate::{Result, domain::Run, hash, runtime::orchestration_recovery};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub(crate) struct Document {
    pub text: String,
    pub hash: String,
    pub count: usize,
}
pub(crate) enum View {
    Preparing,
    Unavailable,
    Ready(Arc<Document>),
}
impl View {
    pub fn state(&self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Unavailable => "unavailable",
            Self::Ready(_) => "ready",
        }
    }
    pub fn document(&self) -> Option<&Document> {
        match self {
            Self::Ready(document) => Some(document),
            _ => None,
        }
    }
}
struct Entry {
    key: String,
    generation: String,
    result: Option<Option<Arc<Document>>>,
}
#[derive(Default)]
pub(crate) struct Cache(Mutex<BTreeMap<String, Entry>>);

impl Cache {
    pub fn view(
        self: &Arc<Self>,
        run: Run,
        run_document_hash: String,
        fresh_sources: bool,
    ) -> Result<View> {
        let eligible = !run.reconciliation_required
            && !matches!(
                run.status,
                crate::domain::Status::Running | crate::domain::Status::Paused
            )
            && run.orchestration_rounds.last().is_some_and(|r| {
                matches!(
                    r.status,
                    crate::orchestration::planning::RoundStatus::Failed
                        | crate::orchestration::planning::RoundStatus::Interrupted
                        | crate::orchestration::planning::RoundStatus::Rejected
                )
            });
        if !eligible {
            self.0.lock().expect("projection mutex").remove(&run.id);
            return Ok(View::Ready(Arc::new(document(
                &run_document_hash,
                BTreeMap::new(),
            )?)));
        }
        // Store::get compares actual database bytes. Do not trust state_hash as
        // the sole identity: an out-of-band row edit may leave it unchanged.
        let key = hash(serde_json::to_vec(&(&run, &run_document_hash))?);
        let existing = {
            let entries = self.0.lock().expect("projection mutex");
            entries
                .get(&run.id)
                .filter(|e| e.key == key)
                .map(|e| e.result.clone())
        };
        match existing {
            Some(None) => return Ok(View::Preparing),
            Some(Some(Some(document))) => {
                if !fresh_sources || orchestration_recovery::sources_current(&run).is_ok() {
                    return Ok(View::Ready(document));
                }
                self.remove(&run.id, &key);
                return Ok(View::Unavailable);
            }
            Some(Some(None)) => {
                self.remove(&run.id, &key);
                return Ok(View::Unavailable);
            }
            None => {}
        }
        let generation = crate::id();
        {
            let mut entries = self.0.lock().expect("projection mutex");
            // Concurrent first snapshots must share the same validation job.
            if entries.get(&run.id).is_some_and(|e| e.key == key) {
                return Ok(View::Preparing);
            }
            entries.insert(
                run.id.clone(),
                Entry {
                    key: key.clone(),
                    generation: generation.clone(),
                    result: None,
                },
            );
        }
        let cache = Arc::clone(self);
        std::thread::spawn(move || {
            let result = (|| -> Result<Document> {
                let packages = orchestration_recovery::pending(&run);
                // Prefix validation may be slow. Re-observe sources before
                // publishing, not just before starting that validation.
                orchestration_recovery::sources_current(&run)?;
                document(&run_document_hash, packages)
            })()
            .ok()
            .map(Arc::new);
            let mut entries = cache.0.lock().expect("projection mutex");
            if let Some(entry) = entries.get_mut(&run.id)
                && entry.key == key
                && entry.generation == generation
            {
                entry.result = Some(result);
            }
        });
        Ok(View::Preparing)
    }
    fn remove(&self, run_id: &str, key: &str) {
        let mut entries = self.0.lock().expect("projection mutex");
        if entries.get(run_id).is_some_and(|e| e.key == key) {
            entries.remove(run_id);
        }
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
