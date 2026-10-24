//! Atomic additive edits to a running DAG. Executed tasks are immutable.
use super::{Store, Task};
use crate::Result;
use crate::lean::tasks::TaskSpec;

#[path = "graph_edit/helpers.rs"]
mod helpers;

impl Store {
    pub fn extend_running_graph(
        &self,
        conversation: &str,
        baseline: &[Task],
        specs: &[TaskSpec],
    ) -> Result<()> {
        let context = helpers::prepare_graph_edit(self, conversation, baseline, specs)?;
        helpers::persist_graph_edit(self, &context, baseline, specs)
    }
}
