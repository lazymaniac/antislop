//! One immutable captured-run view per model/provenance validation. This is
//! derived data, never a serialized permission or a cache across run versions.
use crate::{Result, domain::Run, hash};
use std::collections::BTreeMap;

pub(crate) struct Resolver<'a> {
    run: &'a Run,
    canonical: BTreeMap<String, String>,
    members: Option<BTreeMap<String, String>>,
    checks: BTreeMap<(String, String), bool>,
}
impl<'a> Resolver<'a> {
    pub(crate) fn new(run: &'a Run) -> Self {
        Self::with_hashes(run, run.source_hashes())
    }
    pub(crate) fn with_hashes(run: &'a Run, canonical: BTreeMap<String, String>) -> Self {
        Self {
            run,
            canonical,
            members: None,
            checks: BTreeMap::new(),
        }
    }
    pub(crate) fn canonical(&self) -> &BTreeMap<String, String> {
        &self.canonical
    }
    fn members(&mut self) -> Result<&BTreeMap<String, String>> {
        if self.members.is_none() {
            self.members = Some(super::sources::current_member_hashes(self.run)?);
        }
        Ok(self.members.as_ref().unwrap())
    }
    fn runner(&mut self, reference: &str, expected: &str) -> bool {
        if !reference.starts_with("runner:") {
            return false;
        }
        let run = self.run;
        *self
            .checks
            .entry((reference.into(), expected.into()))
            .or_insert_with(|| {
                run.evidence.iter().any(|e| {
                    e.source == reference
                        && e.output_hash == expected
                        && hash(&e.output) == expected
                        && crate::runtime::team::checks::evidence_current(run, e)
                })
            })
    }
    pub(crate) fn matches(&mut self, reference: &str, expected: &str) -> Result<bool> {
        if let Some(actual) = self.canonical.get(reference) {
            return Ok(actual == expected);
        }
        if reference.starts_with("directory-member:") {
            return Ok(self
                .members()?
                .get(reference)
                .is_some_and(|h| h == expected));
        }
        Ok(self.runner(reference, expected))
    }
    pub(crate) fn exists(&mut self, reference: &str) -> Result<bool> {
        if self.canonical.contains_key(reference) {
            return Ok(true);
        }
        if reference.starts_with("directory-member:") {
            return Ok(self.members()?.contains_key(reference));
        }
        let run = self.run;
        Ok(reference.starts_with("runner:")
            && run
                .evidence
                .iter()
                .any(|e| e.source == reference && self.runner(reference, &e.output_hash)))
    }
}
