use super::*;
use crate::lean::prompts;

const OPTIONS_NOTE: &str = prompts::OPTIONS_NOTE;

pub(crate) const FILESYSTEM_GUIDANCE: &str = prompts::FILESYSTEM_GUIDANCE;

pub(crate) const ACTIVATION_GUIDANCE: &str = prompts::ACTIVATION_GUIDANCE;

const TARGET_IDENTITY: &str = prompts::TARGET_IDENTITY;

const ORCHESTRATOR_WORKFLOW: &str = prompts::ORCHESTRATOR_WORKFLOW;

/// Shared task-boundary policy for planners, orchestrators and plan reviewers.
pub(crate) const PLAN_DECOMPOSITION: &str = prompts::PLAN_DECOMPOSITION;

const PLAN_FORMAT: &str = prompts::PLAN_FORMAT;

const PLANNER_ROLE: &str = prompts::PLANNER_ROLE;

const TEAM_NOTE: &str = prompts::TEAM_NOTE;

const PLAN_REVIEW_NOTE: &str = prompts::PLAN_REVIEW_NOTE;

const ORCHESTRATOR_TAIL: &str = prompts::ORCHESTRATOR_TAIL;

/// What the orchestrator knows: the session workflow, the plan format it
/// revises, and how it answers agents.
pub fn orchestrator_instructions() -> String {
    [
        OPTIONS_NOTE,
        TARGET_IDENTITY,
        ORCHESTRATOR_WORKFLOW,
        prompts::ORCHESTRATOR_INSTRUCTIONS,
        PLAN_DECOMPOSITION,
        prompts::PRESERVE_SELECTED_TEAM,
        crate::lean::tasks::FEATURE_PLANNING_RULES,
        ORCHESTRATOR_TAIL,
    ]
    .join("\n\n")
    .replace(crate::lean::prompts::KIND_NAMES_PLACEHOLDER, &kind_names())
    .replace(
        crate::lean::prompts::AGENT_POOL_PLACEHOLDER,
        &agent_descriptions(),
    )
    .replace(
        crate::lean::prompts::TEAMS_PLACEHOLDER,
        &crate::lean::tasks::team_catalog(),
    )
}

/// What the planner knows: its role and the plan format, not the workflow.
pub fn planner_instructions() -> String {
    [
        PLANNER_ROLE,
        OPTIONS_NOTE,
        TARGET_IDENTITY,
        PLAN_FORMAT,
        PLAN_DECOMPOSITION,
        TEAM_NOTE,
        PLAN_REVIEW_NOTE,
    ]
    .join("\n\n")
    .replace(crate::lean::prompts::KIND_NAMES_PLACEHOLDER, &kind_names())
    .replace(
        crate::lean::prompts::AGENT_POOL_PLACEHOLDER,
        &agent_descriptions(),
    )
    .replace(
        crate::lean::prompts::TEAMS_PLACEHOLDER,
        &crate::lean::tasks::team_catalog(),
    )
}

fn kind_names() -> String {
    [
        "context",
        "requirements",
        "options",
        "planning",
        "architecture",
        "dependencies",
        "implementation",
        "refactoring",
        "static_analysis",
        "test_design",
        "testing",
        "security",
        "performance",
        "integration",
        "research",
        "writing",
        "analysis",
        "delivery",
    ]
    .join(", ")
}

fn agent_descriptions() -> String {
    PREDEFINED_AGENT_POOL
        .iter()
        .chain(FEATURE_SPECIALISTS.iter())
        .map(|agent| {
            format!(
                crate::lean::prompts::template!(AGENT_DESCRIPTION),
                agent.id,
                work_kind_name(agent.kind),
                agent.focus
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}
