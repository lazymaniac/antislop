use crate::{Result, domain::*, hash, id, require};
use std::collections::{BTreeMap, BTreeSet};

fn review_hash(review: &Review) -> String {
    hash(serde_json::to_vec(review).expect("review serialization"))
}

fn current_node(
    run: &Run,
    node_id: &str,
    sources: &BTreeMap<String, String>,
    visiting: &mut BTreeSet<String>,
    cache: &mut BTreeMap<String, bool>,
) -> bool {
    if let Some(valid) = cache.get(node_id) {
        return *valid;
    }
    if !visiting.insert(node_id.to_owned()) {
        return false;
    }
    let valid = run.provenance.get(node_id).is_some_and(|node| {
        node.spec_revision == run.spec_revision
            && match node.kind {
                ProvenanceKind::Source => true,
                ProvenanceKind::Artifact => run.artifacts.iter().any(|artifact| {
                    format!("artifact:{}", artifact.id) == node_id
                        && artifact.spec_revision == run.spec_revision
                        && artifact.hash == node.content_hash
                        && artifact.hash == hash(&artifact.content)
                }),
                _ => node.content_hash == hash(&node.content),
            }
            && (node.kind != ProvenanceKind::Source
                || node_id
                    .strip_prefix("source:")
                    .is_some_and(|reference| sources.get(reference) == Some(&node.content_hash)))
            && node
                .source_hashes
                .iter()
                .all(|(key, value)| sources.get(key) == Some(value))
            && node.dependencies.iter().all(|(dependency, binding)| {
                run.provenance.get(dependency).is_some_and(|upstream| {
                    upstream.binding_hash() == *binding
                        && current_node(run, dependency, sources, visiting, cache)
                })
            })
    });
    visiting.remove(node_id);
    cache.insert(node_id.to_owned(), valid);
    valid
}

pub fn provenance_node_current(run: &Run, node_id: &str) -> bool {
    current_node(
        run,
        node_id,
        &run.source_hashes(),
        &mut BTreeSet::new(),
        &mut BTreeMap::new(),
    )
}

pub fn provenance_validity(run: &Run) -> BTreeMap<String, bool> {
    let sources = run.source_hashes();
    let mut cache = BTreeMap::new();
    for node_id in run.provenance.keys() {
        current_node(run, node_id, &sources, &mut BTreeSet::new(), &mut cache);
    }
    cache
}

/// Recompute validity from actual source and dependency hashes; cached `valid`
/// flags are presentation data and never authorize delivery or evidence reuse.
pub fn refresh_provenance(run: &mut Run) {
    let sources = run.source_hashes();
    for (reference, content_hash) in &sources {
        run.provenance.insert(
            format!("source:{reference}"),
            ProvenanceNode {
                kind: ProvenanceKind::Source,
                spec_revision: run.spec_revision,
                content_hash: content_hash.clone(),
                content: reference.clone(),
                source_hashes: BTreeMap::new(),
                dependencies: BTreeMap::new(),
                valid: true,
                revalidation: None,
            },
        );
    }
    let mut cache = BTreeMap::new();
    for node_id in run.provenance.keys() {
        current_node(run, node_id, &sources, &mut BTreeSet::new(), &mut cache);
    }
    for (node_id, node) in &mut run.provenance {
        node.valid = cache.get(node_id).copied().unwrap_or(false);
    }
}

