use super::*;

pub(super) fn route(context: &Context<'_>, request: Request) -> Result<Route> {
    route_pipeline(
        context,
        request,
        &[
            list_runs,
            reject_proposal,
            get_proposal_and_requirements,
            get_entry,
            get_diff,
            get_evidence_and_propose,
            apply_change_and_create_run,
            add_source,
            get_source,
        ],
    )
}

fn list_runs(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::ListRuns { conversation } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            json!(core.store.list_runs(&conversation)?)
        }),
        request => Route::Unmatched(request),
    })
}

fn reject_proposal(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::RejectLedgerProposal {
            conversation,
            run_id,
            proposal_id,
            diff_hash,
            comment,
        } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            core.store.reject_ledger_proposal(
                &conversation,
                &run_id,
                &proposal_id,
                &diff_hash,
                &comment,
            )?;
            if core
                .store
                .has_pending_requirements_approval(&conversation)?
            {
                core.wake(&conversation);
            }
            json!({"rejected":true})
        }),
        request => Route::Unmatched(request),
    })
}

fn get_proposal_and_requirements(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::GetLedgerProposal {
            conversation,
            run_id,
        } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            core.store.latest_ledger_proposal(&conversation, &run_id)?
        }),
        Request::ListRequirements { query } => Route::Handled({
            validate_project_conversation(core, project, &query.conversation)?;
            json!(core.ledger_query(query)?)
        }),
        request => Route::Unmatched(request),
    })
}

fn get_entry(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::GetLedgerEntry {
            conversation,
            run_id,
            id,
            revision,
        } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            let fp = core.ledger_fingerprint(&conversation).unwrap_or_default();
            let entries = core
                .store
                .ledger_statuses(&conversation, &run_id, revision, &fp)?;
            let entry = entries
                .into_iter()
                .find(|e| e.spec.id == id)
                .ok_or_else(|| Error::Invalid("unknown ledger entry".into()))?;
            json!({"entry":entry,"history":core.store.ledger_entry_history(&conversation, &run_id, &id)?,"evidence":core.store.ledger_evidence(&conversation,&run_id,Some(&id))?,"tasks":core.store.ledger_entry_tasks(&conversation,&run_id,&id)?})
        }),
        request => Route::Unmatched(request),
    })
}

fn get_diff(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::GetLedgerDiff {
            conversation,
            run_id,
            from_revision,
            to_revision,
        } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            json!(
                core.store
                    .ledger_diff(&conversation, &run_id, from_revision, to_revision)?
            )
        }),
        request => Route::Unmatched(request),
    })
}

fn get_evidence_and_propose(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::GetEvidence {
            conversation,
            run_id,
            entry_id,
        } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            json!(
                core.store
                    .ledger_evidence(&conversation, &run_id, entry_id.as_deref())?
            )
        }),
        Request::ProposeLedgerChange { change } => Route::Handled({
            validate_project_conversation(core, project, &change.conversation)?;
            let (proposal_id, diff) = core.store.propose_ledger_change(&change, &core.state)?;
            let diff_hash = super::super::super::ledger::diff_hash(&diff)?;
            json!({"proposal_id":proposal_id,"diff_hash":diff_hash,"diff":diff})
        }),
        request => Route::Unmatched(request),
    })
}

fn apply_change_and_create_run(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::ApplyLedgerChange {
            proposal_id,
            diff_hash,
            change,
        } => Route::Handled({
            validate_project_conversation(core, project, &change.conversation)?;
            json!(core.apply_user_ledger_change(&proposal_id, &diff_hash, &change)?)
        }),
        Request::CreateRun { conversation, goal } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            json!(core.store.create_run(&conversation, &goal)?)
        }),
        request => Route::Unmatched(request),
    })
}

fn add_source(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::AddLedgerSource {
            conversation,
            run_id,
            text,
        } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            json!(core.store.register_document_source(
                &conversation,
                &run_id,
                &text,
                &core.state
            )?)
        }),
        request => Route::Unmatched(request),
    })
}

fn get_source(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::GetLedgerSource {
            conversation,
            run_id,
            source_id,
        } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            json!({"text":core.store.ledger_source_text(&conversation,&run_id,&source_id,&core.state)?})
        }),
        request => Route::Unmatched(request),
    })
}
