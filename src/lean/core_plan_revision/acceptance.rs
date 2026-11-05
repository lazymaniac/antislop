use super::*;

/// Revisions belong to an activated plan in the same conversation.
pub(super) fn ensure_accepted_plan(core: &Core, conversation: &str, plan: &str) -> Result<()> {
    let accepted: bool = core.store.db().query_row(
        "SELECT EXISTS(SELECT 1 FROM plan_activations WHERE plan_id=?1 AND conversation_id=?2)",
        rusqlite::params![plan, conversation],
        |row| row.get(0),
    )?;
    if accepted {
        Ok(())
    } else {
        Err(unaccepted_plan_error(core, conversation, plan)?)
    }
}

fn unaccepted_plan_error(core: &Core, conversation: &str, plan: &str) -> Result<Error> {
    if core
        .store
        .latest_plan(conversation)?
        .first()
        .is_some_and(|task| task.plan_id == plan)
    {
        return Ok(Error::Invalid(format!(
            "plan {plan} is not accepted; correct the draft with submit_plan, not revise_plan; revise_plan only edits an accepted plan"
        )));
    }
    let current = core
        .store
        .latest_plan(conversation)?
        .first()
        .map(|task| task.plan_id.clone())
        .unwrap_or_default();
    Ok(Error::Invalid(format!(
        "plan {plan} is not the accepted plan (it was replaced or never accepted); revise the current plan {current} using its full ID"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_activation_is_required_and_bound_to_its_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        let foreign = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        let tasks = core.store.add_plan(&conversation, &[json!({
            "id":"inspect", "title":"Inspect", "goal":"Inspect the project", "change_kind": "none", "kind":"analysis"
        })]).unwrap();
        let plan = &tasks[0].plan_id;
        assert!(
            ensure_accepted_plan(&core, &conversation, plan)
                .unwrap_err()
                .to_string()
                .contains("not accepted")
        );
        core.store
            .db()
            .execute(
                "INSERT INTO plan_activations(plan_id,conversation_id) VALUES(?1,?2)",
                rusqlite::params![plan, conversation],
            )
            .unwrap();
        assert!(ensure_accepted_plan(&core, &conversation, plan).is_ok());
        assert!(ensure_accepted_plan(&core, &foreign, plan).is_err());
    }
}
