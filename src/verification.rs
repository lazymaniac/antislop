use crate::context::citations::Resolver;
use crate::{Result, domain::*, hash, id, require};
use std::collections::{BTreeMap, BTreeSet};

fn source_current(sources: &mut Resolver<'_>, reference: &str, expected: &str) -> bool {
    sources.matches(reference, expected).unwrap_or(false)
}

fn review_hash(review: &Review) -> String {
    hash(serde_json::to_vec(review).expect("review serialization"))
}

fn revalidation_is_authentic(
    run: &Run,
    node_id: &str,
    binding: &ProvenanceValidation,
    sources: &BTreeMap<String, String>,
) -> bool {
    let review = run
        .provenance_reviews
        .get(&binding.review_hash)
        .or_else(|| {
            run.reviews
                .iter()
                .find(|review| review_hash(review) == binding.review_hash)
        });
    let Some(review) = review else {
        return false;
    };
    let Some(check) = review
        .checks
        .iter()
        .find(|check| check.check_id == format!("REVALIDATE:{node_id}"))
    else {
        return false;
    };
    let snapshot_hash =
        hash(serde_json::to_vec(&run.input_snapshot).expect("snapshot serialization"));
    review_hash(review) == binding.review_hash
        && review.spec_revision == run.spec_revision
        && review.checklist_version == run.checklist_version
        && review.findings.is_empty()
        && review.verifier_session == binding.verifier_session
        && review.verifier_session != review.executor_session
        && !review.verifier_session.trim().is_empty()
        && review.artifact_hash == binding.artifact_hash
        && check.status == CheckStatus::Pass
        && check.evidence_ids == binding.evidence_ids
        && !check.evidence_ids.is_empty()
        && binding
            .source_hashes
            .iter()
            .all(|(reference, value)| sources.get(reference) == Some(value))
        && check.evidence_ids.iter().any(|id| {
            run.evidence.iter().any(|evidence| {
                evidence.id == *id
                    && evidence.source == "runtime:input-snapshot"
                    && evidence.success
                    && evidence.spec_revision == run.spec_revision
                    && evidence.input_hash.as_deref() == Some(snapshot_hash.as_str())
                    && evidence.output_hash == snapshot_hash
                    && evidence.output_hash == hash(&evidence.output)
            })
        })
}

fn current_node(
    run: &Run,
    node_id: &str,
    sources: &mut Resolver<'_>,
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
            && node.superseded_by.is_none()
            && (!node.refresh || node.revalidation.is_some())
            && node.revalidation.as_ref().is_none_or(|binding| {
                revalidation_is_authentic(run, node_id, binding, sources.canonical())
            })
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
                || node_id.strip_prefix("source:").is_some_and(|reference| {
                    source_current(sources, reference, &node.content_hash)
                        || (node_id
                            == format!(
                                "source:derived:{}:{}",
                                hash(&node.content),
                                node.content_hash
                            )
                            && source_current(sources, &node.content, &node.content_hash))
                }))
            && node
                .revalidation
                .as_ref()
                .map(|binding| &binding.source_hashes)
                .unwrap_or(&node.source_hashes)
                .iter()
                .all(|(key, value)| source_current(sources, key, value))
            && node
                .revalidation
                .as_ref()
                .map(|binding| &binding.dependencies)
                .unwrap_or(&node.dependencies)
                .iter()
                .all(|(dependency, binding)| {
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
        &mut Resolver::new(run),
        &mut BTreeSet::new(),
        &mut BTreeMap::new(),
    )
}

pub fn provenance_validity(run: &Run) -> BTreeMap<String, bool> {
    let mut sources = Resolver::new(run);
    let mut cache = BTreeMap::new();
    for node_id in run.provenance.keys() {
        current_node(run, node_id, &mut sources, &mut BTreeSet::new(), &mut cache);
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
                superseded_by: None,
                refresh: false,
            },
        );
    }
    let mut cache = BTreeMap::new();
    let mut citations = Resolver::with_hashes(run, sources);
    for node_id in run.provenance.keys() {
        current_node(
            run,
            node_id,
            &mut citations,
            &mut BTreeSet::new(),
            &mut cache,
        );
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
    insert_provenance(
        run,
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
            superseded_by: None,
            refresh: false,
        },
    );
    Ok(())
}

