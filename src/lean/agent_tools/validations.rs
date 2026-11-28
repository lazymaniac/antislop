//! Agents keep execution in their own CLI; the runtime checks reuse against file state.
use super::*;
use crate::lean::prompts;
use store::{ValidationObservation, ValidationRun, ValidationSpec};

impl Core {
    pub(super) fn validation_tool(
        &self,
        conversation: &str,
        turn: &str,
        tool: &str,
        args: &Value,
    ) -> Result<Value> {
        if tool == "read_validations" {
            let runs = self
                .store
                .validation_index(conversation, args["cursor"].as_str())?;
            let cursor = (runs.len() == 32)
                .then(|| runs.last().map(|run| run.id.clone()))
                .flatten();
            return Ok(
                json!({"validations":runs,"complete":cursor.is_none(),"cursor":cursor,"note":prompts::VALIDATION_HISTORY_GUIDANCE}),
            );
        }
        if tool == "invalidate_validation" {
            let id = required_text(args, "validation")?;
            let reason = required_text(args, "reason")?;
            return Ok(
                json!({"validation":self.store.invalidate_validation(conversation,turn,&id,&reason)?}),
            );
        }
        let context = self.store.run_context(conversation, turn)?.ok_or_else(|| {
            Error::Invalid("validation requires this turn's recorded working directory".into())
        })?;
        let allocation = context.cwd.canonicalize()?;
        let versioned = git::repository_probe(&allocation)?;
        let root = if versioned {
            PathBuf::from(git::ok(&allocation, &["rev-parse", "--show-toplevel"])?)
                .canonicalize()?
        } else {
            allocation.clone()
        };
        if tool == "prepare_validation" {
            let command: Vec<String> = serde_json::from_value(args["command"].clone())?;
            require(
                !command.is_empty()
                    && command.iter().all(|arg| !arg.contains('\0'))
                    && !command[0].is_empty(),
                "command: supply the executable and exact arguments without NUL characters",
            )?;
            require(
                !inline_shell(&command),
                "command: record each check as a direct executable with arguments; an inline shell can hide the check's exit status",
            )?;
            let directory = args["directory"].as_str().unwrap_or(".");
            require(
                !Path::new(directory).is_absolute(),
                "directory must be relative to your allocated checkout",
            )?;
            let directory = allocation.join(directory).canonicalize()?;
            require(
                directory.starts_with(&allocation) && directory.is_dir(),
                "directory is outside the allocated checkout",
            )?;
            let spec = ValidationSpec {
                command,
                directory: directory
                    .strip_prefix(&root)
                    .map_err(crate::invalid)?
                    .to_string_lossy()
                    .into_owned(),
                scope: required_text(args, "scope")?,
                environment: required_text(args, "environment")?,
            };
            let tree = validation_tree(&root, versioned)?;
            let (mut run, mut action) =
                self.store
                    .prepare_validation(conversation, turn, tree.as_deref(), &spec)?;
            let mut missing_evidence = None;
            if action == "reuse" {
                let evidence = run
                    .evidence
                    .as_deref()
                    .ok_or_else(|| {
                        Error::Invalid("validation evidence reference is missing".into())
                    })
                    .and_then(|id| self.store.evidence(conversation, id))
                    .and_then(|evidence| self.store.evidence_text(&evidence));
                if let Err(error) = evidence {
                    let reason = format!("prior validation evidence is unavailable: {error}");
                    self.store
                        .invalidate_validation(conversation, turn, &run.id, &reason)?;
                    (run, action) = self.store.prepare_validation(
                        conversation,
                        turn,
                        tree.as_deref(),
                        &spec,
                    )?;
                    missing_evidence = Some(reason);
                }
            }
            let mut response = validation_response(run, action);
            if let Some(reason) = missing_evidence {
                response["prior_evidence_error"] = json!(reason);
            }
            return Ok(response);
        }
        let id = required_text(args, "validation")?;
        let mut run = self.store.validation(conversation, &id)?;
        require(
            run.producer_turn == turn,
            "validation belongs to another turn",
        )?;
        let exit_code = args["exit_code"]
            .as_i64()
            .and_then(|code| i32::try_from(code).ok())
            .ok_or_else(|| Error::Invalid("exit_code: expected a process exit code".into()))?;
        let output = required_text(args, "output")?;
        let observation = observation(args, &run.spec, exit_code, &output)?;
        let problem = observation.problems.join("\n");
        let evidence_output = if run.status != "running" && run.observation.is_none() {
            // Preserve the exact evidence identity of already recorded legacy results.
            output
        } else {
            format!(
                "Observation: {}\n\n{output}",
                serde_json::to_string(&observation)?
            )
        };
        let text = format!(
            prompts::template!(VALIDATION_EVIDENCE),
            serde_json::to_string(&run.spec.command)?,
            run.spec.directory,
            run.spec.scope,
            run.spec.environment,
            run.tree,
            exit_code = exit_code,
            problem = problem,
            output = evidence_output
        );
        let evidence =
            self.store
                .record_evidence(turn, &format!("validation:{}", run.id), &text)?;
        if run.status != "running" {
            require(
                run.exit_code == Some(exit_code) && run.evidence.as_deref() == Some(&evidence.id),
                "validation already has a different immutable result",
            )?;
            return Ok(validation_response(run, "recorded"));
        }
        let after = validation_tree(&root, versioned)?;
        run.status = if exit_code != 0 || !problem.trim().is_empty() {
            "failed"
        } else if run.tree.is_none() || after.is_none() {
            "uncacheable"
        } else if run.tree != after {
            "changed"
        } else {
            "passed"
        }
        .into();
        run.exit_code = Some(exit_code);
        run.problem = problem;
        run.observation = Some(observation);
        run.evidence = Some(evidence.id);
        Ok(validation_response(
            self.store.complete_validation(conversation, turn, &run)?,
            "recorded",
        ))
    }
}

