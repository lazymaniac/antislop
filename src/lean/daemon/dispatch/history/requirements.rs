use super::super::*;

pub(super) fn task_requirement_texts(core: &Core, plan: &str, record: &Value) -> Result<Value> {
    let ids = |field: &str| -> Vec<&str> {
        record["spec"][field]
            .as_array()
            .map(|ids| ids.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    };
    let (requirement_ids, criterion_ids) = (ids("requirement_ids"), ids("criterion_ids"));
    let Some(run) = core.store.plan_run(plan)? else {
        return Ok(json!([]));
    };
    let conversation = core.store.plan_conversation(plan)?;
    let entries = core.store.ledger_entries(&conversation, &run, None)?;
    Ok(entries
        .iter()
        .filter(|entry| requirement_ids.contains(&entry.spec.id.as_str()))
        .map(|entry| {
            let criteria = entry
                .spec
                .criteria
                .iter()
                .filter(|criterion| criterion_ids.contains(&criterion.id.as_str()))
                .map(|criterion| json!({"text": criterion.text, "mandatory": criterion.mandatory}))
                .collect::<Vec<_>>();
            json!({"text": entry.spec.interpretation, "criteria": criteria})
        })
        .collect())
}