fn insert_provenance(run: &mut Run, node_id: String, node: ProvenanceNode) {
    if let Some(old) = run.provenance.get(&node_id)
        && old.kind != ProvenanceKind::Source
        && old.binding_hash() != node.binding_hash()
    {
        let history_id = format!("history:{node_id}:{}", old.binding_hash());
        let mut archived = old.clone();
        archived.superseded_by = Some(node_id.clone());
        run.provenance.entry(history_id).or_insert(archived);
    }
    run.provenance.insert(node_id, node);
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
    let pinned: ExecutionOutput = serde_json::from_str(&artifact.content)?;
    require(
        artifact.hash == hash(&artifact.content)
            && serde_json::to_value(&pinned)? == serde_json::to_value(output)?,
        "provenance output differs from the pinned executor artifact",
    )?;
    refresh_provenance(run);
    let mut dependencies = Vec::new();
    if artifact.stage == Stage::Discovery {
        let context = output
            .context
            .as_ref()
            .ok_or_else(|| crate::Error::Invalid("missing context provenance".into()))?;
        let direct_sources = {
            let mut citations = Resolver::new(run);
            context
                .facts
                .iter()
                .map(|fact| {
                    require(
                        citations.matches(&fact.source_ref, &fact.source_hash)?,
                        "context source missing or stale",
                    )?;
                    Ok(citations.canonical().contains_key(&fact.source_ref))
                })
                .collect::<Result<Vec<_>>>()?
        };
        let mut facts = Vec::new();
        for (index, (fact, direct)) in context.facts.iter().zip(direct_sources).enumerate() {
            let source_id = if direct {
                format!("source:{}", fact.source_ref)
            } else {
                format!(
                    "source:derived:{}:{}",
                    hash(&fact.source_ref),
                    fact.source_hash
                )
            };
            if !direct {
                insert_provenance(
                    run,
                    source_id.clone(),
                    ProvenanceNode {
                        kind: ProvenanceKind::Source,
                        spec_revision: run.spec_revision,
                        content_hash: fact.source_hash.clone(),
                        content: fact.source_ref.clone(),
                        source_hashes: BTreeMap::new(),
                        dependencies: BTreeMap::new(),
                        valid: true,
                        revalidation: None,
                        superseded_by: None,
                        refresh: false,
                    },
                );
            }
            let node_id = format!("fact:{}:{index}", artifact.id);
            add_node(
                run,
                node_id.clone(),
                ProvenanceKind::ContextFact,
                serde_json::to_string(fact)?,
                BTreeMap::from([(fact.source_ref.clone(), fact.source_hash.clone())]),
                [source_id],
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
    } else if artifact.stage != Stage::Discovery
        && let Some(model_id) = latest_node(run, Stage::Planning, "model")
    {
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

fn typed_claim(kind: &ProvenanceKind) -> bool {
    matches!(
        kind,
        ProvenanceKind::ContextFact
            | ProvenanceKind::ContextModel
            | ProvenanceKind::SolutionModel
            | ProvenanceKind::Simulation
    )
}
pub fn needs_context_refresh(run: &Run) -> bool {
    run.provenance.iter().any(|(id, node)| {
        node.spec_revision == run.spec_revision
            && node.superseded_by.is_none()
            && matches!(
                node.kind,
                ProvenanceKind::ContextFact | ProvenanceKind::ContextModel
            )
            && !provenance_node_current(run, id)
    })
}
pub fn needs_solution_refresh(run: &Run) -> bool {
    run.provenance.iter().any(|(id, node)| {
        node.spec_revision == run.spec_revision
            && node.superseded_by.is_none()
            && matches!(
                node.kind,
                ProvenanceKind::SolutionModel | ProvenanceKind::Simulation
            )
            && !provenance_node_current(run, id)
    })
}
fn resolved_node(run: &Run, node_id: &str) -> Result<String> {
    let mut current = node_id.to_string();
    let mut seen = BTreeSet::new();
    while let Some(next) = run
        .provenance
        .get(&current)
        .and_then(|node| node.superseded_by.clone())
    {
        require(
            seen.insert(current.clone()),
            "cyclic provenance supersession",
        )?;
        current = next;
    }
    Ok(current)
}

pub fn register_refresh(run: &mut Run, artifact: Artifact, output: &ExecutionOutput) -> Result<()> {
    require(
        matches!(artifact.stage, Stage::Discovery | Stage::Planning)
            && artifact.spec_revision == run.spec_revision
            && artifact.hash == hash(&artifact.content)
            && output.edits.is_empty(),
        "refresh must be a pinned executor context/model artifact without edits",
    )?;
    let mut scoped = run.clone();
    scoped.stage = artifact.stage;
    validate_output(&scoped, output)?;
    let retired: Vec<_> = run
        .provenance
        .iter()
        .filter(|(_, node)| {
            node.spec_revision == run.spec_revision
                && node.superseded_by.is_none()
                && if artifact.stage == Stage::Discovery {
                    matches!(
                        node.kind,
                        ProvenanceKind::ContextFact | ProvenanceKind::ContextModel
                    )
                } else {
                    matches!(
                        node.kind,
                        ProvenanceKind::SolutionModel | ProvenanceKind::Simulation
                    )
                }
        })
        .map(|(id, _)| id.clone())
        .collect();
    if artifact.stage == Stage::Discovery {
        let expected: BTreeSet<_> = retired
            .iter()
            .filter_map(|id| run.provenance.get(id))
            .filter(|node| node.kind == ProvenanceKind::ContextFact)
            .filter_map(|node| serde_json::from_str::<ContextFact>(&node.content).ok())
            .map(|fact| fact.source_ref)
            .collect();
        let actual: BTreeSet<_> = output
            .context
            .as_ref()
            .expect("validated refresh context")
            .facts
            .iter()
            .map(|fact| fact.source_ref.clone())
            .collect();
        require(
            expected.is_subset(&actual),
            "replacement context omits a previously tracked fact source",
        )?;
    } else {
        let expected: BTreeSet<_> = retired
            .iter()
            .filter_map(|id| run.provenance.get(id))
            .filter(|node| node.kind == ProvenanceKind::SolutionModel)
            .filter_map(|node| serde_json::from_str::<SolutionModel>(&node.content).ok())
            .flat_map(|model| model.invariants.into_iter().map(|invariant| invariant.id))
            .collect();
        let actual: BTreeSet<_> = output
            .solution
            .as_ref()
            .expect("validated refresh model")
            .invariants
            .iter()
            .map(|invariant| invariant.id.clone())
            .collect();
        require(
            expected.is_subset(&actual),
            "replacement solution omits a previously required invariant",
        )?;
    }
    let old_artifacts: Vec<_> = run
        .artifacts
        .iter()
        .filter(|old| old.stage == artifact.stage && old.spec_revision == run.spec_revision)
        .map(|old| format!("artifact:{}", old.id))
        .collect();
    let insertion = run.artifacts.len().saturating_sub(1);
    run.artifacts.insert(insertion, artifact.clone());
    // register_artifact finds the latest stage node, so temporarily make fresh
    // last while registering; restore the primary current-stage artifact last.
    let primary = run.artifacts.pop();
    run.artifacts.push(artifact.clone());
    register_artifact(run, &artifact, output)?;
    run.artifacts.pop();
    if let Some(primary) = primary {
        run.artifacts.push(primary);
    }
    let replacement = if artifact.stage == Stage::Discovery {
        format!("context:{}", artifact.id)
    } else {
        format!("model:{}", artifact.id)
    };
    for id in retired {
        run.provenance
            .get_mut(&id)
            .expect("retired claim")
            .superseded_by = Some(replacement.clone());
    }
    let fresh_artifact_id = format!("artifact:{}", artifact.id);
    for id in &old_artifacts {
        if let Some(node) = run.provenance.get_mut(id) {
            node.superseded_by = Some(fresh_artifact_id.clone());
        }
    }
    for node in run.provenance.values_mut() {
        if node.kind == ProvenanceKind::Review
            && node
                .dependencies
                .keys()
                .any(|id| old_artifacts.contains(id))
        {
            node.superseded_by = Some(fresh_artifact_id.clone());
        }
    }
    for (id, node) in &mut run.provenance {
        if id.contains(&artifact.id) {
            node.refresh = true;
        }
    }
    run.evidence.push(observe(&artifact));
    refresh_provenance(run);
    Ok(())
}

pub fn refresh_review_scope(run: &Run, artifact: &Artifact) -> Run {
    let mut scoped = run.clone();
    scoped.stage = artifact.stage;
    scoped.quality_gates.clear();
    scoped.risk_coverage = risk_coverage();
    scoped
        .artifacts
        .retain(|candidate| candidate.id != artifact.id);
    scoped.artifacts.push(artifact.clone());
    let mut keep: BTreeSet<_> = scoped
        .provenance
        .keys()
        .filter(|id| id.contains(&artifact.id) || id.starts_with("source:"))
        .cloned()
        .collect();
    loop {
        let before = keep.len();
        for id in keep.clone() {
            if let Some(node) = scoped.provenance.get(&id) {
                keep.extend(node.dependencies.keys().cloned());
            }
        }
        if before == keep.len() {
            break;
        }
    }
    scoped.provenance.retain(|id, _| keep.contains(id));
    scoped
}
pub fn accept_refresh_review(run: &mut Run, artifact: &Artifact, review: &Review) -> Result<()> {
    let mut scoped = refresh_review_scope(run, artifact);
    validate_review(&scoped, artifact, review)?;
    record_review(&mut scoped, artifact, review)?;
    for (id, node) in scoped.provenance {
        if id.contains(&artifact.id)
            || node.kind == ProvenanceKind::RiskAssessment
            || id == format!("review:{}", review_hash(review))
        {
            insert_provenance(run, id, node);
        }
    }
    run.provenance_reviews.extend(scoped.provenance_reviews);
    run.reviews.push(review.clone());
    // Fresh role assessments replace stale risk assessments owned by that role.
    for (id, entry) in scoped.risk_coverage {
        if entry.stage == artifact.stage {
            run.risk_coverage.insert(id, entry);
        }
    }
    for (id, gate) in scoped.quality_gates {
        run.quality_gates.insert(id, gate);
    }
    refresh_provenance(run);
    Ok(())
}

fn apply_revalidations(run: &mut Run, artifact: &Artifact, review: &Review) -> Result<()> {
    run.provenance_reviews
        .insert(review_hash(review), review.clone());
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
                && evidence.input_hash.as_deref() == Some(current_snapshot_hash.as_str())
                && evidence.output_hash == current_snapshot_hash
                && evidence.output_hash == hash(&evidence.output)
        })
        .map(|evidence| evidence.id.clone())
        .collect();
    let mut dependency_cache = BTreeMap::new();
    while !pending.is_empty() {
        let before = pending.len();
        let ready: Vec<_> = pending
            .keys()
            .filter(|id| {
                run.provenance.get(*id).is_some_and(|node| {
                    node.dependencies.keys().all(|dependency| {
                        resolved_node(run, dependency).is_ok_and(|id| !pending.contains_key(&id))
                    })
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
            require(
                !typed_claim(&old.kind)
                    || (old.refresh
                        && old
                            .source_hashes
                            .iter()
                            .all(|(reference, digest)| sources.get(reference) == Some(digest))
                        && old
                            .revalidation
                            .as_ref()
                            .is_none_or(|binding| binding.review_hash == current_review_hash)),
                &format!(
                    "stale typed claim {id} requires a new executor-produced replacement, not source-hash rebinding"
                ),
            )?;
            let dependencies = old
                .dependencies
                .keys()
                .map(|dependency| {
                    let dependency = resolved_node(run, dependency)?;
                    require(
                        current_node(
                            run,
                            &dependency,
                            &mut Resolver::with_hashes(run, sources.clone()),
                            &mut BTreeSet::new(),
                            &mut dependency_cache,
                        ),
                        "revalidation dependency remains stale",
                    )?;
                    Ok((
                        dependency.clone(),
                        run.provenance[&dependency].binding_hash(),
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
            node.revalidation = Some(ProvenanceValidation {
                review_hash: current_review_hash.clone(),
                verifier_session: review.verifier_session.clone(),
                artifact_hash: artifact.hash.clone(),
                evidence_ids: check.evidence_ids.clone(),
                source_hashes: updated_sources,
                dependencies,
            });
            node.valid = true;
            dependency_cache.insert(id, true);
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
    if artifact_node_id(run, &artifact.hash).is_some() {
        crate::quality::record_review(run, artifact, review)?;
    }
    refresh_provenance(run);
    Ok(())
}

pub fn risk_assessment_is_current(run: &Run, entry: &RiskCoverageEntry) -> bool {
    let context = AssessmentContext::new(run);
    risk_assessment_current(run, entry, &context)
}

/// An observation of binding authentication only, not a review or acceptance.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PriorRiskBindingSummary {
    pub schema_version: u32,
    pub run_id: String,
    pub spec_revision: u64,
    pub attempt: u64,
    pub invocation_stage: Stage,
    pub checklist_version: String,
    pub observation_scope_hash: String,
    pub input_hash: String,
    pub source_hashes_hash: String,
    pub checked_prior_risk_count: usize,
    pub current_prior_risk_count: usize,
    pub stale_or_missing_prior_risk_ids: BTreeSet<String>,
    pub all_checked_prior_bindings_current: bool,
}

/// Share the exact acceptance predicate's immutable indexes across all earlier
/// stages. Current and future stage risks are outside this observation's scope.
pub fn prior_risk_binding_summary(run: &Run) -> PriorRiskBindingSummary {
    const STAGES: [Stage; 6] = [
        Stage::Discovery,
        Stage::Planning,
        Stage::Execution,
        Stage::Validation,
        Stage::Integration,
        Stage::Delivery,
    ];
    let earlier: Vec<_> = STAGES
        .into_iter()
        .take_while(|stage| *stage != run.stage)
        .collect();
    let context = AssessmentContext::new(run);
    let mut checked = 0;
    let mut current = 0;
    let mut stale = BTreeSet::new();
    for entry in run
        .risk_coverage
        .values()
        .filter(|entry| earlier.contains(&entry.stage))
    {
        checked += 1;
        if risk_assessment_current(run, entry, &context) {
            current += 1;
        } else {
            stale.insert(entry.risk_id.clone());
        }
    }
    PriorRiskBindingSummary {
        schema_version: 1,
        run_id: run.id.clone(),
        spec_revision: run.spec_revision,
        attempt: run.attempt,
        invocation_stage: run.stage,
        checklist_version: run.checklist_version.clone(),
        observation_scope_hash: run.scope_hash("risk-binding-observation"),
        input_hash: context.input_hash,
        source_hashes_hash: hash(
            serde_json::to_vec(&context.sources).expect("source hashes serialization"),
        ),
        checked_prior_risk_count: checked,
        current_prior_risk_count: current,
        all_checked_prior_bindings_current: stale.is_empty(),
        stale_or_missing_prior_risk_ids: stale,
    }
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
                            && evidence.input_hash.as_deref() == Some(context.input_hash.as_str())
                            && evidence.output_hash == context.input_hash
                    } else if evidence.source == "runtime:artifact-read" {
                        historical_artifact_identity(run, evidence)
                    } else {
                        evidence_is_current(run, artifact, evidence)
                    }
            })
        })
}

/// An already authenticated, source-current assessment may compare a replaced
/// artifact with its replacement. The read still proves that immutable text's
/// identity; it does not make the old artifact or its claims current. Fresh
/// check validation deliberately continues to use evidence_is_current alone.
fn historical_artifact_identity(run: &Run, evidence: &Evidence) -> bool {
    evidence.source == "runtime:artifact-read"
        && evidence.spec_revision == run.spec_revision
        && evidence.success
        && evidence.artifact_hash == evidence.output_hash
        && evidence.output_hash == hash(&evidence.output)
        && run.artifacts.iter().any(|artifact| {
            artifact.spec_revision == run.spec_revision
                && artifact.hash == evidence.artifact_hash
                && artifact.hash == hash(&artifact.content)
                && artifact.content == evidence.output
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
        input_hash: None,
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
            check.status != CheckStatus::NotApplicable
                || (check.check_id.starts_with("RISK:")
                    || crate::quality::allows_not_applicable(&check.check_id)),
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
    if artifact_node_id(run, &artifact.hash).is_some() {
        crate::quality::validate_review(run, artifact, review)?;
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
    if !run.artifacts.is_empty() {
        crate::quality::validate_artifacts(run, output)?;
    }
    require(!output.artifact.trim().is_empty(), "empty artifact")?;
    if run.stage == Stage::Discovery {
        let context = output
            .context
            .as_ref()
            .ok_or_else(|| crate::Error::Invalid("Discovery requires ContextModel".into()))?;
        validate_context_model(run, context)?;
    }
    if run.stage == Stage::Planning {
        let model = output
            .solution
            .as_ref()
            .ok_or_else(|| crate::Error::Invalid("Planning requires SolutionModel".into()))?;
        validate_solution_model(run, model)?;
    }
    Ok(())
}

pub(crate) fn validate_context_model(run: &Run, context: &ContextModel) -> Result<()> {
    require(!context.facts.is_empty(), "context has no sourced facts")?;
    let mut citations = Resolver::new(run);
    for fact in &context.facts {
        require(
            !fact.claim.trim().is_empty()
                && citations.matches(&fact.source_ref, &fact.source_hash)?,
            "context source missing or stale",
        )?;
    }
    for unknown in &context.unknowns {
        require(
            !unknown.question.trim().is_empty()
                && unknown
                    .resolution
                    .as_ref()
                    .is_none_or(|v| !v.trim().is_empty())
                && unknown
                    .source_ref
                    .as_ref()
                    .map(|source| citations.exists(source))
                    .transpose()?
                    .unwrap_or(true),
            "context unknown has no question or claims a missing resolution source",
        )?;
        if unknown.material {
            require(
                unknown
                    .resolution
                    .as_ref()
                    .is_some_and(|s| !s.trim().is_empty())
                    && unknown
                        .source_ref
                        .as_ref()
                        .map(|s| citations.exists(s))
                        .transpose()?
                        .unwrap_or(false),
                "material unknown remains unresolved",
            )?;
        }
    }
    Ok(())
}

pub(crate) fn validate_solution_model(run: &Run, model: &SolutionModel) -> Result<()> {
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
    let requirements: BTreeSet<_> = run.spec.requirements.iter().map(|r| r.id.clone()).collect();
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
    require(
        invariants.len() == model.invariants.len(),
        "duplicate model invariant identity",
    )?;
    for scenario in &model.simulations {
        require(
            !scenario.initial_state.trim().is_empty()
                && !scenario.events.is_empty()
                && scenario.events.iter().all(|event| !event.trim().is_empty())
                && !scenario.expected_result.trim().is_empty()
                && scenario.invariant_ids == invariants,
            "incomplete simulation or invariant coverage",
        )?;
    }
    Ok(())
}

/// Earlier evidence may be cited only when the runtime recorded the
/// exact same input snapshot. Current-stage checks still run and gate separately.
pub fn evidence_is_current(run: &Run, artifact: &Artifact, evidence: &Evidence) -> bool {
    if evidence.spec_revision != run.spec_revision {
        return false;
    }
    if evidence.source.starts_with("runner:baseline:") {
        if run
            .task_checks
            .iter()
            .any(|i| i.evidence_id.as_ref() == Some(&evidence.id))
            || run.team.as_ref().is_some_and(|team| {
                team.tasks.iter().any(|task| {
                    task.artifact.as_ref().is_some_and(|a| a.id == artifact.id)
                        || task
                            .history
                            .iter()
                            .filter_map(|h| h.artifact.as_ref())
                            .any(|a| a.id == artifact.id)
                })
            })
        {
            return crate::runtime::team::checks::baseline_for_artifact(run, artifact, evidence);
        }
        return true; // Legacy fixed-stage baseline; never a positive runner receipt.
    }
    if evidence.source == "runtime:artifact-read" {
        // This immutable receipt proves only the identity of observed text.
        return (evidence.artifact_hash == artifact.hash
            || artifact_node_id(run, &evidence.artifact_hash)
                .is_none_or(|node_id| provenance_node_current(run, &node_id)))
            && evidence.success
            && evidence.output_hash == hash(&evidence.output)
            && evidence.artifact_hash == evidence.output_hash
            && (evidence.artifact_hash == artifact.hash
                || run
                    .artifacts
                    .iter()
                    .any(|candidate| candidate.hash == evidence.artifact_hash));
    }
    let current_hash =
        hash(serde_json::to_vec(&run.input_snapshot).expect("snapshot serialization"));
    if evidence.source == "runtime:orchestration" {
        let Ok(context) = crate::transport::decode::<serde_json::Value>(evidence.output.as_bytes())
        else {
            return false;
        };
        return evidence.success
            && evidence.artifact_hash == artifact.hash
            && evidence.output_hash == hash(&evidence.output)
            && evidence.input_hash.as_deref() == Some(current_hash.as_str())
            && context["run_id"].as_str() == Some(run.id.as_str())
            && context["spec_revision"].as_u64() == Some(run.spec_revision)
            && context["attempt"].as_u64() == Some(artifact.attempt)
            && context["checklist_version"].as_str() == Some(run.checklist_version.as_str());
    }
    evidence.input_hash.as_deref() == Some(current_hash.as_str())
        && if evidence.source == "runtime:input-snapshot" {
            evidence.output_hash == current_hash
        } else {
            run.evidence.iter().any(|snapshot| {
                snapshot.source == "runtime:input-snapshot"
                    && snapshot.spec_revision == run.spec_revision
                    && snapshot.success
                    && snapshot.input_hash.as_deref() == Some(current_hash.as_str())
                    && snapshot.output_hash == current_hash
                    && snapshot.output_hash == hash(&snapshot.output)
            })
        }
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
        input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot)?)),
    })
}
