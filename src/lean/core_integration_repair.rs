use super::*;

mod repair;

impl Core {
    /// Resolve in the allocated worktree, then run the independent verifier
    /// again. A semantic choice is the resolver's explicit user question.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::lean) fn repair_integration(
        self: &Arc<Self>,
        conversation: &str,
        task: &mut Task,
        spec: &TaskSpec,
        worktree: &Path,
        onto: &str,
        change: &str,
        outcome: Outcome,
        files: &[String],
        cancel: &AtomicBool,
    ) -> Result<()> {
        repair::Repair::new(repair::RepairParts {
            core: self,
            conversation,
            task,
            spec,
            worktree,
            onto,
            change,
            outcome,
            files,
            cancel,
        })
        .run()
    }

    /// Protect dirty paths unless their starting version is exactly the
    /// last approved writer's output for that path. Only paths that writer
    /// changed count: an approved snapshot also contains unrelated user edits.
    pub(in crate::lean) fn in_place_user_edits(
        &self,
        project: &Path,
        base: &str,
    ) -> Result<Vec<String>> {
        let committed_base = git::common_base(project, base, "HEAD")?;
        let protected = git::changed_files(project, &committed_base, base)?;
        if protected.is_empty() {
            return Ok(protected);
        }
        let approved = self
            .store
            .approved_writer_results(&project.to_string_lossy())?;
        retain_user_edits(project, base, protected, approved)
    }
}

fn retain_user_edits(
    project: &Path,
    base: &str,
    mut protected: Vec<String>,
    approved: Vec<(String, String)>,
) -> Result<Vec<String>> {
    let mut unresolved = protected.clone();
    for (before, result) in approved {
        let candidates = changed_candidates(project, &before, &result, &unresolved)?;
        if candidates.is_empty() {
            continue;
        }
        let owned = git::matching_paths(project, &result, base, &candidates)?;
        protected.retain(|path| !owned.contains(path));
        unresolved.retain(|path| !candidates.contains(path));
        if unresolved.is_empty() {
            break;
        }
    }
    Ok(protected)
}

fn changed_candidates(
    project: &Path,
    before: &str,
    result: &str,
    unresolved: &[String],
) -> Result<Vec<String>> {
    let changed = git::changed_files(project, before, result)?;
    Ok(unresolved
        .iter()
        .filter(|path| changed.contains(path))
        .cloned()
        .collect())
}
