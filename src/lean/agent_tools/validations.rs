//! Agents keep execution in their own CLI; the runtime checks reuse against file state.
use super::*;
use store::{ValidationRun, ValidationSpec};

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
                json!({"validations":runs,"complete":cursor.is_none(),"cursor":cursor,"note":"Historical index only. Call prepare_validation with the same spec to establish whether it applies to your current files. Read raw output with read_evidence."}),
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
        let problem = args["problems"].as_str().ok_or_else(|| {
            Error::Invalid("problems: describe errors, or supply an empty string if none".into())
        })?;
        let output = required_text(args, "output")?;
        let text = format!(
            "Command: {}\nDirectory: {}\nScope: {}\nEnvironment: {}\nSource tree: {:?}\nExit code: {exit_code}\nReported problems: {problem}\n\n{output}",
            serde_json::to_string(&run.spec.command)?,
            run.spec.directory,
            run.spec.scope,
            run.spec.environment,
            run.tree
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
        run.problem = problem.into();
        run.evidence = Some(evidence.id);
        Ok(validation_response(
            self.store.complete_validation(conversation, turn, &run)?,
            "recorded",
        ))
    }
}

fn validation_response(run: ValidationRun, action: &str) -> Value {
    let instruction = match action {
        "reuse" => {
            "Do not rerun this command. Read its retained evidence and cite the validation/evidence IDs as reused; you did not execute it yourself. Reuse proves the check, not task approval or availability of a compiled artifact in this checkout."
        }
        "wait" => {
            "Another turn owns this identical validation. Continue independent work; call prepare_validation again for its result. Do not launch a duplicate command."
        }
        "run" => {
            "Execute this exact command in your own CLI with its existing permission policy, then call complete_validation with the exit code, decisive raw output and any reported errors. If files, environment or external inputs change before execution, prepare again. Reuse is disabled for unversioned directories."
        }
        _ => {
            "The observation and evidence are retained. Only passed, unchanged, completed observations can be reused by another turn; this result never grants task approval."
        }
    };
    json!({"action":action,"validation":run,"instruction":instruction})
}

fn validation_tree(root: &Path, versioned: bool) -> Result<Option<String>> {
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
