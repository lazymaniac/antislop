//! Feature preparation roles and execution-time delivery checks.
use super::*;
use crate::lean::prompts;

pub const FEATURE_SPECIALISTS: &[PredefinedAgent] = &[
    PredefinedAgent {
        id: "feature-research",
        kind: WorkKind::Research,
        title: prompts::FEATURE_RESEARCH_TITLE,
        focus: prompts::FEATURE_RESEARCH_FOCUS,
        required_for_code_changes: false,
    },
    PredefinedAgent {
        id: "feature-analysis",
        kind: WorkKind::Analysis,
        title: prompts::FEATURE_ANALYSIS_TITLE,
        focus: prompts::FEATURE_ANALYSIS_FOCUS,
        required_for_code_changes: false,
    },
    PredefinedAgent {
        id: "feature-dependencies",
        kind: WorkKind::Dependencies,
        title: prompts::FEATURE_DEPENDENCIES_TITLE,
        focus: prompts::FEATURE_DEPENDENCIES_FOCUS,
        required_for_code_changes: false,
    },
    PredefinedAgent {
        id: "feature-architecture",
        kind: WorkKind::Architecture,
        title: prompts::FEATURE_ARCHITECTURE_TITLE,
        focus: prompts::FEATURE_ARCHITECTURE_FOCUS,
        required_for_code_changes: false,
    },
    PredefinedAgent {
        id: "feature-test-design",
        kind: WorkKind::TestDesign,
        title: prompts::FEATURE_TEST_DESIGN_TITLE,
        focus: prompts::FEATURE_TEST_DESIGN_FOCUS,
        required_for_code_changes: false,
    },
    PredefinedAgent {
        id: "feature-delivery",
        kind: WorkKind::Delivery,
        title: prompts::FEATURE_DELIVERY_TITLE,
        focus: prompts::FEATURE_DELIVERY_FOCUS,
        required_for_code_changes: false,
    },
];

pub const FEATURE_PLANNING_RULES: &str = prompts::FEATURE_PLANNING_RULES;

/// Check the selected feature's actual graph, excluding unchanged approved writers.
pub fn validate_feature_pipeline(
    tasks: &[TaskContract],
    preserved: &BTreeSet<String>,
) -> crate::Result<()> {
    let writers: Vec<_> = tasks
        .iter()
        .filter(|t| {
            t.changes_code()
                && !t.quality_gate
                && t.options_for.is_none()
                && !preserved.contains(&t.id)
        })
        .collect();
    if writers.is_empty() {
        return Ok(());
    }
    let preparation: Vec<_> = tasks
        .iter()
        .filter(|task| {
            !task.changes_code()
                && !task.quality_gate
                && FEATURE_SPECIALISTS
                    .iter()
                    .any(|a| a.kind == task.kind && a.kind != WorkKind::Delivery)
                && !writers
                    .iter()
                    .any(|writer| depends_transitively(tasks, &task.id, &writer.id))
        })
        .collect();
    crate::require(
        tasks
            .iter()
            .any(|task| task.kind == WorkKind::Delivery && !task.changes_code()),
        "feature pipeline is missing delivery; add a final delivery report after all deliverables and checks",
    )?;
    for kind in GATE_KINDS {
        crate::require(
            tasks
                .iter()
                .any(|task| task.quality_gate && task.kind == kind),
            &format!(
                "feature pipeline is missing runtime final {}; regenerate all six feature gates",
                work_kind_name(kind)
            ),
        )?;
    }
    for (consumer, producer) in [
        (WorkKind::Dependencies, WorkKind::Research),
        (WorkKind::Architecture, WorkKind::Analysis),
        (WorkKind::Architecture, WorkKind::Dependencies),
        (WorkKind::TestDesign, WorkKind::Analysis),
    ] {
        for task in preparation
            .iter()
            .filter(|t| t.kind == consumer && !preserved.contains(&t.id))
        {
            require_feature_inputs(
                tasks,
                task,
                preparation.iter().copied().filter(|t| t.kind == producer),
            )?;
        }
    }
    for gate in tasks.iter().filter(|t| t.quality_gate) {
        require_feature_inputs(tasks, gate, writers.iter().copied())?;
    }
    for writer in writers {
        require_feature_inputs(
            tasks,
            writer,
            preparation
                .iter()
                .copied()
                .filter(|t| matches!(t.kind, WorkKind::Architecture | WorkKind::TestDesign)),
        )?;
    }
    for delivery in tasks
        .iter()
        .filter(|t| t.kind == WorkKind::Delivery && !preserved.contains(&t.id))
    {
        require_feature_inputs(
            tasks,
            delivery,
            tasks.iter().filter(|t| t.kind != WorkKind::Delivery),
        )?;
    }
    Ok(())
}

fn require_feature_inputs<'a>(
    tasks: &[TaskContract],
    consumer: &TaskContract,
    producers: impl Iterator<Item = &'a TaskContract>,
) -> crate::Result<()> {
    for producer in producers {
        crate::require(
            depends_transitively(tasks, &consumer.id, &producer.id),
            &format!(
                "feature pipeline task {} must depend on {} ({})",
                consumer.id,
                producer.id,
                work_kind_name(producer.kind)
            ),
        )?;
    }
    Ok(())
}

