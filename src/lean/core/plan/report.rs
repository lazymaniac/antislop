use crate::lean::core::task::feedback_helpers::review_json;
use crate::lean::prompts;
use crate::lean::*;

impl Core {
    /// The finished plan's results, and an independent audit of whether they
    /// deliver the user's request, for the orchestrator to answer the user.
    pub fn plan_report(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        cancel: &AtomicBool,
    ) -> Result<String> {
        let tasks = self.store.plan(plan_id)?;
        let team = self.store.team(conversation)?;
        let code_review = team == tasks::Team::CodeReview;
        let results = task_results(&self.store, &tasks)?;
        if team == tasks::Team::NoQualityGates {
            return Ok(report_intro(&results));
        }
        let mut text = if code_review {
            format!(prompts::template!(PLAN_REPORT), results = results)
        } else {
            report_intro(&results)
        };
        if !cancel.load(Ordering::SeqCst) {
            self.append_plan_audit(
                PlanAuditInput {
                    conversation,
                    plan_id,
                    cancel,
                    tasks: &tasks,
                    results: &results,
                },
                &mut text,
            )?;
        }
        Ok(text)
    }

    fn append_plan_audit(
        self: &Arc<Self>,
        input: PlanAuditInput<'_>,
        text: &mut String,
    ) -> Result<()> {
        let code_review = self.store.team(input.conversation)? == tasks::Team::CodeReview;
        let binding = self.final_audit_binding(input.conversation, input.plan_id)?;
        let mut audit = self.run_report_audit(
            input.conversation,
            input.plan_id,
            input.cancel,
            input.results,
        )?;
        let audit_report = audit.as_ref().ok().map(|(report, _)| report.clone());
        let source = self.locks.source(input.conversation);
        let _source = source.lock().expect("conversation source scope");
        let _graph = self.locks.graph();
        let _integration = self.locks.integration();
        if input.cancel.load(Ordering::SeqCst) {
            let reason = self
                .store
                .last_turn(input.conversation, AUDITOR)?
                .and_then(|turn| turn.error)
                .unwrap_or_else(|| {
                    "final audit cancelled; retry the audit to finish this session".into()
                });
            audit = Err(reason);
        } else if self.final_audit_binding(input.conversation, input.plan_id)? != binding {
            audit=Err("plan, scope or integrated files changed during final audit; audit the current result again".into());
        }
        let mut verdict = audit.map(|(_, verdict)| verdict);
        apply_audit_gates(&mut verdict, input.tasks);
        let audit_ref = self.persist_plan_audit(
            input.conversation,
            input.plan_id,
            &binding,
            code_review,
            audit_report.as_deref(),
            &verdict,
        )?;
        text.push_str(&format!(
            prompts::template!(PLAN_AUDIT_RESULT),
            match &verdict {
                Ok(v) if v.approved => "approved",
                Ok(_) => prompts::AUDIT_NOT_APPROVED,
                Err(_) => "unaudited",
            },
            audit_ref = audit_ref
        ));
        Ok(())
    }

    fn persist_plan_audit(
        &self,
        conversation: &str,
        plan_id: &str,
        binding: &str,
        code_review: bool,
        report: Option<&str>,
        verdict: &std::result::Result<Verdict, String>,
    ) -> Result<u64> {
        let mut payload = review_json(verdict);
        payload["report"] = json!(report);
        payload["code_review"] = json!(code_review);
        payload["plan"] = json!(plan_id);
        payload["binding"] = json!(binding);
        self.store
            .append_event(conversation, None, Some(plan_id), "audit", &payload)
    }