fn executable(command: &[String]) -> Option<&str> {
    Path::new(command.first()?).file_name()?.to_str()
}

fn inline_shell(command: &[String]) -> bool {
    matches!(
        executable(command),
        Some("sh" | "bash" | "zsh" | "dash" | "ksh" | "fish")
    ) && command[1..]
        .iter()
        .take_while(|arg| arg.starts_with('-') && *arg != "--")
        .any(|arg| arg == "--command" || (!arg.starts_with("--") && arg[1..].contains('c')))
}

fn observation(
    args: &Value,
    spec: &ValidationSpec,
    exit_code: i32,
    output: &str,
) -> Result<ValidationObservation> {
    // Older clients may still submit the formerly documented string.
    let mut problems = match args["problems"].as_str() {
        Some(text) if text.trim().is_empty() => Vec::new(),
        Some(text) => vec![text.into()],
        None => messages(&args["problems"], "problems")?,
    };
    let limitations = match args.get("limitations") {
        Some(value) => messages(value, "limitations")?,
        None => Vec::new(),
    };
    let declared = args
        .get("tests_executed")
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| crate::invalid("tests_executed: expected a nonnegative integer"))
        })
        .transpose()?;
    let is_cargo_test = matches!(executable(&spec.command), Some("cargo" | "cargo.sh"))
        && spec.command[1..]
            .iter()
            .find(|arg| !arg.starts_with('+') && !arg.starts_with('-'))
            .is_some_and(|arg| arg == "test")
        && !spec
            .command
            .iter()
            .any(|arg| arg == "--no-run" || arg == "--list");
    let mut summary = None;
    for line in output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("test result: "))
    {
        let Some((_, counts)) = line.split_once(". ") else {
            continue;
        };
        let mut counts = counts.split("; ");
        if let (Some(passed), Some(failed)) = (
            counts
                .next()
                .and_then(|part| part.strip_suffix(" passed"))
                .and_then(|count| count.parse::<u64>().ok()),
            counts
                .next()
                .and_then(|part| part.strip_suffix(" failed"))
                .and_then(|count| count.parse::<u64>().ok()),
        ) {
            let (total, failures) = summary.get_or_insert((0_u64, 0_u64));
            *total = total.saturating_add(passed).saturating_add(failed);
            *failures = failures.saturating_add(failed);
        }
    }
    let tests_executed = summary.map(|(total, _)| total).or(declared);
    if exit_code == 0 {
        if summary.is_some_and(|(total, _)| declared.is_some_and(|count| count != total)) {
            problems.push("Declared test count differs from the raw test summary.".into());
        }
        if summary.is_some_and(|(_, failed)| failed != 0) {
            problems.push("Raw test output reports a failure despite exit code 0.".into());
        }
        if tests_executed == Some(0) {
            problems.push("The check executed zero tests.".into());
        } else if is_cargo_test && summary.is_none() {
            problems.push("Cargo test evidence requires raw libtest totals.".into());
        }
    }
    Ok(ValidationObservation {
        problems,
        limitations,
        tests_executed,
    })
}

fn messages(value: &Value, field: &str) -> Result<Vec<String>> {
    let messages: Vec<String> = serde_json::from_value(value.clone())
        .map_err(|_| crate::invalid(format!("{field}: expected a list of messages")))?;
    require(
        messages.iter().all(|message| !message.trim().is_empty()),
        &format!("{field}: use a nonempty message or an empty list"),
    )?;
    Ok(messages)
}

fn validation_response(run: ValidationRun, action: &str) -> Value {
    let instruction = match action {
        "reuse" => prompts::VALIDATION_REUSE_RESULT,
        "wait" => prompts::VALIDATION_WAIT_RESULT,
        "run" => prompts::VALIDATION_RUN_RESULT,
        _ => prompts::VALIDATION_RECORDED_RESULT,
    };
    json!({"action":action,"validation":run,"instruction":instruction})
}

pub(super) fn validation_tree(root: &Path, versioned: bool) -> Result<Option<String>> {
    if !versioned {
        return Ok(None);
    }
    let status = git::git(
        root,
        &["status", "--porcelain=v2", "-z", "--ignore-submodules=none"],
    )?;
    require(
        status.status.success(),
        &format!("git status: {}", String::from_utf8_lossy(&status.stderr)),
    )?;
    let dirty_submodule = status.stdout.split(|byte| *byte == 0).any(|entry| {
        let field = entry.split(|byte| *byte == b' ').nth(2).unwrap_or_default();
        matches!(entry.first(), Some(b'1' | b'2'))
            && field.starts_with(b"S")
            && field
                .get(2..)
                .is_some_and(|tail| tail.iter().any(|byte| *byte != b'.'))
    });
    if dirty_submodule {
        return Ok(None);
    }
    git::integrated_fingerprint(root).map(Some)
}
