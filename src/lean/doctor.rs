//! Whether the configured CLIs can be launched, checked before a team starts
//! and on demand with `antislop doctor`; and `antislop init`, which writes a
//! first configuration from the CLIs that can.

use crate::Result;
use crate::config::{Config, ProviderKind, RoleKind, WorkKind};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Where `binary` resolves: itself when it names a path, else the first
/// executable of that name on `PATH`.
pub(crate) fn find_executable(binary: &Path) -> Option<PathBuf> {
    let executable = |path: &Path| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    };
    if binary.components().count() > 1 {
        return executable(binary).then(|| binary.to_owned());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(binary))
        .find(|path| executable(path))
}

/// The command a profile launches, or why it cannot be launched.
fn profile_command(config: &Config, name: &str) -> std::result::Result<PathBuf, String> {
    let profile = config
        .profiles
        .get(name)
        .ok_or_else(|| format!("profile `{name}` is not configured"))?;
    let provider = config
        .providers
        .get(&profile.provider)
        .ok_or_else(|| format!("provider of profile `{name}` is not configured"))?;
    let (binary, _) =
        crate::config::resolve_command(provider, profile).map_err(|error| error.to_string())?;
    let resolved = find_executable(&binary).ok_or_else(|| {
        format!(
            "profile `{name}` launches `{}`, which is not an executable file or on PATH",
            binary.display()
        )
    })?;
    if profile.provider == ProviderKind::Agy {
        super::launch::agy_ready().map_err(|error| format!("profile `{name}`: {error}"))?;
    }
    Ok(resolved)
}

/// Why the profiles routed for `kinds` cannot be launched; empty when every
/// one of them can. Each problem names the profiles that would launch.
pub(crate) fn launch_problems(
    config: &Config,
    kinds: impl IntoIterator<Item = WorkKind>,
) -> Vec<String> {
    let mut names = BTreeSet::new();
    for kind in kinds {
        for role in [RoleKind::Executor, RoleKind::Reviewer] {
            if let Ok(resolved) = config.resolve(kind, role) {
                names.insert(resolved.profile);
            }
        }
    }
    let problems = names
        .iter()
        .filter_map(|name| profile_command(config, name).err())
        .collect::<Vec<_>>();
    if problems.is_empty() {
        return problems;
    }
    let working = config
        .profiles
        .keys()
        .filter(|name| profile_command(config, name).is_ok())
        .cloned()
        .collect::<Vec<_>>();
    let hint = if working.is_empty() {
        "no configured profile can be launched; install a CLI or fix `binary`/`command`".to_owned()
    } else {
        format!(
            "profiles that can be launched: {}; route the work to one of them in the configuration",
            working.join(", ")
        )
    };
    problems
        .into_iter()
        .map(|problem| format!("{problem} ({hint})"))
        .collect()
}

/// The first line `binary --version` prints, waiting at most a few seconds.
fn version(binary: &Path) -> String {
    let child = std::process::Command::new(binary)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => return format!("cannot run: {error}"),
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return "--version did not answer within 15 s".into();
            }
        }
    }
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(error) => return format!("cannot read --version: {error}"),
    };
    let text = String::from_utf8_lossy(if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    })
    .lines()
    .next()
    .unwrap_or_default()
    .trim()
    .to_owned();
    if text.is_empty() {
        format!("--version printed nothing ({})", output.status)
    } else {
        text
    }
}

/// A report on the configuration, state and every configured profile.
/// Returns whether every profile can be launched.
pub fn report(
    project: &Path,
    config_path: &Path,
    state: &Path,
    out: &mut dyn std::io::Write,
) -> Result<bool> {
    let executable = std::env::current_exe()?;
    writeln!(
        out,
        "antislop {} (build {})",
        env!("CARGO_PKG_VERSION"),
        super::daemon::build_identity(&executable)
            .map(|identity| identity.chars().take(12).collect::<String>())
            .unwrap_or_else(|error| format!("unknown: {error}"))
    )?;
    writeln!(out, "project: {}", project.display())?;
    writeln!(out, "config:  {}", config_path.display())?;
    writeln!(out, "state:   {}", state.display())?;
    let config = match Config::load(config_path) {
        Ok(config) => config,
        Err(error) => {
            writeln!(out, "✗ {error}")?;
            return Ok(false);
        }
    };
    writeln!(out, "profiles:")?;
    let mut healthy = true;
    let mut versions = std::collections::HashMap::new();
    for name in config.profiles.keys() {
        let provider = super::agent::name(config.profiles[name].provider);
        match profile_command(&config, name) {
            Ok(binary) => writeln!(
                out,
                "  ✓ {name} ({provider}) {} · {}",
                binary.display(),
                versions
                    .entry(binary.clone())
                    .or_insert_with(|| version(&binary))
            )?,
            Err(problem) => {
                healthy = false;
                writeln!(out, "  ✗ {name} ({provider}): {problem}")?;
            }
        }
    }
    writeln!(out, "orchestrator: {}", config.orchestrator.profile)?;
    Ok(healthy)
}

/// The CLIs `init` looks for, in the order it prefers them as workers.
const KNOWN: [(&str, ProviderKind); 5] = [
    ("codex", ProviderKind::Codex),
    ("claude", ProviderKind::Claude),
    ("gemini", ProviderKind::Gemini),
    ("agy", ProviderKind::Agy),
    ("opencode", ProviderKind::Opencode),
];

