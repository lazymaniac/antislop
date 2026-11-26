use crate::lean::prompts;

pub(crate) const FILESYSTEM_GUIDANCE: &str = prompts::FILESYSTEM_GUIDANCE;
pub(crate) const ACTIVATION_GUIDANCE: &str = prompts::ACTIVATION_GUIDANCE;
pub(crate) const PLAN_DECOMPOSITION: &str = prompts::PLAN_DECOMPOSITION;

pub fn planner_instructions() -> String {
    [
        prompts::PLANNER_ROLE,
        prompts::TARGET_IDENTITY,
        prompts::PLAN_FORMAT,
        PLAN_DECOMPOSITION,
    ]
    .join("\n\n")
}

pub fn orchestrator_instructions() -> String {
    prompts::ORCHESTRATOR_ROLE.into()
}