fn add_node(
    run: &mut Run,
    node_id: String,
    kind: ProvenanceKind,
    content: String,
    sources: BTreeMap<String, String>,
    dependencies: impl IntoIterator<Item = String>,
) -> Result<()> {
    let dependencies = dependencies
        .into_iter()
        .map(|id| {
            let node = run
                .provenance
                .get(&id)
                .ok_or_else(|| crate::Error::Invalid("missing provenance dependency".into()))?;
            Ok((id, node.binding_hash()))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    run.provenance.insert(
        node_id,
        ProvenanceNode {
            kind,
            spec_revision: run.spec_revision,
            content_hash: hash(&content),
            content,
            source_hashes: sources,
            dependencies,
            valid: true,
            revalidation: None,
        },
    );
    Ok(())
}

fn latest_node(run: &Run, stage: Stage, prefix: &str) -> Option<String> {
    run.artifacts
        .iter()
        .rev()
        .filter(|artifact| artifact.stage == stage && artifact.spec_revision == run.spec_revision)
        .map(|artifact| format!("{prefix}:{}", artifact.id))
        .find(|id| run.provenance.contains_key(id))
}

pub fn register_artifact(
    run: &mut Run,
    artifact: &Artifact,
    output: &ExecutionOutput,
) -> Result<()> {
    refresh_provenance(run);
    let mut dependencies = Vec::new();
    if artifact.stage == Stage::Discovery {
        let context = output
            .context
            .as_ref()
            .ok_or_else(|| crate::Error::Invalid("missing context provenance".into()))?;
        let mut facts = Vec::new();
        for (index, fact) in context.facts.iter().enumerate() {
            let node_id = format!("fact:{}:{index}", artifact.id);
            add_node(
                run,
                node_id.clone(),
                ProvenanceKind::ContextFact,
                serde_json::to_string(fact)?,
                BTreeMap::from([(fact.source_ref.clone(), fact.source_hash.clone())]),
                [format!("source:{}", fact.source_ref)],
            )?;
            facts.push(node_id);
        }
        let context_id = format!("context:{}", artifact.id);
        add_node(
            run,
            context_id.clone(),
            ProvenanceKind::ContextModel,
            serde_json::to_string(context)?,
            run.source_hashes(),
            facts,
        )?;
        dependencies.push(context_id);
    } else if let Some(context_id) = latest_node(run, Stage::Discovery, "context") {
        dependencies.push(context_id);
    }
    if artifact.stage == Stage::Planning {
        let model = output
            .solution
            .as_ref()
            .ok_or_else(|| crate::Error::Invalid("missing model provenance".into()))?;
        let model_id = format!("model:{}", artifact.id);
        add_node(
            run,
            model_id.clone(),
            ProvenanceKind::SolutionModel,
            serde_json::to_string(model)?,
            run.source_hashes(),
            dependencies.clone(),
        )?;
        dependencies.push(model_id.clone());
        for (index, scenario) in model.simulations.iter().enumerate() {
            let simulation_id = format!("simulation:{}:{index}", artifact.id);
            add_node(
                run,
                simulation_id.clone(),
                ProvenanceKind::Simulation,
                serde_json::to_string(scenario)?,
                BTreeMap::new(),
                [model_id.clone()],
            )?;
            dependencies.push(simulation_id);
        }
    } else if let Some(model_id) = latest_node(run, Stage::Planning, "model") {
        dependencies.push(model_id);
    }
    add_node(
        run,
        format!("artifact:{}", artifact.id),
        ProvenanceKind::Artifact,
        format!("Pinned {:?} artifact {}", artifact.stage, artifact.id),
        run.source_hashes(),
        dependencies,
    )?;
    run.provenance
        .get_mut(&format!("artifact:{}", artifact.id))
        .expect("registered artifact")
        .content_hash = artifact.hash.clone();
    refresh_provenance(run);
    Ok(())
}

fn artifact_node_id(run: &Run, artifact_hash: &str) -> Option<String> {
    run.artifacts
        .iter()
        .rev()
        .find(|artifact| {
            artifact.hash == artifact_hash && artifact.spec_revision == run.spec_revision
        })
        .map(|artifact| format!("artifact:{}", artifact.id))
        .filter(|node_id| run.provenance.contains_key(node_id))
}

fn apply_revalidations(run: &mut Run, artifact: &Artifact, review: &Review) -> Result<()> {
    let mut pending: BTreeMap<_, _> = review
        .checks
        .iter()
        .filter_map(|check| {
            check
                .check_id
                .strip_prefix("REVALIDATE:")
                .map(|id| (id.to_owned(), check))
        })
        .collect();
    let sources = run.source_hashes();
    let current_snapshot_hash = hash(serde_json::to_vec(&run.input_snapshot)?);
    let current_review_hash = review_hash(review);
    let source_receipts: BTreeSet<_> = run
        .evidence
        .iter()
        .filter(|evidence| {
            evidence.source == "runtime:input-snapshot"
                && evidence.spec_revision == run.spec_revision
                && evidence.success
                && evidence.output_hash == current_snapshot_hash
                && evidence.output_hash == hash(&evidence.output)
        })
        .map(|evidence| evidence.id.clone())
        .collect();
    while !pending.is_empty() {
        let before = pending.len();
        let ready: Vec<_> = pending
            .keys()
            .filter(|id| {
                run.provenance.get(*id).is_some_and(|node| {
                    node.dependencies
                        .keys()
                        .all(|dependency| !pending.contains_key(dependency))
                })
            })
            .cloned()
            .collect();
        for id in ready {
            let check = pending.remove(&id).expect("pending revalidation");
            require(
                check.status == CheckStatus::Pass
                    && check
                        .evidence_ids
                        .iter()
                        .any(|id| source_receipts.contains(id)),
                "revalidation requires passing assessment with current source-read evidence",
            )?;
            let old = run
                .provenance
                .get(&id)
                .ok_or_else(|| crate::Error::Invalid("unknown revalidation node".into()))?;
            require(
                old.spec_revision == run.spec_revision && old.kind != ProvenanceKind::Source,
                "historical revision or source cannot be revalidated",
            )?;
            let dependencies = old
                .dependencies
                .keys()
                .map(|dependency| {
                    require(
                        current_node(
                            run,
                            dependency,
                            &sources,
                            &mut BTreeSet::new(),
                            &mut BTreeMap::new(),
                        ),
                        "revalidation dependency remains stale",
                    )?;
                    Ok((
                        dependency.clone(),
                        run.provenance[dependency].binding_hash(),
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?;
            let updated_sources = old
                .source_hashes
                .keys()
                .map(|reference| {
                    let source_hash = sources.get(reference).ok_or_else(|| {
                        crate::Error::Invalid("revalidation source was removed".into())
                    })?;
                    Ok((reference.clone(), source_hash.clone()))
                })
                .collect::<Result<BTreeMap<_, _>>>()?;
            if old.kind == ProvenanceKind::Artifact {
                require(
                    run.artifacts
                        .iter()
                        .filter(|prior| format!("artifact:{}", prior.id) == id)
                        .all(|prior| prior.session_id != review.verifier_session),
                    "revalidation is not independent of original executor",
                )?;
            }
            let node = run.provenance.get_mut(&id).expect("revalidation node");
            node.source_hashes = updated_sources;
            node.dependencies = dependencies;
            node.revalidation = Some(ProvenanceValidation {
                review_hash: current_review_hash.clone(),
                verifier_session: review.verifier_session.clone(),
                artifact_hash: artifact.hash.clone(),
                evidence_ids: check.evidence_ids.clone(),
            });
            node.valid = true;
        }
        require(
            pending.len() != before,
            "cyclic or missing revalidation dependency",
        )?;
    }
    refresh_provenance(run);
    Ok(())
}

fn assessment_binding(
    spec_revision: u64,
    artifact: &Artifact,
    check: &CheckResult,
    node_id: String,
    review_hash: &str,
    sources: &BTreeMap<String, String>,
) -> RiskAssessmentBinding {
    RiskAssessmentBinding {
        node_id,
        review_hash: review_hash.into(),
        artifact_hash: artifact.hash.clone(),
        check_id: check.check_id.clone(),
        evidence_ids: check.evidence_ids.clone(),
        source_hashes: sources.clone(),
        spec_revision,
    }
}

/// Called only after validate_review succeeds. Failed reviews never update coverage.
pub fn record_review(run: &mut Run, artifact: &Artifact, review: &Review) -> Result<()> {
    apply_revalidations(run, artifact, review)?;
    let current_review_hash = review_hash(review);
    let sources = run.source_hashes();
    let review_id = format!("review:{current_review_hash}");
    let dependency = artifact_node_id(run, &artifact.hash).into_iter();
    add_node(
        run,
        review_id.clone(),
        ProvenanceKind::Review,
        format!(
            "Independent review {} of {}",
            current_review_hash, artifact.hash
        ),
        sources.clone(),
        dependency,
    )?;
    for check in &review.checks {
        if let Some(risk_id) = check.check_id.strip_prefix("RISK:") {
            let node_id = format!("risk:{}:{risk_id}", run.spec_revision);
            add_node(
                run,
                node_id.clone(),
                ProvenanceKind::RiskAssessment,
                serde_json::to_string(check)?,
                sources.clone(),
                [review_id.clone()],
            )?;
            let binding = assessment_binding(
                run.spec_revision,
                artifact,
                check,
                node_id,
                &current_review_hash,
                &sources,
            );
            if let Some(entry) = run.risk_coverage.get_mut(risk_id) {
                entry.status = check.status.clone();
                entry.rationale = check.rationale.clone();
                entry.assessment = Some(binding);
            }
        } else if let Some(node_id) = check.check_id.strip_prefix("REVALIDATE:") {
            let risk_id = run.risk_coverage.iter().find_map(|(risk_id, entry)| {
                entry
                    .assessment
                    .as_ref()
                    .filter(|binding| binding.node_id == node_id)
                    .map(|_| risk_id.clone())
            });
            if let Some(risk_id) = risk_id {
                let binding = assessment_binding(
                    run.spec_revision,
                    artifact,
                    check,
                    node_id.to_owned(),
                    &current_review_hash,
                    &sources,
                );
                let entry = run.risk_coverage.get_mut(&risk_id).expect("risk owner");
                entry.status = check.status.clone();
                entry.rationale = check.rationale.clone();
                entry.assessment = Some(binding);
            }
        }
    }
    refresh_provenance(run);
    Ok(())
}

pub fn risk_assessment_is_current(run: &Run, entry: &RiskCoverageEntry) -> bool {
    let context = AssessmentContext::new(run);
    risk_assessment_current(run, entry, &context)
}

struct AssessmentContext<'a> {
    sources: BTreeMap<String, String>,
    validity: BTreeMap<String, bool>,
    reviews: BTreeMap<String, &'a Review>,
    input_hash: String,
    intact_evidence: BTreeSet<String>,
}
impl<'a> AssessmentContext<'a> {
    fn new(run: &'a Run) -> Self {
        Self {
            sources: run.source_hashes(),
            validity: provenance_validity(run),
            reviews: run
                .reviews
                .iter()
                .map(|review| (review_hash(review), review))
                .collect(),
            input_hash: hash(
                serde_json::to_vec(&run.input_snapshot).expect("snapshot serialization"),
            ),
            intact_evidence: run
                .evidence
                .iter()
                .filter(|evidence| evidence.output_hash == hash(&evidence.output))
                .map(|evidence| evidence.id.clone())
                .collect(),
        }
    }
}

fn risk_assessment_current(
    run: &Run,
    entry: &RiskCoverageEntry,
    context: &AssessmentContext<'_>,
) -> bool {
    let Some(binding) = &entry.assessment else {
        return false;
    };
    let node_id = format!("risk:{}:{}", run.spec_revision, entry.risk_id);
    if binding.node_id != node_id
        || (binding.check_id != format!("RISK:{}", entry.risk_id)
            && binding.check_id != format!("REVALIDATE:{node_id}"))
        || binding.spec_revision != run.spec_revision
        || binding.source_hashes != context.sources
        || !context
            .validity
            .get(&binding.node_id)
            .copied()
            .unwrap_or(false)
    {
        return false;
    }
    let node = &run.provenance[&binding.node_id];
    if node.kind != ProvenanceKind::RiskAssessment {
        return false;
    }
    if binding.check_id.starts_with("REVALIDATE:") {
        if !node.revalidation.as_ref().is_some_and(|validation| {
            validation.review_hash == binding.review_hash
                && validation.artifact_hash == binding.artifact_hash
                && validation.evidence_ids == binding.evidence_ids
        }) {
            return false;
        }
    } else if !node
        .dependencies
        .contains_key(&format!("review:{}", binding.review_hash))
    {
        return false;
    }
    let Some(review) = context.reviews.get(&binding.review_hash) else {
        return false;
    };
    let Some(artifact) = run
        .artifacts
        .iter()
        .find(|artifact| artifact.hash == binding.artifact_hash)
    else {
        return false;
    };
    let Some(check) = review
        .checks
        .iter()
        .find(|check| check.check_id == binding.check_id)
    else {
        return false;
    };
    review.artifact_hash == artifact.hash
        && artifact.hash == hash(&artifact.content)
        && review.spec_revision == run.spec_revision
        && review.checklist_version == run.checklist_version
        && review.executor_session == artifact.session_id
        && review.verifier_session != review.executor_session
        && !review.verifier_session.trim().is_empty()
        && review.findings.is_empty()
        && matches!(check.status, CheckStatus::Pass | CheckStatus::NotApplicable)
        && check.status == entry.status
        && !check.rationale.trim().is_empty()
        && check.rationale == entry.rationale
        && check.evidence_ids == binding.evidence_ids
        && !binding.evidence_ids.is_empty()
        && binding.evidence_ids.iter().all(|id| {
            run.evidence.iter().any(|evidence| {
                evidence.id == *id
                    && context.intact_evidence.contains(&evidence.id)
                    && (evidence.success || evidence.source.starts_with("runner:baseline:"))
                    && if evidence.source == "runtime:input-snapshot" {
                        evidence.spec_revision == run.spec_revision
                            && evidence.output_hash == context.input_hash
                    } else {
                        evidence_is_current(run, artifact, evidence)
                    }
            })
        })
}

/// Runner-origin evidence: the runtime actually reads the pinned artifact.
/// This proves the input's identity, not the truth of the verifier's conclusion.
pub fn observe(artifact: &Artifact) -> Evidence {
    Evidence {
        id: id(),
        artifact_hash: artifact.hash.clone(),
        spec_revision: artifact.spec_revision,
        source: "runtime:artifact-read".into(),
        output_hash: hash(&artifact.content),
        output: artifact.content.clone(),
        success: true,
    }
}
pub fn validate_review(run: &Run, artifact: &Artifact, review: &Review) -> Result<()> {
    require(
        artifact.hash == hash(&artifact.content),
        "artifact content hash mismatch",
    )?;
    require(
        artifact.spec_revision == run.spec_revision && review.spec_revision == run.spec_revision,
        "stale specification",
    )?;
    require(
        review.artifact_hash == artifact.hash,
        "review targets another artifact",
    )?;
    require(
        review.executor_session == artifact.session_id
            && !review.verifier_session.trim().is_empty()
            && review.verifier_session != review.executor_session,
        "review is not independent",
    )?;
    require(
        review.checklist_version == run.checklist_version
            && run.checklist_version == checklist_version(),
        "stale checklist",
    )?;
    require(review.findings.is_empty(), "unresolved findings")?;
    require(
        run.evidence
            .iter()
            .filter(|e| e.artifact_hash == artifact.hash)
            .all(|e| e.success),
        "runner check failed",
    )?;
    let catalog = risk_coverage();
    require(
        run.risk_coverage.keys().eq(catalog.keys()),
        "risk ownership catalog is incomplete",
    )?;
    require(
        run.risk_coverage.iter().all(|(id, entry)| {
            entry.risk_id == *id
                && entry.stage == catalog[id].stage
                && entry.executor_role == catalog[id].executor_role
                && entry.verifier_role == catalog[id].verifier_role
                && entry.check_id == catalog[id].check_id
        }),
        "invalid risk owner",
    )?;
    let expected = run.expected_checks();
    let actual: BTreeSet<_> = review.checks.iter().map(|c| c.check_id.clone()).collect();
    require(
        actual == expected && actual.len() == review.checks.len(),
        "missing, duplicate or unknown checklist item",
    )?;
    let mut evidence_validity = BTreeMap::new();
    for check in &review.checks {
        require(
            matches!(check.status, CheckStatus::Pass | CheckStatus::NotApplicable),
            "failed or blocked check",
        )?;
        require(
            !check.method.trim().is_empty() && !check.rationale.trim().is_empty(),
            "missing method or rationale",
        )?;
        require(
            check.status != CheckStatus::NotApplicable || check.check_id.starts_with("RISK:"),
            "mandatory obligation cannot be N/A",
        )?;
        require(!check.evidence_ids.is_empty(), "missing evidence")?;
        for evidence_id in &check.evidence_ids {
            let e = run
                .evidence
                .iter()
                .find(|e| &e.id == evidence_id)
                .ok_or_else(|| crate::Error::Invalid("unknown evidence".into()))?;
            let valid = evidence_validity.entry(e.id.clone()).or_insert_with(|| {
                (e.success || e.source.starts_with("runner:baseline:"))
                    && evidence_is_current(run, artifact, e)
                    && e.output_hash == hash(&e.output)
            });
            require(*valid, "stale, failed or corrupted evidence")?;
        }
    }
    // Simulate the independently reviewed refresh without changing persisted
    // history. Delivery may accept fresh revalidation of prior assessments.
    let mut reviewed = run.clone();
    apply_revalidations(&mut reviewed, artifact, review)?;
    if run.stage == Stage::Delivery {
        record_review(&mut reviewed, artifact, review)?;
        reviewed.reviews.push(review.clone());
        let assessment_context = AssessmentContext::new(&reviewed);
        for entry in reviewed
            .risk_coverage
            .values()
            .filter(|entry| entry.stage != Stage::Delivery)
        {
            require(
                risk_assessment_current(&reviewed, entry, &assessment_context),
                "run has missing, stale or unbound risk evidence from an earlier stage",
            )?;
        }
    }
    Ok(())
}

/// Structural completeness only. The independent reviewer judges the substance.
pub fn validate_output(run: &Run, output: &ExecutionOutput) -> Result<()> {
    require(!output.artifact.trim().is_empty(), "empty artifact")?;
    if run.stage == Stage::Discovery {
        let context = output
            .context
            .as_ref()
            .ok_or_else(|| crate::Error::Invalid("Discovery requires ContextModel".into()))?;
        require(!context.facts.is_empty(), "context has no sourced facts")?;
        for fact in &context.facts {
            let sources = run.sources();
            let source = sources.get(&fact.source_ref);
            require(
                !fact.claim.trim().is_empty()
                    && source.is_some_and(|s| hash(s) == fact.source_hash),
                "context source missing or stale",
            )?;
        }
        for unknown in &context.unknowns {
            if unknown.material {
                require(
                    unknown
                        .resolution
                        .as_ref()
                        .is_some_and(|s| !s.trim().is_empty())
                        && unknown
                            .source_ref
                            .as_ref()
                            .is_some_and(|s| run.sources().contains_key(s)),
                    "material unknown remains unresolved",
                )?;
            }
        }
    }
    if run.stage == Stage::Planning {
        let model = output
            .solution
            .as_ref()
            .ok_or_else(|| crate::Error::Invalid("Planning requires SolutionModel".into()))?;
        require(
            !model.criteria.is_empty() && model.criteria.iter().all(|c| !c.trim().is_empty()),
            "comparison criteria missing",
        )?;
        require(
            model.options.len() >= 2 && !model.rationale.trim().is_empty(),
            "real options and decision rationale required",
        )?;
        let ids: BTreeSet<_> = model.options.iter().map(|o| &o.id).collect();
        let mechanisms: BTreeSet<_> = model.options.iter().map(|o| &o.mechanism).collect();
        require(
            ids.len() == model.options.len() && mechanisms.len() == model.options.len(),
            "duplicate options",
        )?;
        let chosen = model
            .options
            .iter()
            .find(|o| o.id == model.chosen_option)
            .ok_or_else(|| crate::Error::Invalid("chosen option missing".into()))?;
        let requirements: BTreeSet<_> =
            run.spec.requirements.iter().map(|r| r.id.clone()).collect();
        require(
            chosen.requirement_ids == requirements,
            "chosen option omits a mandatory requirement",
        )?;
        for option in &model.options {
            require(
                !option.id.trim().is_empty()
                    && !option.mechanism.trim().is_empty()
                    && !option.evidence.trim().is_empty(),
                "unsubstantiated option",
            )?;
        }
        require(
            !model.invariants.is_empty()
                && model
                    .invariants
                    .iter()
                    .all(|i| !i.id.trim().is_empty() && !i.statement.trim().is_empty()),
            "model invariants missing",
        )?;
        let kinds: BTreeSet<_> = model.simulations.iter().map(|s| s.kind.as_str()).collect();
        require(
            ["success", "failure", "counterexample"]
                .iter()
                .all(|k| kinds.contains(k)),
            "missing simulation classes",
        )?;
        let invariants: BTreeSet<_> = model.invariants.iter().map(|i| i.id.clone()).collect();
        for scenario in &model.simulations {
            require(
                !scenario.initial_state.trim().is_empty()
                    && !scenario.events.is_empty()
                    && !scenario.expected_result.trim().is_empty()
                    && scenario.invariant_ids == invariants,
                "incomplete simulation or invariant coverage",
            )?;
        }
    }
    Ok(())
}

/// Earlier evidence may be cited only when the runtime recorded the
/// exact same input snapshot. Current-stage checks still run and gate separately.
pub fn evidence_is_current(run: &Run, artifact: &Artifact, evidence: &Evidence) -> bool {
    if evidence.spec_revision != run.spec_revision {
        return false;
    }
    if evidence.source == "runtime:input-snapshot" {
        return evidence.output_hash
            == hash(serde_json::to_vec(&run.input_snapshot).expect("snapshot serialization"));
    }
    if evidence.artifact_hash == artifact.hash || evidence.source.starts_with("runner:baseline:") {
        return true;
    }
    if artifact_node_id(run, &evidence.artifact_hash)
        .is_some_and(|node_id| !provenance_node_current(run, &node_id))
    {
        return false;
    }
    let current_hash =
        hash(serde_json::to_vec(&run.input_snapshot).expect("snapshot serialization"));
    evidence.success
        && run.evidence.iter().any(|snapshot| {
            snapshot.source == "runtime:input-snapshot"
                && snapshot.spec_revision == run.spec_revision
                && snapshot.artifact_hash == evidence.artifact_hash
                && snapshot.success
                && snapshot.output_hash == hash(&snapshot.output)
                && snapshot.output_hash == current_hash
        })
}

pub fn workspace_diff(run: &Run, artifact: &Artifact) -> Result<Evidence> {
    let baseline = run
        .evidence
        .iter()
        .find(|e| e.source == "runtime:input-snapshot" && e.spec_revision == run.spec_revision)
        .ok_or_else(|| crate::Error::Invalid("original input snapshot unavailable".into()))?;
    require(
        baseline.success && baseline.output_hash == hash(&baseline.output),
        "baseline snapshot corrupted",
    )?;
    let before: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&baseline.output)?;
    let changed: Vec<_> = run
        .input_snapshot
        .iter()
        .filter(|(name, text)| before.get(*name) != Some(*text))
        .map(|(name, _)| name)
        .collect();
    let output =
        serde_json::json!({"before":before,"after":run.input_snapshot,"changed_files":changed,
        "baseline_evidence_id":baseline.id,"baseline_hash":baseline.output_hash,
        "current_hash":hash(serde_json::to_vec(&run.input_snapshot)?)})
        .to_string();
    Ok(Evidence {
        id: id(),
        artifact_hash: artifact.hash.clone(),
        spec_revision: run.spec_revision,
        source: "runtime:workspace-diff".into(),
        output_hash: hash(&output),
        output,
        success: true,
    })
}
