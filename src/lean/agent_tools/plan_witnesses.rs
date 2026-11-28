//! Read-only witnesses for acceptance criteria anchored to an immutable tree.
use super::*;
use std::collections::BTreeMap;

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Witness {
    Search {
        commit: String,
        pattern: String,
        #[serde(default)]
        paths: Vec<String>,
        expected: Option<usize>,
        expected_files: Option<usize>,
    },
    TestFilter {
        commit: String,
        discovery: String,
        selector: String,
        #[serde(default)]
        exact: bool,
        expected: Option<usize>,
    },
}

impl Core {
    pub(super) fn inspect_plan_witness(
        &self,
        conversation: &str,
        role: &str,
        args: &Value,
    ) -> Result<Value> {
        require(
            matches!(
                role,
                ORCHESTRATOR | COMMENT_ORCHESTRATOR | PLANNER | PLAN_REVIEWER
            ),
            "plan witnesses are available to planning roles",
        )?;
        let project = self.store.conversation(conversation)?.project;
        self.measure_witness(
            conversation,
            Path::new(&project),
            serde_json::from_value(args["witness"].clone())?,
        )
    }

    pub(super) fn check_revision_witnesses(
        &self,
        conversation: &str,
        args: &Value,
    ) -> Result<Vec<Value>> {
        if args.is_null() {
            return Ok(Vec::new());
        }
        let witnesses: Vec<Witness> = serde_json::from_value(args.clone())?;
        let project = self.store.conversation(conversation)?.project;
        witnesses
            .into_iter()
            .map(|witness| {
                let expected = match &witness {
                    Witness::Search { expected, .. } | Witness::TestFilter { expected, .. } => {
                        expected
                    }
                };
                require(
                    expected.is_some(),
                    "revision witness requires the expected match count",
                )?;
                let mut result =
                    self.measure_witness(conversation, Path::new(&project), witness)?;
                // Review needs the exact expression, counts and commit. Large inventories stay pageable via Git.
                result
                    .as_object_mut()
                    .expect("witness result")
                    .remove("samples");
                Ok(result)
            })
            .collect()
    }

    fn measure_witness(
        &self,
        conversation: &str,
        project: &Path,
        witness: Witness,
    ) -> Result<Value> {
        let commit = match &witness {
            Witness::Search { commit, .. } | Witness::TestFilter { commit, .. } => commit,
        };
        require(
            matches!(commit.len(), 40 | 64) && commit.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "commit: supply the full immutable Git commit id",
        )?;
        let commit = git::ok(
            project,
            &["rev-parse", "--verify", &format!("{commit}^{{commit}}")],
        )?;
        match witness {
            Witness::Search {
                pattern,
                paths,
                expected,
                expected_files,
                ..
            } => {
                require(
                    !pattern.is_empty() && !pattern.contains('\0'),
                    "pattern: supply a nonempty Git grep expression",
                )?;
                require(
                    paths.iter().all(|path| {
                        !path.starts_with(':')
                            && !Path::new(path).is_absolute()
                            && Path::new(path)
                                .components()
                                .all(|part| !matches!(part, std::path::Component::ParentDir))
                    }),
                    "paths: use repository-relative literal paths",
                )?;
                let command = [
                    "--literal-pathspecs",
                    "grep",
                    "-c",
                    "-z",
                    "-E",
                    "-e",
                    &pattern,
                    &commit,
                    "--",
                ];
                let arguments = command
                    .into_iter()
                    .chain(paths.iter().map(String::as_str))
                    .collect::<Vec<_>>();
                let output = git::git(project, &arguments)?;
                require(
                    matches!(output.status.code(), Some(0 | 1)),
                    &format!(
                        "git grep: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ),
                )?;
                let files = grep_counts(&output.stdout)?;
                let lines: usize = files.values().sum();
                match_count(expected, lines, "search lines")?;
                match_count(expected_files, files.len(), "search files")?;
                let samples: Vec<_> = files.iter().take(32)
                    .map(|(path, count)| json!({"path":String::from_utf8_lossy(path),"matches":count}))
                    .collect();
                Ok(
                    json!({"kind":"search","commit":commit,"pattern":pattern,"paths":paths,
                    "matches":lines,"files":files.len(),"samples":samples}),
                )
            }
            Witness::TestFilter {
                discovery,
                selector,
                exact,
                expected,
                ..
            } => {
                require(
                    !selector.trim().is_empty(),
                    "selector: supply the libtest filter",
                )?;
                let run = self.store.validation(conversation, &discovery)?;
                let tree = git::ok(
                    project,
                    &["rev-parse", "--verify", &format!("{commit}^{{tree}}")],
                )?;
                require(
                    run.status == "passed"
                        && run.tree.as_deref() == Some(&tree)
                        && run.spec.kind == Some(store::ValidationKind::Discovery)
                        && run.spec.command.iter().any(|arg| arg == "--list"),
                    "discovery: use a passing --list receipt for the named commit's exact tree",
                )?;
                let id = run
                    .evidence
                    .as_deref()
                    .ok_or_else(|| crate::invalid("discovery evidence is missing"))?;
                let text = self
                    .store
                    .evidence_text(&self.store.evidence(conversation, id)?)?;
                let (_, body) = text
                    .split_once("\n\nObservation: ")
                    .and_then(|(_, tail)| tail.split_once("\n\n"))
                    .ok_or_else(|| {
                        crate::invalid("discovery receipt has no retained raw output")
                    })?;
                let matches: Vec<_> = body
                    .lines()
                    .filter_map(|line| line.strip_suffix(": test"))
                    .filter(|name| {
                        if exact {
                            *name == selector
                        } else {
                            name.contains(&selector)
                        }
                    })
                    .collect();
                match_count(expected, matches.len(), "selected tests")?;
                Ok(
                    json!({"kind":"test_filter","commit":commit,"discovery":discovery,"selector":selector,
                    "exact":exact,"matches":matches.len(),"samples":matches.iter().take(32).collect::<Vec<_>>() }),
                )
            }
        }
    }
}

fn grep_counts(mut output: &[u8]) -> Result<BTreeMap<Vec<u8>, usize>> {
    let mut files = BTreeMap::new();
    while !output.is_empty() {
        let separator = output
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| crate::invalid("unexpected git grep path output"))?;
        let (anchor, rest) = output.split_at(separator);
        let prefix = anchor
            .iter()
            .position(|byte| *byte == b':')
            .ok_or_else(|| crate::invalid("git grep result is not anchored to a commit"))?;
        let rest = &rest[1..];
        let newline = rest
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or_else(|| crate::invalid("unexpected git grep count output"))?;
        let count = std::str::from_utf8(&rest[..newline]).map_err(crate::invalid)?;
        files.insert(
            anchor[prefix + 1..].to_vec(),
            count.parse().map_err(crate::invalid)?,
        );
        output = &rest[newline + 1..];
    }
    Ok(files)
}

fn match_count(expected: Option<usize>, actual: usize, field: &str) -> Result<()> {
    if let Some(expected) = expected {
        require(
            expected == actual,
            &format!("{field}: expected {expected}, measured {actual}"),
        )?;
    }
    Ok(())
}
