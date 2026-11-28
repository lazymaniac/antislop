//! Reproduced findings stay open until there is a matching passing observation.
use super::*;
use crate::lean::tasks::ScenarioEvidence;
use std::collections::BTreeMap;

impl Core {
    pub(super) fn check_review_evidence(
        &self,
        conversation: &str,
        turn: &str,
        task: &Task,
        verdict: &tasks::Verdict,
    ) -> Result<()> {
        // Preserve open findings across coordination and cancelled rounds.
        let previous = self.store.previous_task_review(conversation, task, turn)?;
        let previous = reproduced_findings(
            previous
                .as_ref()
                .map_or(&[], |(_, verdict)| verdict.findings.as_slice()),
        )?;
        let current = reproduced_findings(&verdict.findings)?;
        require(
            verdict
                .resolution_evidence
                .keys()
                .all(|id| previous.contains_key(id.as_str()) && verdict.resolved.contains(id)),
            "resolution_evidence: use only ids of earlier reproduced findings listed in resolved",
        )?;
        if current.is_empty() && previous.is_empty() {
            return Ok(());
        }
        let context = self
            .store
            .run_context(conversation, turn)?
            .ok_or_else(|| crate::invalid("review evidence requires a recorded checkout"))?;
        let root = PathBuf::from(git::ok(&context.cwd, &["rev-parse", "--show-toplevel"])?);
        let tree = super::validations::validation_tree(&root, true)?
            .ok_or_else(|| crate::invalid("review evidence requires a verifiable Git tree"))?;
        for (id, proof) in &current {
            require(
                !verdict.resolved.iter().any(|fixed| fixed == id),
                &format!("finding {id}: cannot be both reproduced and resolved"),
            )?;
            if previous.get(id) == Some(proof) {
                continue;
            }
            self.check_review_validation(conversation, turn, &tree, &proof.validation, "failed")?;
        }
        for (id, reproduction) in previous {
            if current.get(id) == Some(&reproduction) {
                continue;
            }
            let fixed = verdict.resolution_evidence.get(id).ok_or_else(|| crate::invalid(format!(
                "finding {id} ({}): keep the reproduction open or provide resolution_evidence with a new passing run",
                reproduction.scenario
            )))?;
            require(
                fixed.scenario == reproduction.scenario,
                &format!("finding {id}: rerun the same reproduction scenario"),
            )?;
            let passing = self.check_review_validation(
                conversation,
                turn,
                &tree,
                &fixed.validation,
                "passed",
            )?;
            let earlier = self
                .store
                .validation(conversation, &reproduction.validation)?;
            require(
                passing.spec.command == earlier.spec.command
                    && passing.spec.directory == earlier.spec.directory
                    && passing.spec.environment == earlier.spec.environment
                    && self.store.validation_is_newer(&passing.id, &earlier.id)?,
                &format!(
                    "finding {id}: require a newer passing run of the same command, directory and environment"
                ),
            )?;
        }
        Ok(())
    }

    fn check_review_validation(
        &self,
        conversation: &str,
        turn: &str,
        tree: &str,
        validation: &str,
        status: &str,
    ) -> Result<store::ValidationRun> {
        let run = self.store.validation(conversation, validation)?;
        require(
            run.status == status
                && run.tree.as_deref() == Some(tree)
                && run.observation.is_some()
                && self.store.validation_available_to_turn(&run, turn)?,
            &format!(
                "validation {validation}: require a {status} observation on the reviewed tree, run in this turn or explicitly reused"
            ),
        )?;
        let evidence = run
            .evidence
            .as_deref()
            .ok_or_else(|| crate::invalid("validation evidence was not retained"))?;
        self.store
            .evidence_text(&self.store.evidence(conversation, evidence)?)?;
        Ok(run)
    }
}

fn reproduced_findings(findings: &[Value]) -> Result<BTreeMap<&str, ScenarioEvidence>> {
    findings
        .iter()
        .filter(|finding| progress::blocks(finding) && finding["reproduction"].is_object())
        .map(|finding| {
            let id = finding["id"]
                .as_str()
                .ok_or_else(|| crate::invalid("reproduced finding requires an id"))?;
            Ok((id, serde_json::from_value(finding["reproduction"].clone())?))
        })
        .collect()
}
