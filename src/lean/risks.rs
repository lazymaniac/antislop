//! The risk catalog of design §19, shared out among the kinds of work: each
//! entry belongs to the kind whose verifier can settle it, so a verifier
//! accounts only for the risks of its worker's domain.

use crate::config::WorkKind;
use std::sync::OnceLock;

/// `id<TAB>kind<TAB>problem<TAB>how to detect it`, one risk per line.
const CATALOGS: &[&str] = &[
    include_str!("risks/analysis.tsv"),
    include_str!("risks/architecture.tsv"),
    include_str!("risks/context.tsv"),
    include_str!("risks/delivery.tsv"),
    include_str!("risks/dependencies.tsv"),
    include_str!("risks/implementation.tsv"),
    include_str!("risks/integration.tsv"),
    include_str!("risks/options.tsv"),
    include_str!("risks/orchestration.tsv"),
    include_str!("risks/performance.tsv"),
    include_str!("risks/planning.tsv"),
    include_str!("risks/refactoring.tsv"),
    include_str!("risks/requirements.tsv"),
    include_str!("risks/research.tsv"),
    include_str!("risks/security.tsv"),
    include_str!("risks/static_analysis.tsv"),
    include_str!("risks/test_design.tsv"),
    include_str!("risks/testing.tsv"),
    include_str!("risks/writing.tsv"),
];

struct Risk {
    id: &'static str,
    kind: WorkKind,
    text: String,
}

fn risks() -> &'static [Risk] {
    static RISKS: OnceLock<Vec<Risk>> = OnceLock::new();
    RISKS.get_or_init(|| {
        CATALOGS
            .iter()
            .flat_map(|catalog| catalog.lines())
            .map(|line| {
                let mut fields = line.split('\t');
                let (id, kind, problem, detect) = (
                    fields.next().unwrap_or_default(),
                    fields.next().unwrap_or_default(),
                    fields.next().unwrap_or_default(),
                    fields.next().unwrap_or_default(),
                );
                Risk {
                    id,
                    kind: serde_json::from_value(kind.into())
                        .unwrap_or_else(|_| panic!("risk catalog: unknown kind {kind:?} for {id}")),
                    text: format!("{problem} How to check: {detect}"),
                }
            })
            .collect()
    })
}

/// The catalog's risks for one kind of work, as `(id, text)`.
pub fn for_kind(kind: WorkKind) -> impl Iterator<Item = (String, String)> {
    risks()
        .iter()
        .filter(move |risk| risk.kind == kind)
        .map(|risk| (risk.id.to_owned(), risk.text.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const KINDS: [WorkKind; 19] = {
        use WorkKind::*;
        [
            Orchestration,
            Context,
            Requirements,
            Options,
            Planning,
            Architecture,
            Dependencies,
            Implementation,
            Refactoring,
            StaticAnalysis,
            TestDesign,
            Testing,
            Security,
            Performance,
            Integration,
            Research,
            Writing,
            Analysis,
            Delivery,
        ]
    };

    #[test]
    fn every_risk_has_exactly_one_kind_and_every_kind_has_risks() {
        let ids: BTreeSet<_> = risks().iter().map(|risk| risk.id).collect();
        assert_eq!(ids.len(), risks().len(), "ids are unique");
        let shared: usize = KINDS.iter().map(|kind| for_kind(*kind).count()).sum();
        assert_eq!(shared, risks().len(), "no risk is left out or doubled");
        for kind in KINDS {
            assert!(for_kind(kind).next().is_some(), "{kind:?} has no risks");
        }
    }
}
