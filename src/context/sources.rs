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
pub(super) fn extend(
    run: &crate::domain::Run,
    prompt: &str,
    sources: &mut BTreeMap<String, String>,
) -> Result<()> {
    crate::runtime::orchestration_recovery::handoff::extend(run, prompt, sources)?;
    if let Some(round) = run
        .orchestration_rounds
        .last()
        .filter(|r| r.status == crate::orchestration::planning::RoundStatus::Running)
    {
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
        }
    }
    let mut directories = Vec::new();
    for (selector, body) in &run.input_snapshot {
        if root_selector(&run.spec.workspace, selector) {
            directories.push((format!("file:{selector}"), body.clone()));
        }
    }
    if run.intake
        && let Some(round) = run.orchestration_rounds.last()
    {
        round.validate_interview_sources()?;
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
            if root_selector(&run.spec.workspace, selector) {
                directories.push((reference, body.clone()));
            }
        }
    }
    for (source_ref, body) in directories {
        let source_hash = hash(&body);
        let mut members = vec![];
        for (path, entry) in crate::workspace::inputs::context_members(&body)? {
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
