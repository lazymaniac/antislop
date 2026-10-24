use super::*;

pub(super) struct Recipient {
    pub(super) kind: WorkKind,
    pub(super) role_kind: RoleKind,
    pub(super) sender: String,
    pub(super) title: String,
}

impl Recipient {
    pub(super) fn new(role: &str, task: Option<&Task>) -> Result<Self> {
        let spec = task
            .map(|task| serde_json::from_value::<TaskSpec>(task.spec.clone()))
            .transpose()?;
        Ok(spec.map_or_else(|| Self::for_role(role), |spec| Self::for_task(role, spec)))
    }

    fn for_task(role: &str, spec: TaskSpec) -> Self {
        let verifier = role.starts_with("verifier:") || role.starts_with("code-review:");
        Self {
            kind: spec.kind,
            role_kind: if verifier {
                RoleKind::Verifier
            } else {
                RoleKind::Executor
            },
            sender: task_sender(role, &spec.id, verifier),
            title: spec.title,
        }
    }

    fn for_role(role: &str) -> Self {
        Self {
            kind: WorkKind::Orchestration,
            role_kind: if matches!(role, ORCHESTRATOR | COMMENT_ORCHESTRATOR) {
                RoleKind::Orchestrator
            } else {
                RoleKind::Verifier
            },
            sender: role.to_owned(),
            title: role.to_owned(),
        }
    }
}

fn task_sender(role: &str, task_id: &str, verifier: bool) -> String {
    if role.starts_with("code-review:") {
        format!("{task_id}/review")
    } else if verifier {
        format!("{task_id}/verifier")
    } else {
        task_id.to_owned()
    }
}
