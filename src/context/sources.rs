//! Preserve parent observations while making their original members browsable.
//! No source filesystem read, selector expansion or new permission is involved.
use super::*;
use std::path::{Component, Path};

fn root_selector(workspace: &str, selector: &str) -> bool {
    let path = Path::new(selector);
    if path.is_absolute() {
        path.components().eq(Path::new(workspace).components())
    } else {
        !selector.is_empty() && path.components().all(|p| p == Component::CurDir)
    }
}
fn current_directories(run: &crate::domain::Run) -> Result<Vec<(String, &str)>> {
    if let Some(observation) = &run.input_observation {
        observation.validate(&run.input_snapshot)?;
    }
    Ok(run
        .input_snapshot
        .iter()
        .filter(|(selector, _)| {
            run.input_observation.as_ref().map_or_else(
                || root_selector(&run.spec.workspace, selector),
                |o| o.entries[*selector].kind == crate::workspace::observation::Kind::Directory,
            )
        })
        .map(|(selector, body)| (format!("file:{selector}"), body.as_str()))
        .collect())
}

pub(super) fn current_member_hashes(run: &crate::domain::Run) -> Result<BTreeMap<String, String>> {
    let mut members = BTreeMap::new();
    for (parent, body) in current_directories(run)? {
        for (path, entry) in crate::workspace::inputs::context_members(body)? {
            if let Some(text) = entry.text {
                let reference = format!(
                    "directory-member:{}",
                    serde_json::to_string(&(&parent, &path))?
                );
                require(
                    members.insert(reference, hash(&text)).is_none(),
                    "ambiguous directory member reference",
                )?;
            }
        }
    }
    Ok(members)
}
pub(super) fn extend(
    run: &crate::domain::Run,
    prompt: &str,
    sources: &mut BTreeMap<String, String>,
) -> Result<()> {
    crate::runtime::orchestration_recovery::handoff::extend(run, prompt, sources)?;
    crate::runtime::team::replacement::context::extend(run, prompt, sources)?;
    let mut directories = Vec::new();
    if let Some(round) = run
        .orchestration_rounds
        .last()
        .filter(|r| r.status == crate::orchestration::planning::RoundStatus::Running)
    {
        round.validate_proposed_observation()?;
        if let Some(observation) = &round.proposed_input_observation {
            require(
                sources
                    .insert(
                        "orchestration-proposed-input-observation".into(),
                        serde_json::to_string(observation)?,
                    )
                    .is_none(),
                "ambiguous proposed input observation",
            )?;
        }
        for (selector, body) in &round.proposed_sources {
            require(
                sources
                    .insert(
                        format!("orchestration-proposed-source:{selector}"),
                        body.clone(),
                    )
                    .is_none(),
                "ambiguous proposed source reference",
            )?;
            if round.proposed_input_observation.as_ref().is_some_and(|o| {
                o.entries[selector].kind == crate::workspace::observation::Kind::Directory
            }) {
                directories.push((
                    format!("orchestration-proposed-source:{selector}"),
                    body.as_str(),
                ));
            }
        }
    }
    if let Some(observation) = &run.input_observation {
        observation.validate(&run.input_snapshot)?;
        require(
            sources
                .insert(
                    "input-observation".into(),
                    serde_json::to_string(observation)?,
                )
                .is_none(),
            "ambiguous input observation reference",
        )?;
    }
    directories.extend(current_directories(run)?);
    if run.intake
        && let Some(round) = run.orchestration_rounds.last()
    {
        round.validate_interview_sources()?;
        if let Some(observation) = &round.interview_input_observation {
            require(
                sources
                    .insert(
                        "interview-input-observation".into(),
                        serde_json::to_string(observation)?,
                    )
                    .is_none(),
                "ambiguous interview input observation",
            )?;
        }
        require(
            round.spec_revision == run.spec_revision,
            "interview source observation differs from its pinned context",
        )?;
        for (selector, body) in &round.observed_interview_sources {
            let reference = format!("interview-file:{selector}");
            require(
                sources.insert(reference.clone(), body.clone()).is_none(),
                "ambiguous interview source reference",
            )?;
            if round.interview_input_observation.as_ref().map_or_else(
                || root_selector(&run.spec.workspace, selector),
                |o| o.entries[selector].kind == crate::workspace::observation::Kind::Directory,
            ) {
                directories.push((reference, body.as_str()));
            }
        }
    }
    for (source_ref, body) in directories {
        let source_hash = hash(body);
        let mut members = vec![];
        for (path, entry) in crate::workspace::inputs::context_members(body)? {
            let (reference, content_hash) = if let Some(text) = entry.text {
                let reference = format!(
                    "directory-member:{}",
                    serde_json::to_string(&(&source_ref, &path))?
                );
                let content_hash = hash(&text);
                require(
                    sources.insert(reference.clone(), text).is_none(),
                    "ambiguous directory member reference",
                )?;
                (Some(reference), Some(content_hash))
            } else {
                (None, None)
            };
            members.push(serde_json::json!({"path":path,"metadata":entry.metadata,
                "source_ref":reference,"document_hash":content_hash}));
        }
        let catalog = serde_json::json!({"format":"pinned-directory-index-v1","parent_source_ref":source_ref,
            "parent_document_hash":source_hash,"members":members,
            "contract":"A browsable view of the original captured directory, not a new input list or filesystem grant. Read text through its document_hash. Binary, symlink and missing entries are metadata only; links are never followed. Parent document retains the exact original capture, selection and limitations. This index is not semantic source inspection."});
        let reference = format!("directory-index:{}", serde_json::to_string(&source_ref)?);
        require(
            sources
                .insert(reference, serde_json::to_string(&catalog)?)
                .is_none(),
            "ambiguous directory index reference",
        )?;
    }
    Ok(())
}
