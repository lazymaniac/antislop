use crate::lean::core::task::feedback_helpers::review_json;
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
        let code_review = self.store.team(conversation)? == tasks::Team::CodeReview;
        let results = task_results(&tasks);
        let mut text = if code_review {
            format!("# Code review results\n\nReviewed scope and candidate reports:\n{results}")
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
        let source = self.locks.source(input.conversation);
        let _source = source.lock().expect("conversation source scope");
        let _graph = self.locks.graph();
        let _integration = self.locks.integration();
        if input.cancel.load(Ordering::SeqCst)
            || self.final_audit_binding(input.conversation, input.plan_id)? != binding
        {
            audit=Err("plan, scope or integrated files changed during final audit; audit the current result again".into());
        }
        let mut audit_report = None;
        if let Ok((report, _)) = &audit {
            audit_report = Some(report.clone());
        }
        let mut verdict = audit.map(|(_, verdict)| verdict);
        apply_audit_gates(&mut verdict, input.tasks);
        let mut payload = review_json(&verdict);
        if code_review {
            payload["report"] = json!(audit_report);
        }
        payload["plan"] = json!(input.plan_id);
        payload["binding"] = json!(binding);
        self.store.append_event(
            input.conversation,
            None,
            Some(input.plan_id),
            "audit",
            &payload,
        )?;
        text.push_str(&audit_text(&verdict, code_review, audit_report.as_deref()));
        Ok(())
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
            let integrated = self.session_worktree(conversation, &project)?;
            let artifacts = specs
                .iter()
                .filter(|task| task.delivery == Some(tasks::DeliveryMode::Untracked))
                .flat_map(|task| &task.outputs)
                .map(|path| project.join(path).to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            (
                git::integrated_fingerprint(&integrated)?,
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
            "{}\nAudit the integrated result at {}. Artifacts with delivery=untracked are delivered at the original project {} instead; inspect their declared output paths there.\n",
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
            |first, request| {
                let audit_request = format!("{sources}\nNew messages:\n{request}");
                if first {
                    if code_review {
                        tasks::final_scope_audit_prompt(&project_text, &audit_request, results)
                    } else {
                        tasks::audit_prompt(&project_text, &audit_request, results)
                    }
                } else {
                    if code_review {
                        tasks::final_scope_audit_prompt(&project_text, &audit_request, results)
                    } else {
                        tasks::audit_followup(&audit_request, results)
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

fn task_results(tasks: &[Task]) -> String {
    tasks.iter().map(task_result).collect()
}

fn report_intro(results: &str) -> String {
    format!(
        "The plan finished. Results of each task follow. Answer the user's \
         request using them; review anything doubtful in the files yourself.\n{results}"
    )
}

fn task_result(task: &Task) -> String {
    let report = task.report.as_deref().unwrap_or("(no report)");
    let findings = task_findings(task);
    format!(
        "\n## {} — {} ({}, {} round(s))\nGoal: {}\nAcceptance criteria: {}\nWorker report:\n{report}\nOpen reviewer findings:\n{findings}\n",
        task.spec["id"].as_str().unwrap_or_default(),
        task.spec["title"].as_str().unwrap_or_default(),
        task.status,
        task.round,
        task.spec["goal"].as_str().unwrap_or_default(),
        task.spec["acceptance_criteria"],
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
        return format!("task ended as {} without approval", task.status);
    };
    if verdict.approved {
        format!(
            "the reviewer approved this task's worktree, but the task ended as {}",
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

fn audit_text(
    audit: &std::result::Result<Verdict, String>,
    code_review: bool,
    report: Option<&str>,
) -> String {
    match audit {
        Ok(verdict) if code_review => {
            let report =
                report.unwrap_or("The independent auditor returned no user-facing report text.");
            let overall = if verdict.approved {
                "**Overall verdict: no blocking findings.**"
            } else {
                "**Overall verdict: unresolved findings or checks remain.**"
            };
            let findings = if verdict.findings.is_empty() {
                String::new()
            } else {
                format!(
                    "\nAdditional audit findings:\n{}\n",
                    tasks::findings_text(&verdict.findings)
                )
            };
            format!("\n## Independent code review\n{overall}\n\n{report}\n{findings}")
        }
        Ok(verdict) if verdict.approved => {
            "\n## Independent audit\nThe request is fully delivered.\n".to_owned()
        }
        Ok(verdict) => format!(
            "\n## Independent audit\nThe request is not fully delivered:\n{}\nPropose a plan for what is missing, or tell the user why it stays open.\n",
            tasks::findings_text(&verdict.findings)
        ),
        Err(error) => format!(
            "\n## Independent audit\nThe audit could not run ({error}); tell the user the result is unaudited.\n"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_review_report_presents_scope_verdict_and_confirmed_findings_or_none() {
        let clean = Verdict {
            approved: true,
            findings: vec![],
            coverage: serde_json::Map::new(),
            resolved: vec![],
        };
        let clean_report = "Scope: src/lib.rs:1-80\nVerdict: no confirmed findings\nUnresolved: dependency behavior not exercised; no tests run.";
        let clean_text = audit_text(&Ok(clean), true, Some(clean_report));
        assert!(clean_text.contains("Independent code review"));
        assert!(clean_text.contains("Scope: src/lib.rs:1-80"));
        assert!(clean_text.contains("no confirmed findings"));
        assert!(clean_text.contains("Unresolved: dependency behavior not exercised"));
        assert!(!clean_text.contains("Additional audit findings"));

        let with_finding = Verdict {
            approved: false,
            findings: vec![
                json!({"severity":"medium","issue":"Input X causes Y","where":"src/lib.rs:42"}),
            ],
            coverage: serde_json::Map::new(),
            resolved: vec![],
        };
        let text = audit_text(
            &Ok(with_finding),
            true,
            Some("Verdict: one confirmed issue"),
        );
        assert!(text.contains("one confirmed issue"));
        assert!(text.contains("[medium] Input X causes Y (src/lib.rs:42)"));
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
