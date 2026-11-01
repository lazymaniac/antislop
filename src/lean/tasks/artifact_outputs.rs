use super::*;

/// Explicit documents can live in ignored directories, including runtime state.
/// Hidden names are valid; traversal, repository metadata and runtime internals are not.
pub(crate) fn validate_artifact_output(output: &str) -> Result<()> {
    let path = std::path::Path::new(output);
    let parts: Vec<_> = output.split('/').collect();
    crate::require(
        !output.is_empty()
            && path
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
            && parts
                .iter()
                .all(|part| !part.is_empty() && !matches!(*part, "." | ".."))
            && !output.contains(['*', '?', '['])
            && !output.chars().any(char::is_control),
        &format!("outputs must be explicit relative file paths: {output}"),
    )?;
    crate::require(
        !parts.iter().any(|part| {
            [".git", ".hg", ".svn"]
                .iter()
                .any(|name| part.eq_ignore_ascii_case(name))
        }) && !(parts
            .first()
            .is_some_and(|part| part.eq_ignore_ascii_case(".antislop"))
            && parts.get(1).is_some_and(|part| {
                [
                    "worktrees",
                    "sessions",
                    "content",
                    "sources",
                    "locks",
                    "core.db",
                    "core.db-wal",
                    "core.db-shm",
                    "core.lock",
                    "core.sock",
                    "daemon.log",
                    "stopped",
                ]
                .iter()
                .any(|name| part.eq_ignore_ascii_case(name))
            })),
        &format!(
            "artifact output cannot replace repository metadata or runtime internals: {output}"
        ),
    )?;
    crate::require(
        (matches!(
            path.extension().and_then(|s| s.to_str()),
            Some("md" | "txt" | "rst" | "pdf" | "csv" | "html" | "log")
        ) || json_report_path(path, &parts))
            && !parts
                .iter()
                .any(|part| matches!(*part, "src" | "tests" | "test" | "config")),
        &format!(
            "artifact output must be a report/document, not code, tests or configuration: {output}"
        ),
    )
}

/// JSON is also a report format, but project manifests/settings stay code changes.
fn json_report_path(path: &std::path::Path, parts: &[&str]) -> bool {
    if path.extension().and_then(|s| s.to_str()) != Some("json") {
        return false;
    }
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(
        name.as_str(),
        "package" | "package-lock" | "composer" | "composer-lock" | "deno" | "config" | "settings"
    ) || ["tsconfig", "jsconfig", ".eslintrc", ".prettierrc"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
        || name.ends_with(".config")
        || name.ends_with(".settings")
    {
        return false;
    }
    parts
        .iter()
        .any(|part| matches!(*part, "results" | "reports" | "artifacts"))
        || name.split(['-', '_', '.']).any(|part| {
            matches!(
                part,
                "report" | "summary" | "analysis" | "findings" | "metrics" | "validation"
            )
        })
}
