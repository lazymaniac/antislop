use super::core_feedback_helpers::{event_files, land, note};
use super::core_path_helpers::{worktree_branch, worktree_path};
use super::*;

mod actions;
mod integrate;
mod manual;
mod prepare;
mod response;

pub(super) struct RecoveryWork {
    pub(super) req: TaskRecoveryRequest,
    pub(super) actor: String,
    pub(super) project: PathBuf,
    pub(super) config: Arc<Config>,
    pub(super) spec: TaskContract,
    pub(super) task: Task,
    pub(super) token: String,
    pub(super) integrated_files: Vec<String>,
    pub(super) preserved_branch: Option<String>,
    pub(super) notice: Option<String>,
}

impl Core {
    pub(in crate::lean) fn recover_task_as(
        self: &Arc<Self>,
        req: TaskRecoveryRequest,
        actor: &str,
    ) -> Result<TaskRecoveryResponse> {
        let _graph = self.graph_updates.lock().expect("graph update mutex");
        let _integration = self.integrating.lock().expect("integration mutex");
        let mut work = prepare::load(self, req, actor)?;
        if let Some(response) = prepare::interrupt_if_active(self, &mut work) {
            return Ok(response);
        }
        actions::apply(self, &mut work)?;
        response::build(self, work)
    }
}