/// A configuration that sends work to `worker` and reviews to `reviewer`,
/// built from the documented example so routing and policy stay current.
fn initial_config(worker: &str, reviewer: &str) -> Result<String> {
    let template = include_str!("../../examples/antislop.toml");
    let providers_start = template
        .find("[providers.codex]")
        .ok_or_else(|| crate::Error::Invalid("example configuration has no providers".into()))?;
    let providers_end = template
        .find("# Profiles select")
        .ok_or_else(|| crate::Error::Invalid("example configuration has no profiles".into()))?;
    let mut providers = String::new();
    for name in [worker, reviewer] {
        let table = format!("[providers.{name}]\nbinary = \"{name}\"\n\n");
        if !providers.contains(&table) {
            providers.push_str(&table);
        }
    }
    let mut text = format!(
        "# Written by `antislop init` from the CLIs found on PATH: {worker} works,\n\
         # {reviewer} reviews. See examples/antislop.toml in the antislop sources\n\
         # for every option; `antislop doctor` checks this file.\n"
    );
    let body = format!(
        "{}{providers}{}",
        &template[..providers_start],
        &template[providers_end..]
    );
    let mut profile = "";
    for line in body.lines().skip_while(|line| line.starts_with('#')) {
        if let Some(name) = line
            .strip_prefix("[profiles.")
            .and_then(|rest| rest.strip_suffix(']'))
        {
            profile = name;
        } else if line.starts_with('[') {
            profile = "";
        }
        let line = match line {
            line if !profile.is_empty() && line.starts_with("provider = ") => {
                let provider = if profile == "review" {
                    reviewer
                } else {
                    worker
                };
                format!("provider = \"{provider}\"")
            }
            // Model examples name other providers' models.
            line if !profile.is_empty() && line.starts_with("# model = ") => continue,
            line => line.to_owned(),
        };
        text.push_str(&line);
        text.push('\n');
    }
    Config::from_toml(&text)?;
    Ok(text)
}

/// `antislop init`: writes `antislop.toml` in `project` for the CLIs found
/// on PATH. Refuses to replace an existing file unless `force`.
pub fn init(project: &Path, force: bool, out: &mut dyn std::io::Write) -> Result<PathBuf> {
    let path = project.join("antislop.toml");
    crate::require(
        force || !path.exists(),
        &format!(
            "{} already exists; edit it, or pass --force to replace it",
            path.display()
        ),
    )?;
    let found = KNOWN
        .iter()
        .filter(|(binary, _)| find_executable(Path::new(binary)).is_some())
        .collect::<Vec<_>>();
    let names = KNOWN.map(|(binary, _)| binary).join(", ");
    let (worker, _) = found.first().ok_or_else(|| {
        crate::Error::Invalid(format!(
            "none of {names} is on PATH; install one and log in first"
        ))
    })?;
    let (reviewer, _) = found.get(1).unwrap_or(&found[0]);
    std::fs::write(&path, initial_config(worker, reviewer)?)?;
    writeln!(out, "wrote {}", path.display())?;
    writeln!(out, "  {worker} orchestrates and works; {reviewer} reviews")?;
    if [*worker, *reviewer].contains(&"agy") && super::launch::agy_ready().is_err() {
        writeln!(
            out,
            "  run `antislop setup agy` once so agy can reach antislop"
        )?;
    }
    writeln!(out, "next: `antislop doctor`, then `antislop`")?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executables_resolve_by_path_or_on_path() {
        let dir = tempfile::tempdir().unwrap();
        let tool = dir.path().join("tool");
        std::fs::write(&tool, "#!/bin/sh\n").unwrap();
        assert_eq!(find_executable(&tool), None, "not executable yet");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(find_executable(&tool), Some(tool.clone()));
        assert_eq!(find_executable(&dir.path().join("missing")), None);
        assert!(find_executable(Path::new("sh")).is_some());
        assert_eq!(find_executable(Path::new("antislop-no-such-cli")), None);
    }

    #[test]
    fn a_team_whose_cli_is_missing_is_refused_naming_the_profiles_that_launch() {
        let dir = tempfile::tempdir().unwrap();
        let codex = dir.path().join("codex");
        std::fs::write(&codex, "#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
        let text = include_str!("../../examples/antislop.toml")
            .replace(
                "binary = \"codex\"",
                &format!("binary = {:?}", codex.display().to_string()),
            )
            .replace(
                "binary = \"agy\"",
                &format!(
                    "binary = {:?}",
                    dir.path().join("agy").display().to_string()
                ),
            );
        let config = Config::from_toml(&text).unwrap();
        let problems = launch_problems(&config, [WorkKind::Implementation]);
        assert!(!problems.is_empty());
        assert!(
            problems.iter().all(
                |problem| problem.contains("is not an executable file or on PATH")
                    && problem.contains("profiles that can be launched: orchestrator")
            ),
            "{problems:?}"
        );
        let mut fixed = config.clone();
        for profile in fixed.profiles.values_mut() {
            profile.provider = ProviderKind::Codex;
        }
        assert!(launch_problems(&fixed, [WorkKind::Implementation]).is_empty());
    }

    #[test]
    fn the_initial_configuration_routes_work_and_review_to_the_cli_found() {
        for (worker, reviewer) in [("codex", "claude"), ("claude", "claude"), ("gemini", "agy")] {
            let config = Config::from_toml(&initial_config(worker, reviewer).unwrap()).unwrap();
            let implementation = config
                .resolve(WorkKind::Implementation, RoleKind::Executor)
                .unwrap();
            let review = config
                .resolve(WorkKind::Implementation, RoleKind::Reviewer)
                .unwrap();
            assert_eq!(super::super::agent::name(implementation.provider), worker);
            assert_eq!(super::super::agent::name(review.provider), reviewer);
            assert_eq!(config.providers.len(), 1 + usize::from(worker != reviewer));
        }
    }
}
