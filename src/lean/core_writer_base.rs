use super::core_plan_helpers::writer_base_scratch;
use super::*;

impl Core {
    pub(in crate::lean) fn writer_base(
        &self,
        plan_id: &str,
        spec: &TaskContract,
        project: &Path,
    ) -> Result<String> {
        let _integration = self.integrating.lock().expect("integration mutex");
        let onto = self.plan_base(plan_id, project)?;
        // A result that landed is in the project the base was taken from.
        let landed = self.plan_base_record(plan_id)?.landed;
        let changes = self.writer_dependency_changes(plan_id, spec, &landed)?;
        if changes.is_empty() {
            return Ok(onto);
        }
        let scratch = writer_base_scratch(&self.state, plan_id, &spec.id);
        git::combine(project, &scratch, &onto, &changes).map_err(|error| {
            Error::Invalid(format!(
                "could not combine the results of {} for {}: {error}",
                spec.depends_on.join(", "),
                spec.id
            ))
        })
    }

    fn writer_dependency_changes(
        &self,
        plan_id: &str,
        spec: &TaskContract,
        landed: &[(String, String)],
    ) -> Result<Vec<(String, String)>> {
        Ok(self
            .dependency_tasks(plan_id, spec)?
            .into_iter()
            .filter(|task| {
                !landed
                    .iter()
                    .any(|(id, result)| *id == task.id && task.result.as_ref() == Some(result))
            })
            .filter_map(|task| Some((task.base?, task.result?)))
            .collect())
    }

    /// The commit every task of a plan starts from. A snapshot is a new commit
    /// each time, even of an unchanged tree, so tasks that snapshotted when
    /// they started disagreed about their baseline and kept asking which
    /// commit was authoritative. The first task of the plan fixes it; what the
    /// user changes later stays protected when results are integrated. A
    /// result of the plan landing in the project releases it: tasks starting
    /// afterwards begin from the project that holds that result, since the
    /// result of a conflict repair no longer applies to the old commit.
    fn plan_base(&self, plan_id: &str, project: &Path) -> Result<String> {
        let mut record = self.plan_base_record(plan_id)?;
        if let Some(pinned) = &record.base
            && git::tree_identity(project, pinned).is_ok()
        {
            return Ok(pinned.clone());
        }
        let base = if git::uncommitted(project)?.is_empty() {
            git::head(project)?
        } else {
            git::snapshot(project, "User working tree before task")?
        };
        record.base = Some(base.clone());
        self.save_plan_base_record(plan_id, &record)?;
        Ok(base)
    }

    /// Records that a task's result landed in the project, which releases
    /// the plan's base. Called while integration is serialized.
    pub(in crate::lean) fn record_plan_landing(
        &self,
        plan_id: &str,
        task: &str,
        result: &str,
    ) -> Result<()> {
        let mut record = self.plan_base_record(plan_id)?;
        record.base = None;
        record.landed.retain(|(id, _)| id != task);
        record.landed.push((task.to_owned(), result.to_owned()));
        self.save_plan_base_record(plan_id, &record)
    }

    fn plan_base_path(&self, plan_id: &str) -> PathBuf {
        self.state
            .join("plan-bases")
            .join(crate::hash(plan_id.as_bytes()))
    }

    fn plan_base_record(&self, plan_id: &str) -> Result<PlanBase> {
        let path = self.plan_base_path(plan_id);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(PlanBase::default());
            }
            Err(error) => return Err(error.into()),
        };
        serde_json::from_str(&text).map_err(|error| {
            Error::Invalid(format!("invalid plan base {}: {error}", path.display()))
        })
    }

    fn save_plan_base_record(&self, plan_id: &str, record: &PlanBase) -> Result<()> {
        let path = self.plan_base_path(plan_id);
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory)?;
        }
        use std::io::Write;
        let mut temporary =
            tempfile::NamedTempFile::new_in(path.parent().expect("plan base parent"))?;
        temporary.write_all(&serde_json::to_vec(record)?)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&path).map_err(|error| error.error)?;
        Ok(())
    }
}

/// A plan's base commit and the task results that have landed since the
/// plan started, each as (task id, result commit).
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanBase {
    #[serde(default)]
    base: Option<String>,
    #[serde(default)]
    landed: Vec<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_or_corrupt_plan_base_is_reported_without_reinterpreting_it_as_a_commit() {
        let directory = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
        let core = Core::open(&directory.path().join("state"), config).unwrap();
        let path = core.plan_base_path("plan");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        for raw in ["0123456789012345678901234567890123456789", "{truncated"] {
            std::fs::write(&path, raw).unwrap();
            let error = core.plan_base_record("plan").err().unwrap().to_string();
            assert!(error.contains("invalid plan base"), "{error}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        }
        core.save_plan_base_record(
            "plan",
            &PlanBase {
                base: Some("current".into()),
                landed: vec![],
            },
        )
        .unwrap();
        assert_eq!(
            core.plan_base_record("plan").unwrap().base.as_deref(),
            Some("current")
        );
    }
}
