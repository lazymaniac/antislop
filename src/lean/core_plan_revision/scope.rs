use super::*;
use std::path::Path;

pub(super) fn validate_plan_authorization(
    core: &Core,
    conversation: &str,
    plan: &str,
    specs: &[TaskContract],
) -> Result<()> {
    use rusqlite::OptionalExtension;
    let authorization = core.store.db().query_row(
        "SELECT scope,write_scope FROM plan_authorizations WHERE plan_id=?1 AND conversation_id=?2",
        rusqlite::params![plan, conversation],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    ).optional()?;
    let Some((allowed, write_allowed)) = authorization else {
        return Err(unaccepted_plan_error(core, conversation, plan)?);
    };
    let allowed: Vec<String> = serde_json::from_str(&allowed)?;
    let write_allowed: Vec<String> = serde_json::from_str(&write_allowed)?;
    validate_authorized_scope(specs, &allowed, &write_allowed)
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

pub(super) fn validate_authorized_scope(
    specs: &[TaskContract],
    allowed: &[String],
    write_allowed: &[String],
) -> Result<()> {
    validate_nonempty_scopes(specs, allowed)?;
    validate_write_scopes(specs, write_allowed)?;
    validate_read_scopes(specs, allowed)?;
    Ok(())
}

fn validate_nonempty_scopes(specs: &[TaskContract], allowed: &[String]) -> Result<()> {
    require(
        allowed.iter().any(|path| path == ".") || specs.iter().all(|spec| !spec.scope.is_empty()),
        "empty scope expands the approved scope",
    )
}

fn validate_write_scopes(specs: &[TaskContract], write_allowed: &[String]) -> Result<()> {
    for spec in specs.iter().filter(|spec| spec.writes) {
        require(
            !write_allowed.is_empty(),
            "revision adds write permission; user approval is required",
        )?;
        let scope = effective_scope(spec);
        require(
            scope.iter().all(|path| project_relative(path)),
            "revision write scope must stay inside the project",
        )?;
        require(
            scope
                .iter()
                .all(|path| write_allowed.iter().any(|approved| covers(approved, path))),
            "revision expands approved write scope; user approval is required",
        )?;
    }
    Ok(())
}

fn validate_read_scopes(specs: &[TaskContract], allowed: &[String]) -> Result<()> {
    for path in specs.iter().flat_map(|spec| &spec.scope) {
        require(
            allowed.contains(path) || project_relative(path),
            "revision scope contains parent traversal",
        )?;
        require(
            allowed.iter().any(|approved| covers(approved, path)),
            "revision expands the approved scope; user approval is required",
        )?;
    }
    Ok(())
}

fn effective_scope(spec: &TaskContract) -> Vec<String> {
    if spec.scope.is_empty() {
        vec![".".to_owned()]
    } else {
        spec.scope.clone()
    }
}

fn project_relative(path: &str) -> bool {
    Path::new(path).is_relative()
        && !Path::new(path)
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
}

/// Checks approved path and glob entries against proposed task scope entries.
pub(super) fn covers(approved: &str, path: &str) -> bool {
    if approved == path || (approved == "." && Path::new(path).is_relative()) {
        return true;
    }
    covers_names(&path_names(approved), &path_names(path))
}

fn path_names(path: &str) -> Vec<String> {
    Path::new(path)
        .components()
        .filter(|component| !matches!(component, std::path::Component::CurDir))
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect()
}

fn covers_names(approved: &[String], path: &[String]) -> bool {
    match approved.split_first() {
        None => true,
        Some((first, rest)) if first == "**" => {
            (0..=path.len()).any(|skip| covers_names(rest, &path[skip..]))
        }
        Some((first, rest)) => path.split_first().is_some_and(|(name, below)| {
            name_matches(first.as_bytes(), name.as_bytes()) && covers_names(rest, below)
        }),
    }
}

fn name_matches(pattern: &[u8], name: &[u8]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some((b'*', rest)) => (0..=name.len()).any(|skip| name_matches(rest, &name[skip..])),
        Some((b'?', rest)) => !name.is_empty() && name_matches(rest, &name[1..]),
        Some((byte, rest)) => name.first() == Some(byte) && name_matches(rest, &name[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(writes: bool, scope: &[&str]) -> TaskContract {
        serde_json::from_value(serde_json::json!({
            "id": "task", "title": "Task", "goal": "Check scope",
            "kind": "implementation", "writes": writes, "scope": scope,
            "verifier": {"focus": "Check scope"}
        }))
        .unwrap()
    }

    #[test]
    fn authorized_scopes_allow_nested_globs_and_reject_traversal_or_expansion() {
        let spec = task(true, &["src/lean/*.rs"]);
        assert!(validate_authorized_scope(&[spec], &["src".into()], &["src/lean".into()]).is_ok());

        let traversal = task(true, &["src/../secrets"]);
        assert!(validate_authorized_scope(&[traversal], &[".".into()], &[".".into()]).is_err());

        let expanded = task(true, &["tests"]);
        assert!(validate_authorized_scope(&[expanded], &[".".into()], &["src".into()]).is_err());
    }

    #[test]
    fn empty_scope_requires_project_wide_authorization_and_write_scope() {
        let readonly = task(false, &[]);
        assert!(
            validate_authorized_scope(std::slice::from_ref(&readonly), &["src".into()], &[])
                .is_err()
        );
        assert!(validate_authorized_scope(&[readonly], &[".".into()], &[]).is_ok());

        let writer = task(true, &[]);
        assert!(validate_authorized_scope(&[writer], &[".".into()], &[]).is_err());
    }
}
