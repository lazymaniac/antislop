//! Task-owned cleanup after execution and revisions; busy views are retried.
use super::core_path_helpers::{release_view, task_path_component, task_view_paths};
use super::*;

/// A checkout lease excludes cleanup from every in-flight checkout user.
pub(in crate::lean) struct WorktreeLease<'a> {
    core: &'a Core,
    path: PathBuf,
}
impl Drop for WorktreeLease<'_> {
    fn drop(&mut self) {
        if let Ok(mut leases) = self.core.worktree_leases.lock() {
            if let Some(state) = leases.get_mut(&self.path) {
                state.leased = false;
                if state.pins == 0 {
                    leases.remove(&self.path);
                }
            }
            self.core.worktree_lease_changed.notify_all();
        }
    }
}

#[derive(Default)]
pub(super) struct WorktreeLeaseState {
    leased: bool,
    pins: usize,
}

/// A cold native-session launch pins its checkout without holding scheduler locks.
pub(in crate::lean) struct WorktreePin<'a> {
    core: &'a Core,
    path: PathBuf,
}

impl Drop for WorktreePin<'_> {
    fn drop(&mut self) {
        if let Ok(mut leases) = self.core.worktree_leases.lock()
            && let Some(state) = leases.get_mut(&self.path)
        {
            state.pins -= 1;
            if !state.leased && state.pins == 0 {
                leases.remove(&self.path);
            }
            self.core.worktree_lease_changed.notify_all();
        }
    }
}

