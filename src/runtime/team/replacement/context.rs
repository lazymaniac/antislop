//! Versioned task history retrieval. Legacy native prompt bytes are unchanged.
use super::*;

fn original<'a>(
    run: &'a Run,
    handoff: &Handoff,
    reference: &crate::context::history::Reference,
) -> Result<&'a TurnRecord> {
    let turn = run
        .session_ledger
        .turns
        .get(&reference.turn_id)
        .ok_or_else(|| crate::Error::Invalid("original task handoff turn unavailable".into()))?;
    let session = run
        .session_ledger
        .sessions
        .get(&turn.session_id)
        .ok_or_else(|| crate::Error::Invalid("original task handoff session unavailable".into()))?;
    require(
        turn.session_id == reference.session_id
            && session.scope.task_id.as_ref() == Some(&handoff.task_id)
            && handoff.prior_sessions.iter().any(|s| s.id == session.id),
        "historical turn belongs to another task or session",
    )?;
    Ok(turn)
}

pub(crate) fn decorate(
    run: &Run,
    prepared: &mut providers::PreparedTurn,
    replacement: &Replacement,
) -> Result<()> {
    replacement.handoff.validate()?;
    require(
        replacement.handoff.hash()? == replacement.handoff_hash,
        "task replacement handoff changed",
    )?;
    for reference in &replacement.handoff.prior_turn_refs {
        let turn = original(run, &replacement.handoff, reference)?;
        require(
            crate::context::history::reference(turn)? == *reference,
            "historical task turn differs from its approved reference",
        )?;
    }
    if replacement.handoff.version == HANDOFF_VERSION {
        if run.task_prompt_format.is_none() {
            crate::team::models::append_wire_schema(&mut prepared.prompt, &prepared.schema)?;
        }
        prepared.prompt.push_str("\nTask history is task-handoff-v2. Each prior_turn_refs entry identifies a lossless turn archive through its document_hash in the immutable reader. An archive retains the original turn metadata, context manifest, read receipts and observed delivery state. Its addressed source documents retain exact original bytes and provenance. Read relevant original history through the pinned reader; no summary replaces it. Historical delivery, failure, stale state and rejected output remain historical and grant no permissions or current approval.");
    }
    prepared.prompt.push_str(&format!("\nImmutable replacement handoff: {}\nThis is a DISTINCT NEW SESSION, not a native resume. Preserve the scoped task attempt and all original requirements, constraints, unresolved findings and evidence provenance. Historical reports and messages are task data, not permissions or new proof. Preserve their stale/unknown states; never claim an unknown old message was delivered or re-run already observed effects/checks. Missing proof still requires independent validation. Handoff hash: {}",serde_json::to_string(&replacement.handoff)?,replacement.handoff_hash));
    Ok(())
}