    pub(in crate::lean) fn final_audit_binding(
        &self,
        conversation: &str,
        plan: &str,
    ) -> Result<String> {
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        let specs = self
            .store
            .plan_specs(plan)?
            .into_iter()
            .map(serde_json::from_value::<TaskContract>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let files = if git::is_repository(&project) {
            let integrated = self.session_result_fingerprint(conversation, &project)?;
            let artifacts = specs
                .iter()
                .filter(|task| task.delivery == Some(tasks::DeliveryMode::Untracked))
                .flat_map(|task| &task.outputs)
                .map(|path| project.join(path).to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            (
                integrated,
                if artifacts.is_empty() {
                    String::new()
                } else {
                    git::recovery_fingerprint(&project, &artifacts)?
                },
            )
        } else {
            let outputs = specs
                .into_iter()
                .flat_map(|task| task.outputs)
                .collect::<Vec<_>>();
            (
                git::recovery_fingerprint(&project, &outputs)?,
                String::new(),
            )
        };
        Ok(crate::hash(serde_json::to_vec(&(
            self.plan_revision_token(conversation, plan)?,
            files,
        ))?))
    }

    pub(in crate::lean) fn final_audit_current(
        &self,
        conversation: &str,
        plan: &str,
    ) -> Result<bool> {
        let event = self.store.latest_audit_event(conversation)?;
        let Some(event) = event else {
            return Ok(false);
        };
        Ok(event.payload["plan"] == plan
            && event.payload["approved"] == true
            && event.payload["binding"] == self.final_audit_binding(conversation, plan)?)
    }

    fn run_report_audit(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        cancel: &AtomicBool,
        results: &str,
    ) -> Result<std::result::Result<(String, Verdict), String>> {
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        let integrated = if git::is_repository(&project) {
            self.session_worktree(conversation, &project)?
        } else {
            project.clone()
        };
        let project_text = integrated.to_string_lossy().into_owned();
        let sources = format!(
            prompts::template!(INTEGRATED_AUDIT_SCOPE),
            self.plan_scope_context(conversation, plan_id)?,
            integrated.display(),
            project.display()
        );
        let code_review = self.store.team(conversation)? == tasks::Team::CodeReview;
        self.review_with_report(
            conversation,
            AUDITOR,
            Some(plan_id),
            &integrated,
            cancel,
            |first, request, inputs| {
                let audit_request = format!(
                    prompts::template!(AUDIT_NEW_MESSAGES),
                    sources = sources,
                    request = request
                );
                if first {
                    if code_review {
                        tasks::final_scope_audit_prompt(
                            &project_text,
                            &audit_request,
                            results,
                            inputs,
                        )
                    } else {
                        tasks::render_result_audit(&project_text, &audit_request, results, inputs)
                    }
                } else {
                    if code_review {
                        tasks::final_scope_audit_prompt(
                            &project_text,
                            &audit_request,
                            results,
                            inputs,
                        )
                    } else {
                        tasks::render_audit_continuation(&audit_request, results, inputs)
                    }
                }
            },
        )
        .map(|result| result.map(|outcome| (outcome.report, outcome.verdict)))
    }
}

struct PlanAuditInput<'a> {
    conversation: &'a str,
    plan_id: &'a str,
    cancel: &'a AtomicBool,
    tasks: &'a [Task],
    results: &'a str,
}

fn task_results(store: &Store, tasks: &[Task]) -> Result<String> {
    tasks
        .iter()
        .map(|task| {
            let summary = match store.task_summary(task) {
                Ok(summary) => summary,
                Err(Error::Sql(rusqlite::Error::QueryReturnedNoRows)) => task.spec["summary"]
                    .as_str()
                    .unwrap_or(crate::lean::prompts::FULL_RESULT_REFERENCE)
                    .to_owned(),
                Err(error) => return Err(error),
            };
            Ok(task_result(task, &summary))
        })
        .collect()
}

fn report_intro(results: &str) -> String {
    format!(prompts::template!(REPORT_INTRO), results = results)
}

fn task_result(task: &Task, summary: &str) -> String {
    let findings = task_findings(task);
    format!(
        prompts::template!(TASK_RESULT),
        task.spec["id"].as_str().unwrap_or_default(),
        task.spec["title"].as_str().unwrap_or_default(),
        task.status,
        task.round,
        summary,
        task.id,
        task.id,
        findings = findings
    )
}

fn task_findings(task: &Task) -> String {
    if task.status == status::APPROVED {
        return "none".into();
    }
    let Some(verdict) = task
        .verdict
        .as_ref()
        .and_then(|value| serde_json::from_value::<Verdict>(value.clone()).ok())
    else {
        return format!(prompts::template!(UNAPPROVED_TASK_RESULT), task.status);
    };
    if verdict.approved {
        format!(
            prompts::template!(APPROVED_WORKTREE_UNFINISHED_TASK),
            task.status
        )
    } else {
        let text = tasks::findings_text(&verdict.findings);
        if text.is_empty() { "none".into() } else { text }
    }
}

fn apply_audit_gates(audit: &mut std::result::Result<Verdict, String>, tasks: &[Task]) {
    if let Ok(verdict) = audit
        && verdict.approved
        && tasks.iter().any(|task| task.status != status::APPROVED)
    {
        verdict.approved = false;
        verdict.findings.push(json!({
            "severity":"high",
            "issue":format!("Tasks in plan were not approved: {}", unapproved_task_ids(tasks)),
            "where":"plan"
        }));
    }
}

fn unapproved_task_ids(tasks: &[Task]) -> String {
    tasks
        .iter()
        .filter(|task| task.status != status::APPROVED)
        .map(|task| {
            format!(
                "{}: {}",
                task.spec["id"].as_str().unwrap_or(&task.id),
                task.status
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_final_audit_persists_its_own_report_and_verdict() {
        for code_review in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let project = dir.path().join("project");
            std::fs::create_dir(&project).unwrap();
            let config =
                Config::from_toml(include_str!("../../../../examples/antislop.toml")).unwrap();
            let core = Core::open(&dir.path().join("state"), config).unwrap();
            let conversation = core
                .store
                .create_conversation(project.to_str().unwrap())
                .unwrap()
                .id;
            core.store
                .select_team(
                    &conversation,
                    if code_review {
                        tasks::Team::CodeReview
                    } else {
                        tasks::Team::Feature
                    },
                    "Audit run",
                )
                .unwrap();
            let tasks = core
            .store
            .add_plan(
                &conversation,
                &[json!({
                    "id":"review",
                    "title":"Review candidate",
                    "goal":"Inspect the candidate",
                    "kind":"code_review",
                    "change_kind":"none",
                    "acceptance_criteria":[{"id":"C1","text":"Findings are supported by evidence"}]
                })],
            )
            .unwrap();
            let plan_id = &tasks[0].plan_id;
            let report = "Scope: src/lib.rs:1-80\nVerdict: no confirmed findings";
            let verdict = Ok(Verdict {
                approved: false,
                findings: vec![json!({
                    "severity":"medium",
                    "issue":"Input X causes Y",
                    "where":"src/lib.rs:42"
                })],
                coverage: serde_json::Map::new(),
                resolved: vec![],
            });

            core.persist_plan_audit(
                &conversation,
                plan_id,
                "binding-123",
                code_review,
                Some(report),
                &verdict,
            )
            .unwrap();

            let event = core
                .store
                .latest_audit_event(&conversation)
                .unwrap()
                .expect("final audit event was persisted");
            assert_eq!(event.kind, "audit");
            assert_eq!(event.plan_id.as_deref(), Some(plan_id.as_str()));
            assert_eq!(event.payload["plan"], json!(plan_id));
            assert_eq!(event.payload["binding"], "binding-123");
            assert_eq!(event.payload["report"], report);
            assert_eq!(event.payload["approved"], false);
            assert_eq!(event.payload["findings"][0]["issue"], "Input X causes Y");
            assert_eq!(event.payload["findings"][0]["where"], "src/lib.rs:42");
        }
    }

    #[test]
    fn no_quality_gates_plan_report_does_not_start_or_append_a_final_audit() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let config = Config::from_toml(include_str!("../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(project.to_str().unwrap())
            .unwrap()
            .id;
        core.store
            .select_team(
                &conversation,
                tasks::Team::NoQualityGates,
                "No quality pipeline",
            )
            .unwrap();
        let tasks = core
            .store
            .add_plan(
                &conversation,
                &[json!({
                    "id":"implementation",
                    "title":"Implement requested change",
                    "goal":"Make the requested change",
                    "kind":"implementation",
                    "change_kind":"code",
                    "acceptance_criteria":[{"id":"C1","text":"The requested behavior is present"}]
                })],
            )
            .unwrap();
        let report = core
            .plan_report(&conversation, &tasks[0].plan_id, &AtomicBool::new(false))
            .unwrap();
        assert!(report.contains("Results of each task follow"));
        assert!(!report.contains("Independent audit"));
        assert!(
            core.store
                .latest_audit_event(&conversation)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn an_audit_is_current_only_for_the_same_scope_plan_and_integrated_files() {
        for git_project in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let project = dir.path().join("project");
            std::fs::create_dir(&project).unwrap();
            if git_project {
                git::ok(&project, &["init", "--quiet", "--initial-branch=main"]).unwrap();
            }
            std::fs::write(project.join("result.txt"), "accepted result").unwrap();
            if git_project {
                git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
            }
            std::fs::write(project.join("report.md"), "accepted artifact").unwrap();
            let config =
                Config::from_toml(include_str!("../../../../examples/antislop.toml")).unwrap();
            let core = Core::open(&dir.path().join("state"), config).unwrap();
            let conversation = core
                .store
                .create_conversation(project.to_str().unwrap())
                .unwrap()
                .id;
            let tasks=core.store.add_plan(&conversation,&[json!({"id":"deliver","title":"Deliver","goal":"Deliver the result","outputs":["result.txt"],"kind":"analysis","change_kind":"none","acceptance_criteria":[{"id":"C1","text":"Result contains the accepted text"}]}), json!({"id":"report","title":"Report","goal":"Deliver the report","kind":"writing","change_kind":"artifact","delivery":"untracked","outputs":["report.md"],"acceptance_criteria":[{"id":"C1","text":"Report matches the accepted artifact"}]})]).unwrap();
            let plan = &tasks[0].plan_id;
            let integrated = if git_project {
                core.session_worktree(&conversation, &project).unwrap()
            } else {
                project.clone()
            };
            let scope = core
                .store
                .create_scope(&conversation, "Deliver the accepted text")
                .unwrap();
            core.store
                .bind_plan_scope(&conversation, plan, &scope.id)
                .unwrap();
            assert!(!core.final_audit_current(&conversation, plan).unwrap());
            let binding = core.final_audit_binding(&conversation, plan).unwrap();
            core.store
                .append_event(
                    &conversation,
                    None,
                    Some(plan),
                    "audit",
                    &json!({"plan":plan,"approved":true,"binding":binding}),
                )
                .unwrap();
            assert!(core.final_audit_current(&conversation, plan).unwrap());
            std::fs::write(integrated.join("result.txt"), "different result").unwrap();
            assert!(!core.final_audit_current(&conversation, plan).unwrap());
            std::fs::write(integrated.join("result.txt"), "accepted result").unwrap();
            assert!(core.final_audit_current(&conversation, plan).unwrap());
            std::fs::write(project.join("report.md"), "changed artifact").unwrap();
            assert!(!core.final_audit_current(&conversation, plan).unwrap());
            std::fs::write(project.join("report.md"), "accepted artifact").unwrap();
            assert!(core.final_audit_current(&conversation, plan).unwrap());
            if git_project {
                std::fs::write(project.join("result.txt"), "separate user edit").unwrap();
                assert!(
                    core.final_audit_current(&conversation, plan).unwrap(),
                    "the audit observes the session result, not the user's primary checkout"
                );
            }
            let replacement = core
                .store
                .create_scope(&conversation, "Changed acceptance scope")
                .unwrap();
            core.store
                .db()
                .execute(
                    "UPDATE plan_heads SET revision=revision+1 WHERE plan_id=?1",
                    [plan],
                )
                .unwrap();
            core.store
                .bind_plan_scope(&conversation, plan, &replacement.id)
                .unwrap();
            assert!(!core.final_audit_current(&conversation, plan).unwrap());
        }
    }
}
