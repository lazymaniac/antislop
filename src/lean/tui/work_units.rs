//! Frontend ownership projection shared by Canvas, inspector and Workbench.
//! Stage addresses remain the runtime's original role addresses; retries never
//! change a work unit's identity. Similar names and event timing confer no ownership.
use super::graph::{self, Health};
use super::view::{AUDITOR, ORCHESTRATOR, PLAN_REVIEWER, TaskRow, View};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WorkUnitId {
    Orchestration,
    Planning,
    Task(String),
    FinalAudit,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StageId(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkSelection {
    pub unit: WorkUnitId,
    pub stage: Option<StageId>,
}

#[derive(Debug, Clone)]
pub struct WorkStage {
    pub id: StageId,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct WorkUnit {
    pub id: WorkUnitId,
    pub card: String,
    pub title: String,
    pub task: Option<String>,
    pub stages: Vec<WorkStage>,
}

#[derive(Debug, Default)]
pub struct WorkUnits {
    pub units: Vec<WorkUnit>,
    by_stage: HashMap<String, usize>,
    by_id: HashMap<WorkUnitId, usize>,
    /// Options task ID -> its unique owning task ID. Shared options stay separate.
    pub options_owner: HashMap<String, String>,
}

impl WorkUnits {
    pub fn new(view: &View) -> Self {
        let options_owner = options_owners(view);
        let mut result = Self {
            options_owner,
            ..Self::default()
        };
        result.push(WorkUnit {
            id: WorkUnitId::Orchestration,
            card: ORCHESTRATOR.into(),
            title: "Orchestration".into(),
            task: None,
            stages: std::iter::once(stage(ORCHESTRATOR, "Orchestrator"))
                .chain(
                    view.agents
                        .contains_key(crate::lean::COMMENT_ORCHESTRATOR)
                        .then(|| stage(crate::lean::COMMENT_ORCHESTRATOR, "Comment orchestration")),
                )
                .collect(),
        });
        if [crate::lean::PLANNER, PLAN_REVIEWER]
            .iter()
            .any(|key| view.agents.contains_key(*key))
        {
            let stages = [
                (crate::lean::PLANNER, "Planner"),
                (PLAN_REVIEWER, "Plan reviewer"),
            ]
            .into_iter()
            .filter(|(key, _)| view.agents.contains_key(*key))
            .map(|(key, label)| stage(key, label))
            .collect();
            result.push(WorkUnit {
                id: WorkUnitId::Planning,
                card: crate::lean::PLANNER.into(),
                title: "Planning".into(),
                task: None,
                stages,
            });
        }
        let mut owned: HashMap<String, Vec<&TaskRow>> = HashMap::new();
        for task in &view.tasks {
            if let Some(owner) = result.options_owner.get(&task.id) {
                owned.entry(owner.clone()).or_default().push(task);
            }
        }
        for task in &view.tasks {
            if result.options_owner.contains_key(&task.id) {
                continue;
            }
            let mut stages = Vec::new();
            for options in owned.get(task.id.as_str()).into_iter().flatten() {
                task_stages(view, options, "Options", &mut stages);
            }
            task_stages(
                view,
                task,
                if task.kind == "options" {
                    "Options"
                } else {
                    "Worker"
                },
                &mut stages,
            );
            result.push(WorkUnit {
                id: WorkUnitId::Task(task.id.clone()),
                card: format!("worker:{}", task.id),
                title: task.title.clone(),
                task: Some(task.id.clone()),
                stages,
            });
        }
        if view.agents.contains_key(AUDITOR) {
            result.push(WorkUnit {
                id: WorkUnitId::FinalAudit,
                card: AUDITOR.into(),
                title: "Final audit".into(),
                task: None,
                stages: vec![stage(AUDITOR, "Auditor")],
            });
        }
        let mut remaining: Vec<_> = view
            .agents
            .keys()
            .filter(|key| !result.by_stage.contains_key(*key))
            .cloned()
            .collect();
        remaining.sort_unstable();
        for key in remaining {
            // Unknown ownership is deliberately represented as independent work.
            result.push(WorkUnit {
                id: WorkUnitId::Other(key.clone()),
                card: key.clone(),
                title: super::view::agent_name(&key),
                task: None,
                stages: vec![stage(&key, &super::view::agent_name(&key))],
            });
        }
        result
    }

    fn push(&mut self, unit: WorkUnit) {
        let index = self.units.len();
        self.by_stage.insert(unit.card.clone(), index);
        for stage in &unit.stages {
            self.by_stage.insert(stage.id.0.clone(), index);
        }
        if let Some(task) = &unit.task {
            self.by_stage.insert(format!("task:{task}"), index);
        }
        self.by_id.insert(unit.id.clone(), index);
        self.units.push(unit);
    }

    pub fn for_stage(&self, key: &str) -> Option<&WorkUnit> {
        self.by_stage.get(key).map(|index| &self.units[*index])
    }

    pub fn get(&self, id: &WorkUnitId) -> Option<&WorkUnit> {
        self.by_id.get(id).map(|index| &self.units[*index])
    }

    pub fn card_key(&self, key: &str) -> String {
        self.for_stage(key)
            .map_or_else(|| key.into(), |unit| unit.card.clone())
    }
}

impl WorkUnit {
    pub fn active_stage<'a>(&'a self, view: &View) -> &'a WorkStage {
        self.stages
            .iter()
            .find(|stage| {
                view.agents
                    .get(&stage.id.0)
                    .is_some_and(|agent| agent.running)
            })
            .or_else(|| {
                self.stages
                    .iter()
                    .find(|stage| graph::health(view, &stage.id.0) == Health::Working)
            })
            .or_else(|| {
                if self.task.is_some() {
                    self.stages.iter().find(|stage| stage.id.0 == self.card)
                } else {
                    self.stages
                        .iter()
                        .rev()
                        .find(|stage| view.agents.contains_key(&stage.id.0))
                }
            })
            .unwrap_or(&self.stages[0])
    }

    pub fn health(&self, view: &View) -> Health {
        let states: Vec<_> = self
            .stages
            .iter()
            .map(|stage| graph::health(view, &stage.id.0))
            .collect();
        if states.contains(&Health::Problem) {
            Health::Problem
        } else if self.decisions(view) > 0 {
            Health::Waiting
        } else if states.contains(&Health::Working) {
            Health::Working
        } else if states.iter().all(|state| *state == Health::Done) {
            Health::Done
        } else if states.contains(&Health::Waiting) {
            Health::Waiting
        } else {
            Health::Idle
        }
    }

    pub fn decisions(&self, view: &View) -> usize {
        view.approvals
            .iter()
            .filter(|approval| {
                self.stages
                    .iter()
                    .any(|stage| stage.id.0 == view.agent_key(&approval.role))
            })
            .count()
            + usize::from(
                self.id == WorkUnitId::Orchestration
                    && view.interview_question.is_some()
                    && (view.user_decision || view.workflow_phase.as_deref() == Some("interview")),
            )
    }

    pub fn elapsed_ms(&self, view: &View) -> u64 {
        let now = crate::lean::store::now_ms();
        self.stages
            .iter()
            .filter_map(|stage| view.agents.get(&stage.id.0))
            .map(|agent| {
                agent.busy_ms
                    + if agent.running {
                        now.saturating_sub(agent.started_ms.unwrap_or(now)).max(0) as u64
                    } else {
                        0
                    }
            })
            .sum()
    }
}

fn stage(key: &str, label: &str) -> WorkStage {
    WorkStage {
        id: StageId(key.into()),
        label: label.into(),
    }
}

fn task_stages(view: &View, task: &TaskRow, label: &str, stages: &mut Vec<WorkStage>) {
    stages.push(stage(&format!("worker:{}", task.id), label));
    let reviewer = format!("reviewer:{}", task.id);
    if task.reviewer_summary.is_some()
        || view.agents.contains_key(&reviewer)
        || task.status == "reviewing"
    {
        stages.push(stage(
            &reviewer,
            if task.kind == "options" {
                "Options review"
            } else {
                "Reviewer"
            },
        ));
    }
}

fn options_owners(view: &View) -> HashMap<String, String> {
    let ids: HashSet<_> = view
        .tasks
        .iter()
        .filter(|task| task.kind != "options")
        .map(|task| task.id.as_str())
        .collect();
    let mut consumers: HashMap<&str, Vec<&str>> = HashMap::new();
    for task in &view.tasks {
        for dependency in &task.depends_on {
            consumers.entry(dependency).or_default().push(&task.id);
        }
    }
    view.tasks
        .iter()
        .filter(|task| task.kind == "options")
        .filter_map(|task| {
            let owner = view.task_meta.get(&task.id)?.options_for.as_deref()?;
            let consumers = consumers.get(task.id.as_str())?;
            (ids.contains(owner) && consumers.iter().all(|consumer| *consumer == owner))
                .then(|| (task.id.clone(), owner.to_owned()))
        })
        .collect()
}

#[derive(Debug, Clone)]
pub enum InspectorAction {
    Stage(String),
    Decisions,
    Results(String),
    Diff(String),
    Recovery(String),
}
