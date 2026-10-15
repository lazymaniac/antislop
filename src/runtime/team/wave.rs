//! Read-only projection of the exact scheduler wave offered for user approval.
use crate::{
    Result,
    config::{Policy, ResolvedProfile, RoleKind},
    domain::{Run, Status},
    require,
    team::{
        CatalogRole, TaskOperation, TaskRecord,
        checks::{CheckPhase, PlannedCheck},
    },
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub task: TaskRecord,
    pub agent_id: String,
    pub role: CatalogRole,
    pub operation: TaskOperation,
    pub profile: ResolvedProfile,
    pub dependency_hashes: BTreeMap<String, String>,
    /// Planned checks for this operation; authenticated existing receipts are reused.
    pub checks: Vec<PlannedCheck>,
    pub write_scopes: BTreeSet<String>,
    pub file_operations: Vec<crate::domain::FileOperation>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    pub run_id: String,
    pub spec_revision: u64,
    pub scope_hash: String,
    pub workspace: String,
    pub policy: Policy,
    pub actions: Vec<Action>,
}

pub fn pending(run: &Run) -> Result<Pending> {
    require(
        !run.intake
            && !run.reconciliation_required
            && run
                .pending_edits
                .iter()
                .all(|edit| edit.state == crate::recovery::EditState::Applied)
            && matches!(
                run.status,
                Status::Ready
                    | Status::AwaitingStart
                    | Status::AwaitingResult
                    | Status::ChangesRequested
                    | Status::Blocked
            ),
        "stop and reconcile before offering a team wave",
    )?;
    crate::workspace::managed::require_active(run)?;
    let team = run
        .team
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("no installed team".into()))?;
    let config = &run
        .spec
        .config
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("team configuration missing".into()))?
        .config;
    let invocations = team.ready_wave(&run.spec, &super::gate_context(run))?;
    require(!invocations.is_empty(), "no ready scheduler actions")?;
    let mut actions = Vec::new();
    for invocation in invocations {
        let role = if invocation.operation == TaskOperation::Verify {
            RoleKind::Verifier
        } else {
            invocation.agent.kind
        };
        let phase = if invocation.operation == TaskOperation::Verify {
            CheckPhase::BeforeVerification
        } else {
            CheckPhase::BeforeExecution
        };
        let profile = config.resolve(invocation.agent.role.work_kind(), role)?;
        let checks = invocation
            .task
            .check_plan
            .iter()
            .filter(|c| c.phase == phase)
            .cloned()
            .collect();
        let write_scopes = if invocation.operation == TaskOperation::Execute {
            invocation.task.write_scopes.clone()
        } else {
            BTreeSet::new()
        };
        actions.push(Action {
            file_operations: if write_scopes.is_empty() {
                vec![]
            } else {
                use crate::domain::FileOperation::*;
                vec![Write, Create, Delete]
            },
            task: invocation.task,
            agent_id: invocation.agent.id,
            role: invocation.agent.role,
            operation: invocation.operation,
            profile,
            dependency_hashes: invocation.dependency_hashes,
            checks,
            write_scopes,
        });
    }
    Ok(Pending {
        run_id: run.id.clone(),
        spec_revision: run.spec_revision,
        scope_hash: run.team_step_scope()?,
        workspace: run.spec.workspace.clone(),
        policy: config.policy.clone(),
        actions,
    })
}