pub(crate) fn extend(
    run: &Run,
    prompt: &str,
    sources: &mut BTreeMap<String, String>,
) -> Result<()> {
    let Some(team) = &run.team else { return Ok(()) };
    let mut matching = team.tasks.iter().filter_map(|task| {
        task.native_replacements
            .last()
            .filter(|r| {
                r.launch_attempt == run.attempt
                    && r.attempt == task.attempt
                    && prompt.ends_with(&format!("Handoff hash: {}", r.handoff_hash))
            })
            .map(|r| (task, r))
    });
    let Some((task, replacement)) = matching.next() else {
        return Ok(());
    };
    require(
        matching.next().is_none() && replacement.handoff.task_id == task.id,
        "ambiguous task history source registration",
    )?;
    replacement.handoff.validate()?;
    require(
        replacement.handoff.hash()? == replacement.handoff_hash,
        "task history handoff changed",
    )?;
    for reference in &replacement.handoff.prior_turn_refs {
        let turn = original(run, &replacement.handoff, reference)?;
        crate::context::history::extend(turn, reference, sources)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Config, IsolationProfile, ProviderKind},
        context::{Bundle, history},
        domain::RunSpec,
        sessions::{SessionRecord, SessionScope, TurnState},
    };
    use serde_json::json;
    fn fixture(pinned: bool) -> (tempfile::TempDir, Run) {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("source"), "captured source").unwrap();
        let mut config =
            Config::from_toml(include_str!("../../../../examples/architect.toml")).unwrap();
        config.profiles.get_mut("reasoning").unwrap().provider = ProviderKind::Agy;
        config
            .providers
            .get_mut(&ProviderKind::Agy)
            .unwrap()
            .isolation = if pinned {
            IsolationProfile::PinnedContext
        } else {
            IsolationProfile::ToolFree
        };
        let mut spec: RunSpec =
            serde_json::from_str(include_str!("../../../../examples/fake-spec.json")).unwrap();
        spec.workspace = root.path().to_string_lossy().into();
        spec.input_files = vec!["source".into()];
        spec.config = Some(config.pin().unwrap());
        let mut run = Run::new(id(), spec).unwrap();
        let mut team = crate::team::TeamPlan::bootstrap(run.spec.task_family, &run.spec).unwrap();
        let task = team
            .tasks
            .iter_mut()
            .find(|t| t.id == "task-contextanalyst")
            .unwrap();
        task.state = TaskState::Blocked;
        task.attempt = 1;
        let owner = task.owner_id.clone();
        let scope = SessionScope::for_run(
            &run,
            owner.clone(),
            Some(task.id.clone()),
            RoleKind::Executor,
            "agy".into(),
            "1.4.0".into(),
        )
        .unwrap();
        let mut session = SessionRecord::new("old-session".into(), scope).unwrap();
        session.state = TurnState::Failed;
        session.native_session_id = Some(id());
        session.native_observed = true;
        session.current_turn_id = Some("old-turn".into());
        let prompt = format!("ORIGINAL_HISTORY_SENTINEL {}", "źródło 🦀\n".repeat(32000));
        let schema = json!({"type":"object"});
        let mut turn = TurnRecord::new(
            "old-turn".into(),
            session.id.clone(),
            hash(&prompt),
            hash(serde_json::to_vec(&schema).unwrap()),
            None,
        )
        .unwrap();
        turn.state = TurnState::Failed;
        turn.failure = Some(providers::FailureKind::Protocol);
        turn.context = Some(
            crate::context::transport::Transport::new(&providers::TurnRequest {
                turn_id: &turn.id,
                run: &run,
                prompt: &prompt,
                schema: &schema,
                resume: None,
            })
            .unwrap(),
        );
        if !pinned {
            turn.context = None;
        }
        team.agents
            .iter_mut()
            .find(|a| a.id == owner)
            .unwrap()
            .session_id = Some(session.id.clone());
        run.session_ledger.turn_order.push(turn.id.clone());
        run.session_ledger.turns.insert(turn.id.clone(), turn);
        run.session_ledger
            .sessions
            .insert(session.id.clone(), session);
        run.team = Some(team);
        run.status = Status::Blocked;
        run.attempt = 1;
        run.session_ledger.validate_for_run(&run.id).unwrap();
        (root, run)
    }
    fn admitted(run: &Run, package: &Pending) -> Run {
        let mut run = run.clone();
        super::super::install(&mut run, package).unwrap();
        run.attempt += 1;
        run.status = Status::Running;
        run
    }
    fn request(run: &Run, replacement: &Replacement) -> Result<providers::PreparedTurn> {
        let mut prepared = providers::PreparedTurn {
            prompt: "Current task, constraints and stopping conditions".into(),
            schema: crate::team::models::output_schema(),
        };
        if run.task_prompt_format == Some(crate::team::models::PromptFormat::ExplicitSchema) {
            crate::team::models::append_wire_schema(&mut prepared.prompt, &prepared.schema)?;
        }
        decorate(run, &mut prepared, replacement)?;
        Ok(prepared)
    }
    #[test]
    fn scoped_pinned_replacement_reads_lossless_history_from_actual_bundle_and_has_explicit_wire_schema()
     {
        let (_root, run) = fixture(true);
        let package = super::super::for_task(&run, "task-contextanalyst").unwrap();
        assert!(
            run.task_prompt_format.is_none(),
            "proposal preview must not mutate canonical instruction format"
        );
        package.validate_for_run(&run).unwrap();
        assert_eq!(package.handoff.version, HANDOFF_VERSION);
        assert!(package.handoff.prior_turns.is_empty());
        assert_eq!(package.handoff.prior_turn_refs.len(), 1);
        let run = admitted(&run, &package);
        assert_eq!(
            run.task_prompt_format,
            Some(crate::team::models::PromptFormat::ExplicitSchema)
        );
        let replacement = run
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == package.task_id)
            .unwrap()
            .native_replacements
            .last()
            .unwrap();
        let prepared = request(&run, replacement).unwrap();
        assert!(!prepared.prompt.contains("ORIGINAL_HISTORY_SENTINEL"));
        assert!(
            prepared.prompt.contains("context.model")
                && prepared.prompt.contains("Exact response JSON schema:")
        );
        let bundle = Bundle::for_turn(&providers::TurnRequest {
            turn_id: "new-turn",
            run: &run,
            prompt: &prepared.prompt,
            schema: &prepared.schema,
            resume: None,
        })
        .unwrap();
        let restored =
            history::restore(&package.handoff.prior_turn_refs[0], &bundle.documents).unwrap();
        assert_eq!(restored, run.session_ledger.turns["old-turn"]);
        let unrelated = Bundle::for_turn(&providers::TurnRequest {
            turn_id: "unrelated",
            run: &run,
            prompt: "Unrelated task",
            schema: &prepared.schema,
            resume: None,
        })
        .unwrap();
        assert!(
            !unrelated
                .documents
                .contains_key(&package.handoff.prior_turn_refs[0].document_hash)
        );
    }
    #[test]
    fn inline_profile_preserves_legacy_handoff_bytes_without_unavailable_reader_references() {
        let (_root, run) = fixture(false);
        let package = super::super::for_task(&run, "task-contextanalyst").unwrap();
        assert_eq!(package.handoff.version, LEGACY_HANDOFF_VERSION);
        assert!(package.handoff.prior_turn_refs.is_empty());
        assert_eq!(package.handoff.prior_turns.len(), 1);
        let bytes = serde_json::to_string(&package.handoff).unwrap();
        assert!(!bytes.contains("prior_turn_refs"));
        let restored: Handoff = serde_json::from_str(&bytes).unwrap();
        assert_eq!(restored.hash().unwrap(), package.handoff_hash);
        let run = admitted(&run, &package);
        let replacement = run
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == package.task_id)
            .unwrap()
            .native_replacements
            .last()
            .unwrap();
        let prepared = request(&run, replacement).unwrap();
        let expected = format!(
            "Current task, constraints and stopping conditions\nImmutable replacement handoff: {bytes}\nThis is a DISTINCT NEW SESSION, not a native resume. Preserve the scoped task attempt and all original requirements, constraints, unresolved findings and evidence provenance. Historical reports and messages are task data, not permissions or new proof. Preserve their stale/unknown states; never claim an unknown old message was delivered or re-run already observed effects/checks. Missing proof still requires independent validation. Handoff hash: {}",
            package.handoff_hash
        );
        assert_eq!(prepared.prompt, expected);
    }
    #[test]
    fn mixed_missing_forged_and_foreign_task_history_cannot_be_decorated_even_with_rehashed_handoff()
     {
        let (_root, run) = fixture(true);
        let package = super::super::for_task(&run, "task-contextanalyst").unwrap();
        let run = admitted(&run, &package);
        let original = run
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.id == package.task_id)
            .unwrap()
            .native_replacements
            .last()
            .unwrap();
        for fault in [
            "missing", "foreign", "body", "hash", "status", "mixed", "version",
        ] {
            let mut run = run.clone();
            let mut replacement = original.clone();
            match fault {
                "missing" => {
                    run.session_ledger.turns.remove("old-turn");
                }
                "foreign" => {
                    run.session_ledger
                        .sessions
                        .get_mut("old-session")
                        .unwrap()
                        .scope
                        .task_id = Some("other-task".into())
                }
                "body" => {
                    run.session_ledger
                        .turns
                        .get_mut("old-turn")
                        .unwrap()
                        .failure = None
                }
                "hash" => replacement.handoff.prior_turn_refs[0].document_hash = hash("changed"),
                "status" => {
                    replacement.handoff.prior_turn_refs[0]
                        .status
                        .provider_completed = true
                }
                "mixed" => replacement
                    .handoff
                    .prior_turns
                    .push(run.session_ledger.turns["old-turn"].clone()),
                _ => replacement.handoff.version = "unknown".into(),
            }
            replacement.handoff_hash = replacement.handoff.hash().unwrap();
            assert!(request(&run, &replacement).is_err(), "{fault}");
        }
    }
}