impl Core {
    #[cfg(test)]
    pub(in crate::lean) fn lease_worktree(&self, path: &Path) -> WorktreeLease<'_> {
        let mut leases = self.worktree_leases.lock().expect("worktree leases mutex");
        while leases.contains_key(path) {
            leases = self
                .worktree_lease_changed
                .wait(leases)
                .expect("worktree leases mutex");
        }
        leases.entry(path.to_path_buf()).or_default().leased = true;
        WorktreeLease {
            core: self,
            path: path.to_path_buf(),
        }
    }

    pub(in crate::lean) fn wait_for_worktree_available(
        &self,
        paths: &std::collections::HashSet<PathBuf>,
        cancel: &AtomicBool,
    ) -> bool {
        let mut leases = self.worktree_leases.lock().expect("worktree leases mutex");
        while paths.iter().all(|path| leases.contains_key(path)) && !cancel.load(Ordering::SeqCst) {
            leases = self
                .worktree_lease_changed
                .wait_timeout(leases, std::time::Duration::from_millis(100))
                .expect("worktree leases mutex")
                .0;
        }
        !cancel.load(Ordering::SeqCst)
    }

    pub(in crate::lean) fn try_lease_worktree(&self, path: &Path) -> Option<WorktreeLease<'_>> {
        let mut leases = self.worktree_leases.lock().expect("worktree leases mutex");
        if leases.contains_key(path) {
            return None;
        }
        leases.entry(path.to_path_buf()).or_default().leased = true;
        Some(WorktreeLease {
            core: self,
            path: path.to_path_buf(),
        })
    }

    pub(in crate::lean) fn try_pin_worktree(&self, path: &Path) -> Option<WorktreePin<'_>> {
        let mut leases = self.worktree_leases.lock().expect("worktree leases mutex");
        let state = leases.entry(path.to_path_buf()).or_default();
        if state.leased {
            return None;
        }
        state.pins += 1;
        Some(WorktreePin {
            core: self,
            path: path.to_path_buf(),
        })
    }

    pub(in crate::lean) fn try_task_worktree_lease(
        &self,
        task: &Task,
    ) -> Result<(bool, Option<WorktreeLease<'_>>)> {
        let path = self.task_worktree_path(task)?;
        match path.as_deref() {
            None => Ok((false, None)),
            Some(path) => Ok((true, self.try_lease_worktree(path))),
        }
    }

    #[cfg(test)]
    pub(in crate::lean) fn task_worktree_lease(
        &self,
        task: &Task,
    ) -> Result<Option<WorktreeLease<'_>>> {
        Ok(self
            .task_worktree_path(task)?
            .as_deref()
            .map(|path| self.lease_worktree(path)))
    }

    pub(in crate::lean) fn task_worktree_path(&self, task: &Task) -> Result<Option<PathBuf>> {
        let contract: tasks::TaskContract = serde_json::from_value(task.spec.clone())?;
        let path = if let Some(owner_id) = contract.options_for.as_deref() {
            self.store
                .task_in_plan_by_id(&task.plan_id, owner_id)?
                .map(|owner| super::core_path_helpers::worktree_path(&self.state, &owner))
                .transpose()?
        } else if contract.change_kind.has_delivery() {
            Some(super::core_path_helpers::worktree_path(&self.state, task)?)
        } else {
            None
        };
        Ok(path)
    }

    pub(in crate::lean) fn queue_worktree_cleanup(&self, task: &str) {
        self.worktree_cleanup
            .lock()
            .expect("worktree cleanup mutex")
            .insert(task.into());
    }

    pub(in crate::lean) fn drain_worktree_cleanup(&self) {
        let pending = std::mem::take(
            &mut *self
                .worktree_cleanup
                .lock()
                .expect("worktree cleanup mutex"),
        );
        for id in pending {
            match self.cleanup_task_worktrees(&id) {
                Ok(false) => self.queue_worktree_cleanup(&id),
                Ok(true) => {}
                Err(error) => {
                    self.cleanup_notice(&id, &error);
                    self.queue_worktree_cleanup(&id);
                }
            }
        }
    }

    /// Retire obsolete legacy read-only views before a task uses its stable path.
    pub(in crate::lean) fn release_task_views(&self, id: &str, keep: Option<&Path>) {
        self.release_task_views_with_hook(id, keep, || {});
    }

    fn release_task_views_with_hook(
        &self,
        id: &str,
        keep: Option<&Path>,
        mut before_external_cleanup: impl FnMut(),
    ) {
        let result = task_view_paths(&self.state, id, keep).map(|paths| {
            let mut first_error = None;
            for path in paths {
                let Some(_lease) = self.try_lease_worktree(&path) else {
                    self.queue_worktree_cleanup(id);
                    continue;
                };
                before_external_cleanup();
                match self.cleanup_view_path(&path) {
                    Ok(false) => self.queue_worktree_cleanup(id),
                    Ok(true) => {}
                    Err(error) => {
                        first_error.get_or_insert(error);
                    }
                }
            }
            first_error
        });
        if let Ok(Some(error)) = result {
            self.cleanup_notice(id, &error);
        } else if let Err(error) = result {
            self.cleanup_notice(id, &error);
        }
    }

    fn cleanup_view_path(&self, path: &Path) -> Result<bool> {
        require(
            git::is_clean(path),
            &format!(
                "worktree {} has uncommitted files; preserved",
                path.display()
            ),
        )?;
        if !self.native.retire_directory(path)? {
            return Ok(false);
        }
        release_view(path)?;
        Ok(true)
    }

    fn cleanup_task_worktrees(&self, id: &str) -> Result<bool> {
        self.cleanup_task_worktrees_with_hook(id, || {})
    }

    fn cleanup_task_worktrees_with_hook(
        &self,
        id: &str,
        mut before_external_cleanup: impl FnMut(),
    ) -> Result<bool> {
        task_path_component(id)?;
        let directory = super::core_path_helpers::worktree_directory(&self.state)?;
        // Filesystem enumeration and generation pruning can block independently
        // of graph edits, so collect names before briefly reserving candidates.
        let entries = if directory.exists() {
            std::fs::read_dir(&directory)?
                .map(|entry| {
                    entry.map(|entry| {
                        (
                            entry.file_name().to_string_lossy().into_owned(),
                            entry.path(),
                        )
                    })
                })
                .collect::<std::io::Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        let (task, candidates, reservation_blocked) = {
            let _graph = self.graph_updates.lock().expect("graph update mutex");
            if self
                .task_cancels
                .lock()
                .expect("task cancel mutex")
                .contains_key(id)
            {
                return Ok(false);
            }
            let Some(task) = self.store.task_by_id(id)? else {
                return Ok(true);
            };
            let mut candidates = Vec::new();
            let mut reservation_blocked = false;
            for (name, path) in entries {
                let Some(kind) = checkout_kind(&name, id) else {
                    continue;
                };
                if !eligible(&task, kind) {
                    continue;
                }
                let Some(lease) = self.try_lease_worktree(&path) else {
                    reservation_blocked = true;
                    continue;
                };
                candidates.push((name, path, kind, lease));
            }
            (task, candidates, reservation_blocked)
        };
        super::core_task_runtime::prune_settled_generations(&self.state, &task)?;
        let mut complete = !reservation_blocked;
        let mut first_error = None;
        for (name, path, kind, _lease) in candidates {
            before_external_cleanup();
            // A replacement path or symlink is never followed or removed.
            let result = (|| {
                require(
                    !std::fs::symlink_metadata(&path)?.file_type().is_symlink(),
                    "task worktree path is a symlink; preserved",
                )?;
                // Recheck eligibility under the graph lock while the path lease
                // prevents a new writer or pin from reaching this checkout.
                let still_eligible = {
                    let _graph = self.graph_updates.lock().expect("graph update mutex");
                    self.store
                        .task_by_id(id)?
                        .is_some_and(|current| eligible(&current, kind))
                        && !self
                            .task_cancels
                            .lock()
                            .expect("task cancel mutex")
                            .contains_key(id)
                };
                if !still_eligible {
                    return Ok(false);
                }
                require(
                    git::is_clean(&path),
                    &format!(
                        "worktree {} has uncommitted files; preserved",
                        path.display()
                    ),
                )?;
                if !self.native.retire_directory(&path)? {
                    return Ok(false);
                }
                if matches!(kind, CheckoutKind::ReadOnlyView { .. }) {
                    release_view(&path)?;
                } else {
                    let project = git::worktree_project(&path)?;
                    git::remove_clean_writer(
                        &project,
                        &path,
                        &format!("antislop/{name}"),
                        &self.store.artifact_output_history(&task)?,
                    )?;
                }
                Ok(true)
            })();
            match result {
                Ok(done) => complete &= done,
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        if let Some(error) = first_error {
            self.cleanup_notice(id, &error);
        } else {
            self.worktree_cleanup_notes
                .lock()
                .expect("worktree cleanup notes mutex")
                .remove(id);
        }
        Ok(complete)
    }

    fn cleanup_notice(&self, id: &str, error: &Error) {
        let text = error.to_string();
        let mut notes = self
            .worktree_cleanup_notes
            .lock()
            .expect("worktree cleanup notes mutex");
        if notes.get(id) == Some(&text) {
            return;
        }
        notes.insert(id.into(), text);
        drop(notes);
        if let Ok(Some(task)) = self.store.task_by_id(id)
            && let Ok(conversation) = self.store.plan_conversation(&task.plan_id)
        {
            self.notice(
                &conversation,
                &format!("worktree cleanup for task {id}: {error}"),
            );
        }
    }
}

#[derive(Clone, Copy)]
enum CheckoutKind {
    StableWriter,
    LegacyWriter { generation: i64 },
    ReadOnlyView { generation: i64, round: Option<u32> },
}

/// Only exact runtime names qualify; generation suffixes are legacy checkouts.
fn checkout_kind(name: &str, id: &str) -> Option<CheckoutKind> {
    let suffix = name.strip_prefix(id)?;
    if suffix.is_empty() {
        return Some(CheckoutKind::StableWriter);
    }
    let suffix = suffix.strip_prefix('-')?;
    if let Some(generation) = suffix.strip_prefix("observation-g") {
        return Some(CheckoutKind::ReadOnlyView {
            generation: generation.parse().ok()?,
            round: None,
        });
    }
    if let Some(review) = suffix.strip_prefix("review-g") {
        let (generation, round) = review.split_once('-')?;
        return Some(CheckoutKind::ReadOnlyView {
            generation: generation.parse().ok()?,
            round: Some(round.parse().ok()?),
        });
    }
    Some(CheckoutKind::LegacyWriter {
        generation: suffix.strip_prefix('g')?.parse().ok()?,
    })
}

fn eligible(task: &Task, checkout: CheckoutKind) -> bool {
    let settled = matches!(
        task.status.as_str(),
        status::APPROVED | status::STALE | "superseded"
    );
    match checkout {
        CheckoutKind::StableWriter => settled,
        CheckoutKind::LegacyWriter { generation } => generation < task.input_generation || settled,
        CheckoutKind::ReadOnlyView { generation, round } => {
            generation < task.input_generation
                || generation == task.input_generation
                    && round.is_some_and(|round| round < task.round)
                || settled
                || matches!(task.status.as_str(), status::CANCELLED | status::SKIPPED)
        }
    }
}

#[cfg(test)]
mod tests;