/// Complete authored specialist inputs for existing tests of gate mechanics.
#[cfg(test)]
pub(crate) fn feature_fixture(mut specs: Vec<TaskContract>) -> Vec<TaskContract> {
    if !specs.iter().any(TaskContract::changes_code) {
        return specs;
    }
    for agent in FEATURE_SPECIALISTS
        .iter()
        .filter(|a| a.kind != WorkKind::Delivery)
    {
        if specs.iter().any(|t| t.id == agent.id) {
            continue;
        }
        let dependencies: Vec<&str> = match agent.kind {
            WorkKind::Dependencies => vec!["feature-research"],
            WorkKind::Architecture => vec!["feature-analysis", "feature-dependencies"],
            WorkKind::TestDesign => vec!["feature-analysis"],
            _ => vec![],
        };
        specs.push(serde_json::from_value(serde_json::json!({"id":agent.id,"title":agent.title,"goal":agent.focus,"kind":agent.kind,"change_kind":"none","depends_on":dependencies,"acceptance_criteria":[{"id":"C1","text":agent.focus}]})).unwrap());
    }
    for writer in specs
        .iter_mut()
        .filter(|t| t.changes_code() && !t.quality_gate && t.options_for.is_none())
    {
        for id in ["feature-architecture", "feature-test-design"] {
            if !writer.depends_on.iter().any(|d| d == id) {
                writer.depends_on.push(id.into());
            }
        }
    }
    specs.retain(|t| t.id != "feature-delivery");
    specs = with_options(with_feature_gates(specs));
    let inputs: Vec<_> = specs.iter().map(|t| t.id.clone()).collect();
    specs.push(serde_json::from_value(serde_json::json!({"id":"feature-delivery","title":"Delivery","goal":"Reconcile delivered requirements and evidence","kind":"delivery","change_kind":"none","depends_on":inputs,"acceptance_criteria":[{"id":"C1","text":"Delivery matches observed results"}]})).unwrap());
    order_graph(specs).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<TaskContract> {
        let writer = serde_json::from_value(serde_json::json!({"id":"writer","title":"Implement","goal":"Deliver behavior","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"Behavior works"}]})).unwrap();
        feature_fixture(vec![writer])
    }
    #[test]
    fn full_pipeline_is_complete_ordered_and_idempotent() {
        let plan = fixture();
        validate_feature_pipeline(&plan, &BTreeSet::new()).unwrap();
        assert!(!depends_transitively(
            &plan,
            "feature-analysis",
            "feature-research"
        ));
        assert!(!depends_transitively(
            &plan,
            "feature-architecture",
            "feature-test-design"
        ));
        assert_eq!(plan, with_options(with_feature_gates(plan.clone())));
        assert_eq!(plan.iter().filter(|t| t.quality_gate).count(), 6);
    }
    #[test]
    fn preparation_is_optional_in_execution_but_delivery_and_declared_edges_are_required() {
        let plan = fixture();
        let without_delivery: Vec<_> = plan
            .iter()
            .filter(|t| t.kind != WorkKind::Delivery)
            .cloned()
            .collect();
        assert!(
            validate_feature_pipeline(&without_delivery, &BTreeSet::new())
                .unwrap_err()
                .to_string()
                .contains("delivery")
        );
        let mut execution: Vec<_> = plan
            .iter()
            .filter(|t| {
                !FEATURE_SPECIALISTS
                    .iter()
                    .any(|a| a.kind != WorkKind::Delivery && a.kind == t.kind)
            })
            .cloned()
            .collect();
        let ids: BTreeSet<_> = execution.iter().map(|t| t.id.clone()).collect();
        for task in &mut execution {
            task.depends_on.retain(|id| ids.contains(id));
        }
        validate_feature_pipeline(&execution, &BTreeSet::new()).unwrap();
        for (consumer, producer) in [
            ("feature-dependencies", "feature-research"),
            ("feature-architecture", "feature-dependencies"),
            ("feature-test-design", "feature-analysis"),
            ("writer", "feature-architecture"),
            ("feature-delivery", "integration-gate"),
        ] {
            let mut broken = plan.clone();
            broken
                .iter_mut()
                .find(|t| t.id == consumer)
                .unwrap()
                .depends_on
                .clear();
            let error = validate_feature_pipeline(&broken, &BTreeSet::new())
                .unwrap_err()
                .to_string();
            assert!(error.contains(consumer), "missing {producer}: {error}");
        }
    }
    #[test]
    fn preliminary_audits_cannot_replace_final_feature_gates() {
        let mut plan = fixture();
        let mut early = plan
            .iter()
            .find(|t| t.id == "security-auditor")
            .unwrap()
            .clone();
        early.id = "early-security".into();
        early.quality_gate = false;
        early.depends_on.clear();
        plan.push(early);
        let plan = with_feature_gates(plan);
        assert_eq!(
            plan.iter().filter(|t| t.kind == WorkKind::Security).count(),
            2
        );
        let missing_final: Vec<_> = plan
            .iter()
            .filter(|t| !(t.quality_gate && t.kind == WorkKind::Security))
            .cloned()
            .collect();
        let error = validate_feature_pipeline(&missing_final, &BTreeSet::new())
            .unwrap_err()
            .to_string();
        assert!(error.contains("runtime final security"), "{error}");
        assert!(depends_transitively(&plan, "security-auditor", "writer"));
        assert!(depends_transitively(
            &plan,
            "integration-gate",
            "security-auditor"
        ));
    }
    #[test]
    fn preserved_approved_writers_do_not_acquire_retroactive_requirements() {
        let writer = fixture().into_iter().find(|t| t.id == "writer").unwrap();
        validate_feature_pipeline(&[writer], &BTreeSet::from(["writer".into()])).unwrap();
    }
}
