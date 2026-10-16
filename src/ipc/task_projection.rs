//! Read-only task recovery projection. No cached package grants mutation
//! authority: runtime admission still authenticates the actual current task.
use crate::{
    Result,
    domain::{Run, Status},
    hash, require,
};
use serde_json::{Value, json};
use std::sync::Arc;

pub(crate) struct Document {
    pub text: String,
    pub hash: String,
    pub packages: Value,
    source_fingerprint: String,
}
impl Document {
    pub fn count(&self, kind: &str) -> usize {
        self.packages[kind].as_object().map_or(0, |v| v.len())
    }
}
pub(crate) type Cache = super::projection::Cache<Document>;
pub(crate) type View = super::projection::View<Document>;

fn document(
    run_document_hash: &str,
    packages: Value,
    source_fingerprint: String,
) -> Result<Document> {
    let mut value = packages.clone();
    value["run_document_hash"] = json!(run_document_hash);
    let text = serde_json::to_string(&value)?;
    Ok(Document {
        hash: hash(&text),
        text,
        packages,
        source_fingerprint,
    })
}
fn empty() -> Value {
    json!({"result_restores":{},"effect_reconciliations":{},"wave_effect_reconciliations":{},"native_resumes":{},"replacements":{},"check_restores":{},"source_refreshes":{},"verification_retries":{}})
}
impl Cache {
    pub fn view(
        self: &Arc<Self>,
        run: Run,
        run_document_hash: String,
        fresh_sources: bool,
    ) -> Result<View> {
        if run.team.is_none()
            || matches!(
                run.status,
                Status::Running | Status::Paused | Status::Completed | Status::Cancelled
            )
        {
            self.clear(&run.id);
            return Ok(View::Ready(Arc::new(document(
                &run_document_hash,
                empty(),
                String::new(),
            )?)));
        }
        let key = hash(serde_json::to_vec(&(&run, &run_document_hash))?);
        let captured = run.clone();
        let view = self.prepare(run.id.clone(),key.clone(),move || {
            use crate::runtime::team::{adjudicate,check_recovery,replacement,resume,restore};
            let fingerprint = crate::recovery::observe(&captured)?.hash;
            let packages = json!({
                "result_restores":restore::scopes(&captured),
                "effect_reconciliations":adjudicate::scopes(&captured),
                "wave_effect_reconciliations":adjudicate::pending_waves(&captured),
                "native_resumes":resume::pending(&captured),
                "replacements":replacement::pending(&captured),
                "check_restores":check_recovery::pending(&captured),
                "source_refreshes":captured.team.as_ref().unwrap().tasks.iter().filter(|t|t.state==crate::team::TaskState::Stale)
                    .filter_map(|t| captured.task_source_refresh_scope(&t.id).ok().map(|s|(t.id.clone(),s))).collect::<std::collections::BTreeMap<_,_>>(),
                "verification_retries":captured.team.as_ref().unwrap().tasks.iter().filter(|t|t.state==crate::team::TaskState::Blocked)
                    .filter_map(|t| captured.task_verification_retry_scope(&t.id).ok().map(|s|(t.id.clone(),s))).collect::<std::collections::BTreeMap<_,_>>(),
            });
            require(crate::recovery::observe(&captured)?.hash == fingerprint,
                "task recovery sources changed during preparation")?;
            document(&run_document_hash,packages,fingerprint)
        });
        if matches!(view, View::Unavailable)
            || matches!(&view,View::Ready(d) if fresh_sources && crate::recovery::observe(&run).map_or(true, |o| o.hash != d.source_fingerprint))
        {
            self.remove(&run.id, &key);
            return Ok(View::Unavailable);
        }
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    fn ready(cache: &Arc<Cache>, run: &Run) -> Arc<Document> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match cache
                .view(run.clone(), "same-db-metadata".into(), true)
                .unwrap()
            {
                View::Ready(d) => return d,
                View::Unavailable => panic!("fixture recovery unexpectedly unavailable"),
                View::Preparing => (),
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn immutable_task_pages_bind_actual_run_bytes_and_live_source_versions() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("source"), "original").unwrap();
        let mut spec: crate::domain::RunSpec =
            serde_json::from_str(include_str!("../../examples/fake-spec.json")).unwrap();
        spec.workspace = root.path().to_string_lossy().into();
        spec.input_files = vec!["source".into()];
        let mut run = Run::new(crate::id(), spec).unwrap();
        run.team = Some(crate::team::TeamPlan::bootstrap(run.spec.task_family, &run.spec).unwrap());
        run.status = Status::Blocked;
        let before = serde_json::to_vec(&run).unwrap();
        let cache = Arc::new(Cache::default());
        assert!(matches!(
            cache
                .view(run.clone(), "same-db-metadata".into(), true)
                .unwrap(),
            View::Preparing
        ));
        let first = ready(&cache, &run);
        assert!(Arc::ptr_eq(&first, &ready(&cache, &run)));
        assert_eq!(first.hash, hash(&first.text));
        std::fs::write(root.path().join("source"), "changed").unwrap();
        assert!(matches!(
            cache
                .view(run.clone(), "same-db-metadata".into(), true)
                .unwrap(),
            View::Unavailable
        ));
        std::fs::write(root.path().join("source"), "original").unwrap();
        assert!(!Arc::ptr_eq(&first, &ready(&cache, &run)));
        assert_eq!(before, serde_json::to_vec(&run).unwrap());
        run.diagnostic = Some("canonical bytes changed without changing db metadata".into());
        assert!(matches!(
            cache
                .view(run.clone(), "same-db-metadata".into(), true)
                .unwrap(),
            View::Preparing
        ));
        assert!(!Arc::ptr_eq(&first, &ready(&cache, &run)));
    }
}
