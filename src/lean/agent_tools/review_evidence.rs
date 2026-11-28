//! Reproduced findings stay open until there is a matching passing observation.
use super::*;
use crate::lean::tasks::ScenarioEvidence;
use std::collections::BTreeMap;

impl Core {
    pub(super) fn carry_review_findings(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        verdict: &mut tasks::Verdict,
        requested: &Value,
    ) -> Result<()> {
        if requested.is_null() {
            return Ok(());
        }
        let requested: Vec<CarryFinding> = serde_json::from_value(requested.clone())?;
        if requested.is_empty() {
            return Ok(());
        }
        let (task_id, _) = task_role(role)
            .filter(|(_, reviewer)| *reviewer)
            .ok_or_else(|| crate::invalid("carry_findings requires a task reviewer"))?;
        let task = self
            .store
            .task_by_id(task_id)?
            .ok_or_else(|| crate::invalid("task no longer exists"))?;
        let (_, previous) = self
            .store
            .previous_task_review(conversation, &task, turn)?
            .ok_or_else(|| {
                crate::invalid("carry_findings: no earlier review of these task inputs")
            })?;
        let disposed = self.store.disposed_findings(&task)?;
        for carry in requested {
            require(
                !disposed.contains(&carry.id),
                "carry_findings: this finding already has an approved disposition",
            )?;
            let mut entry = previous
                .findings
                .iter()
                .find(|finding| finding["id"] == carry.id)
                .cloned()
                .ok_or_else(|| {
                    crate::invalid(format!("carry_findings: unknown open id {}", carry.id))
                })?;
            require(
                !verdict.resolved.contains(&carry.id)
                    && !verdict
                        .findings
                        .iter()
                        .any(|finding| finding["id"] == carry.id),
                &format!(
                    "finding {} cannot be carried more than once or both carried and resolved",
                    carry.id
                ),
            )?;
            if let Some(location) = carry.location {
                require(
                    !location.trim().is_empty(),
                    "carry_findings.where must be nonempty",
                )?;
                entry["where"] = json!(location);
            }
            verdict.approved &= !progress::blocks(&entry);
            verdict.findings.push(entry);
        }
        Ok(())
    }

    pub(super) fn check_review_evidence(
        &self,
        conversation: &str,
        turn: &str,
        task: &Task,
        verdict: &tasks::Verdict,
    ) -> Result<()> {
        // Preserve open findings across coordination and cancelled rounds.
        let previous = self.store.previous_task_review(conversation, task, turn)?;
        let mut previous = reproduced_findings(
            previous
                .as_ref()
                .map_or(&[], |(_, verdict)| verdict.findings.as_slice()),
        )?;
        let disposed = self.store.disposed_findings(task)?;
        previous.retain(|id, _| !disposed.contains(*id));
        let current = reproduced_findings(&verdict.findings)?;
        require(
            current.keys().all(|id| !disposed.contains(*id)),
            "a reproduced ID already has an approved disposition; a new finding needs its own location",
        )?;
        let transferred = if verdict.approved {
            self.store.transferred_criteria(task)?
        } else {
            Default::default()
        };
        require(
            verdict
                .resolution_evidence
                .keys()
                .all(|id| previous.contains_key(id.as_str()) && verdict.resolved.contains(id)),
            "resolution_evidence: use only ids of earlier reproduced findings listed in resolved",
        )?;
        if current.is_empty() && previous.is_empty() && transferred.is_empty() {
            return Ok(());
        }
        let context = self
            .store
            .run_context(conversation, turn)?
            .ok_or_else(|| crate::invalid("review evidence requires a recorded checkout"))?;
        let root = PathBuf::from(git::ok(&context.cwd, &["rev-parse", "--show-toplevel"])?);
        let tree = super::validations::validation_tree(&root, true)?
            .ok_or_else(|| crate::invalid("review evidence requires a verifiable Git tree"))?;
        for criterion in transferred {
            let validation = verdict.coverage.get(&criterion).and_then(|check| check["validation"].as_str())
                .ok_or_else(|| crate::invalid(format!("checks.{criterion}.validation: attach the passing receipt for the transferred scenario")))?;
            self.check_review_validation(conversation, turn, &tree, validation, "passed")?;
        }
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
                "finding {id} ({}): keep the reproduction open with carry_findings using its exact ID, or provide resolution_evidence with a new passing run",
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
                    && passing.spec.expected_exit_code == earlier.spec.expected_exit_code
                    && (earlier.spec.kind.is_none() || passing.spec.kind == earlier.spec.kind)
                    && self.store.validation_is_newer(&passing.id, &earlier.id)?,
                &format!(
                    "finding {id}: require a newer passing run with the same command, directory, environment and expected exit"
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

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CarryFinding {
    id: String,
    #[serde(rename = "where")]
    location: Option<String>,
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
