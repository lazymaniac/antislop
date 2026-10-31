use super::*;
use std::path::Path;

pub(super) struct Authorization {
    pub(super) read: Vec<String>,
    pub(super) write: Vec<String>,
}

/// Paths a revision needs beyond what the user approved for the plan.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Expansion {
    pub(super) read: Vec<String>,
    pub(super) write: Vec<String>,
}

impl Expansion {
    pub(super) fn is_empty(&self) -> bool {
        self.read.is_empty() && self.write.is_empty()
    }
}

pub(super) fn plan_authorization(
    core: &Core,
    conversation: &str,
    plan: &str,
) -> Result<Authorization> {
    use rusqlite::OptionalExtension;
    let authorization = core.store.db().query_row(
        "SELECT scope,write_scope FROM plan_authorizations WHERE plan_id=?1 AND conversation_id=?2",
        rusqlite::params![plan, conversation],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    ).optional()?;
    let Some((read, write)) = authorization else {
        return Err(unaccepted_plan_error(core, conversation, plan)?);
    };
    Ok(Authorization {
        read: serde_json::from_str(&read)?,
        write: serde_json::from_str(&write)?,
    })
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

/// Refuses any scope beyond the approved one; see [`scope_expansion`].
#[cfg(test)]
pub(super) fn validate_authorized_scope(
    specs: &[TaskContract],
    allowed: &[String],
    write_allowed: &[String],
) -> Result<()> {
    let expansion = scope_expansion(specs, allowed, write_allowed)?;
    require(
        expansion.is_empty(),
        &expansion_text(&expansion, write_allowed),
    )
}

/// What the revision needs beyond the approved scope. A task without a scope
/// or with a path outside the project is refused outright: the user is asked
/// only about named paths inside it.
pub(super) fn scope_expansion(
    specs: &[TaskContract],
    allowed: &[String],
    write_allowed: &[String],
) -> Result<Expansion> {
    validate_nonempty_scopes(specs, allowed, write_allowed)?;
    let mut expansion = Expansion::default();
    for spec in specs.iter().filter(|spec| spec.writes) {
        require(
            spec.scope.iter().all(|path| project_relative(path)),
            "revision write scope must stay inside the project",
        )?;
        let missing = spec
            .scope
            .iter()
            .filter(|path| !write_allowed.iter().any(|approved| covers(approved, path)));
        push_new(&mut expansion.write, missing);
    }
    for path in specs.iter().flat_map(|spec| &spec.scope) {
        require(
            allowed.contains(path) || project_relative(path),
            "revision scope contains parent traversal",
        )?;
    }
    let missing = specs
        .iter()
        .flat_map(|spec| &spec.scope)
        .filter(|path| !allowed.iter().any(|approved| covers(approved, path)));
    push_new(&mut expansion.read, missing);
    Ok(expansion)
}

fn push_new<'a>(into: &mut Vec<String>, paths: impl Iterator<Item = &'a String>) {
    for path in paths {
        if !into.contains(path) {
            into.push(path.clone());
        }
    }
}

fn validate_nonempty_scopes(
    specs: &[TaskContract],
    allowed: &[String],
    write_allowed: &[String],
) -> Result<()> {
    let whole = |approved: &[String]| approved.iter().any(|path| path == ".");
    for spec in specs.iter().filter(|spec| spec.scope.is_empty()) {
        require(
            whole(allowed) && (!spec.writes || whole(write_allowed)),
            &format!(
                "task {} has no scope, which means the whole project; name the files or globs it works on{}",
                spec.id,
                approved_hint(write_allowed)
            ),
        )?;
    }
    Ok(())
}

/// The refusal names what is missing and what is approved, so a revision
/// can be narrowed without asking.
pub(super) fn expansion_text(expansion: &Expansion, write_allowed: &[String]) -> String {
    let mut parts = Vec::new();
    if !expansion.write.is_empty() {
        parts.push(format!("write {}", expansion.write.join(", ")));
    }
    if !expansion.read.is_empty() {
        parts.push(format!("read {}", expansion.read.join(", ")));
    }
    tasks::bound(
        &format!(
            "revision needs paths outside the approved scope ({}); user approval is required, or keep each task's scope inside the approved paths{}",
            parts.join("; "),
            approved_hint(write_allowed)
        ),
        2000,
    )
}

fn approved_hint(write_allowed: &[String]) -> String {
    if write_allowed.is_empty() {
        return "; this plan approves no writes".to_owned();
    }
    format!(
        " (approved for writing: {})",
        tasks::bound(&write_allowed.join(", "), 800)
    )
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
