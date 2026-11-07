//! Task-owned cleanup after execution and revisions; busy views are retried.
use super::core_path_helpers::{release_view, task_path_component, task_view_paths};
use super::*;

/// A cold native-session launch pins its checkout without holding scheduler locks.
pub(in crate::lean) struct WorktreePin<'a> {
    core: &'a Core,
    path: PathBuf,
}
impl Drop for WorktreePin<'_> {
    fn drop(&mut self) {
        if let Ok(mut pins) = self.core.worktree_pins.lock()
            && let Some(count) = pins.get_mut(&self.path)
        {
            *count -= 1;
            if *count == 0 {
                pins.remove(&self.path);
            }
        }
    }
}

impl Core {
    pub(in crate::lean) fn pin_worktree(&self, path: &Path) -> WorktreePin<'_> {
        *self
            .worktree_pins
            .lock()
            .expect("worktree pins mutex")
            .entry(path.to_path_buf())
            .or_default() += 1;
        WorktreePin {
            core: self,
            path: path.to_path_buf(),
        }
    }

    fn worktree_is_pinned(&self, path: &Path) -> bool {
        self.worktree_pins
            .lock()
            .expect("worktree pins mutex")
            .contains_key(path)
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
            let Ok(_graph) = self.graph_updates.try_lock() else {
                self.queue_worktree_cleanup(&id);
                continue;
            };
            if self
                .task_cancels
                .lock()
                .expect("task cancel mutex")
                .contains_key(&id)
            {
                self.queue_worktree_cleanup(&id);
                continue;
            }
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
        let _graph = self.graph_updates.lock().expect("graph update mutex");
        let result = task_view_paths(&self.state, id, keep).and_then(|paths| {
            let mut first_error = None;
            for path in paths {
                match self.cleanup_view_path(&path) {
                    Ok(false) => self.queue_worktree_cleanup(id),
                    Ok(true) => {}
                    Err(error) => {
                        first_error.get_or_insert(error);
                    }
                }
            }
            first_error.map_or(Ok(()), Err)
        });
        if let Err(error) = result {
            self.cleanup_notice(id, &error);
        }
    }

    fn cleanup_view_path(&self, path: &Path) -> Result<bool> {
        if self.worktree_is_pinned(path) {
            return Ok(false);
        }
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
        task_path_component(id)?;
        let Some(task) = self.store.task_by_id(id)? else {
            return Ok(true);
        };
        super::core_task_runtime::prune_settled_generations(&self.state, &task)?;
        let directory = super::core_path_helpers::worktree_directory(&self.state)?;
        if !directory.exists() {
            return Ok(true);
        }
        let mut complete = true;
        let mut first_error = None;
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(kind) = checkout_kind(&name, id) else {
                continue;
            };
            if !eligible(&task, kind) {
                continue;
            }
            let path = entry.path();
            if self.worktree_is_pinned(&path) {
                complete = false;
                continue;
            }
            // A replacement path or symlink is never followed or removed.
            let result = (|| {
                require(
                    !entry.file_type()?.is_symlink(),
                    "task worktree path is a symlink; preserved",
                )?;
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
