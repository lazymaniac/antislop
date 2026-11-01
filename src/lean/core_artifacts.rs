//! Provenance checks for updating an artifact owned by this conversation.
use super::*;
use std::collections::BTreeMap;
use store::ArtifactReceipt;

type Inputs = BTreeMap<String, Option<ArtifactReceipt>>;

impl Core {
    pub(in crate::lean) fn artifact_inputs(
        &self,
        task: &Task,
        spec: &TaskContract,
    ) -> Result<Inputs> {
        let conversation = self.store.plan_conversation(&task.plan_id)?;
        let mut sources = self.dependency_tasks(&task.plan_id, spec)?;
        sources.push(task.clone());
        let originals = sources
            .iter()
            .filter_map(|source| {
                self.store
                    .completed_task_result(source)
                    .ok()
                    .map(|(id, _)| id)
            })
            .collect::<Vec<_>>();
        for id in originals {
            if !sources.iter().any(|source| source.id == id)
                && let Some(source) = self.store.task_by_id(&id)?
            {
                require(
                    self.store.plan_conversation(&source.plan_id)? == conversation,
                    "artifact source belongs to another conversation",
                )?;
                sources.push(source);
            }
        }
        let mut inputs = BTreeMap::new();
        for path in &spec.outputs {
            let input = match self.store.artifact_input(task, path)? {
                Some(input) => input,
                None => {
                    let receipt = self.artifact_source(&conversation, path, &sources)?;
                    self.store
                        .freeze_artifact_input(task, path, receipt.as_ref())?
                }
            };
            inputs.insert(path.clone(), input);
        }
        Ok(inputs)
    }

    fn artifact_source(
        &self,
        conversation: &str,
        path: &str,
        sources: &[Task],
    ) -> Result<Option<ArtifactReceipt>> {
        if let Some(receipt) = self.store.artifact_receipt(conversation, path)? {
            return Ok(sources
                .iter()
                .any(|task| task.id == receipt.task)
                .then_some(receipt));
        }
        // Older daemons already persisted successful landings. Use only a
        // declared artifact dependency with that durable proof, never a file
        // name, an unlanded result, or a report's assertion of ownership.
        for task in sources.iter().rev() {
            let spec: TaskContract = serde_json::from_value(task.spec.clone())?;
            if spec.delivery == Some(tasks::DeliveryMode::Untracked)
                && spec.outputs.iter().any(|p| p == path)
                && let Some(commit) = self.artifact_landing(task)
                && git::artifact_was_created(
                    Path::new(&self.store.conversation(conversation)?.project),
                    &commit,
                    path,
                )?
            {
                let receipt = ArtifactReceipt {
                    task: task.id.clone(),
                    generation: task.input_generation,
                    commit: commit.clone(),
                };
                self.store
                    .record_artifact_delivery(conversation, path, &receipt)?;
                return Ok(Some(receipt));
            }
        }
        Ok(None)
    }

    pub(in crate::lean) fn artifact_commits(
        &self,
        task: &Task,
        spec: &TaskContract,
    ) -> Result<BTreeMap<String, String>> {
        Ok(self
            .artifact_inputs(task, spec)?
            .into_iter()
            .filter_map(|(path, receipt)| receipt.map(|r| (path, r.commit)))
            .collect())
    }

    pub(in crate::lean) fn prepare_artifact_outputs(
        &self,
        task: &Task,
        spec: &TaskContract,
        project: &Path,
    ) -> Result<BTreeMap<String, String>> {
        let inputs = self.artifact_commits(task, spec)?;
        let mut current = inputs.clone();
        let conversation = self.store.plan_conversation(&task.plan_id)?;
        for path in &spec.outputs {
            if let Some(receipt) =
                self.artifact_source(&conversation, path, std::slice::from_ref(task))?
                && receipt.task == task.id
                && receipt.generation == task.input_generation
            {
                current.insert(path.clone(), receipt.commit);
            }
        }
        git::require_owned_outputs(project, &spec.outputs, &current)?;
        Ok(inputs)
    }

    pub(in crate::lean) fn validate_artifact_delta(
        &self,
        task: &Task,
        spec: &TaskContract,
        worktree: &Path,
    ) -> Result<()> {
        if spec.delivery == Some(tasks::DeliveryMode::Untracked)
            && let (Some(base), Some(result)) = (&task.base, &task.result)
        {
            git::require_added_files(worktree, base, result, &self.artifact_commits(task, spec)?)?;
        }
        Ok(())
    }

    pub(in crate::lean) fn deliver_artifact_change(
        &self,
        task: &Task,
        spec: &TaskContract,
        project: &Path,
        base: &str,
        change: &str,
    ) -> Result<git::Integration> {
        let conversation = self.store.plan_conversation(&task.plan_id)?;
        let inputs = self.artifact_inputs(task, spec)?;
        let original = self.artifact_commits(task, spec)?;
        let mut expected = BTreeMap::new();
        for (path, input) in &inputs {
            let current = self.store.artifact_receipt(&conversation, path)?;
            let replay = current
                .as_ref()
                .is_some_and(|r| r.task == task.id && r.generation == task.input_generation);
            if !replay && current != *input {
                return Ok(git::Integration::UserEdits(vec![path.clone()]));
            }
            if let Some(receipt) = if replay {
                current.as_ref()
            } else {
                input.as_ref()
            } {
                expected.insert(path.clone(), receipt.commit.clone());
            }
        }
        spec.validate_change_paths(&git::changed_files(project, base, change)?)?;
        let outcome = git::deliver_owned_artifacts(
            project,
            base,
            change,
            &original,
            &expected,
            &self.state.join("artifact-backups"),
        )?;
        if outcome == git::Integration::Applied {
            for path in git::changed_files(project, base, change)? {
                self.store.record_artifact_delivery(
                    &conversation,
                    &path,
                    &ArtifactReceipt {
                        task: task.id.clone(),
                        generation: task.input_generation,
                        commit: change.into(),
                    },
                )?;
            }
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests;
