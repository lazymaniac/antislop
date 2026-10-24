use super::core_feedback_helpers::review_json;
use super::*;

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
        let results = task_results(&tasks);
        let compliance_gaps = self.report_compliance_gaps(conversation, plan_id, cancel);
        let mut text = report_intro(&results);
        if !cancel.load(Ordering::SeqCst) {
            self.append_plan_audit(
                PlanAuditInput {
                    conversation,
                    plan_id,
                    cancel,
                    tasks: &tasks,
                    results: &results,
                    compliance_gaps: &compliance_gaps,
                },
                &mut text,
            )?;
        }
        append_compliance_gaps(&mut text, &compliance_gaps);
        Ok(text)
    }

    fn report_compliance_gaps(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        cancel: &AtomicBool,
    ) -> Vec<String> {
        if cancel.load(Ordering::SeqCst) {
            return vec!["compliance audit cancelled".to_owned()];
        }
        self.compliance_audit(conversation, plan_id, cancel)
            .unwrap_or_else(|error| vec![format!("compliance audit failed: {error}")])
    }

    fn append_plan_audit(
        self: &Arc<Self>,
        input: PlanAuditInput<'_>,
        text: &mut String,
    ) -> Result<()> {
        let mut audit = self.run_report_audit(
            input.conversation,
            input.plan_id,
            input.cancel,
            input.results,
        )?;
        apply_audit_gates(&mut audit, input.tasks, input.compliance_gaps);
        self.store_report_audit(input.conversation, input.plan_id, &audit, text)?;
        Ok(())
    }

    fn run_report_audit(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        cancel: &AtomicBool,
        results: &str,
    ) -> Result<std::result::Result<Verdict, String>> {
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        let project_text = project.to_string_lossy().into_owned();
        self.review(
            conversation,
            AUDITOR,
            Some(plan_id),
            &project,
            cancel,
            |first, request| {
                if first {
                    tasks::audit_prompt(&project_text, request, results)
                } else {
                    tasks::audit_followup(request, results)
                }
            },
        )
    }

    fn store_report_audit(
        &self,
        conversation: &str,
        plan_id: &str,
        audit: &std::result::Result<Verdict, String>,
        text: &mut String,
    ) -> Result<()> {
        let mut payload = review_json(audit);
        payload["plan"] = json!(plan_id);
        self.store
            .append_event(conversation, None, Some(plan_id), "audit", &payload)?;
        text.push_str(&audit_text(audit));
        Ok(())
    }
}

struct PlanAuditInput<'a> {
    conversation: &'a str,
    plan_id: &'a str,
    cancel: &'a AtomicBool,
    tasks: &'a [Task],
    results: &'a str,
    compliance_gaps: &'a [String],
}

fn task_results(tasks: &[Task]) -> String {
    tasks.iter().map(task_result).collect()
}

fn report_intro(results: &str) -> String {
    format!(
        "The plan finished. Results of each task follow. Answer the user's \
         request using them; verify anything doubtful in the files yourself.\n{results}"
    )
}

fn task_result(task: &Task) -> String {
    let report = task.report.as_deref().unwrap_or("(no report)");
    let findings = task_findings(task);
    format!(
        "\n## {} — {} ({}, {} round(s))\nGoal: {}\nWorker report:\n{report}\nOpen verifier findings:\n{findings}\n",
        task.spec["id"].as_str().unwrap_or_default(),
        task.spec["title"].as_str().unwrap_or_default(),
        task.status,
        task.round,
        task.spec["goal"].as_str().unwrap_or_default(),
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
            "the verifier approved this task's worktree, but the task ended as {}",
            task.status
        )
    } else {
        let text = tasks::findings_text(&verdict.findings);
        if text.is_empty() { "none".into() } else { text }
    }
}

fn apply_audit_gates(
    audit: &mut std::result::Result<Verdict, String>,
    tasks: &[Task],
    gaps: &[String],
) {
    if let Ok(verdict) = audit {
        verdict.findings.extend(gaps.iter().map(|gap| {
            json!({
                "severity":"high", "issue":gap, "where":"requirement ledger"
            })
        }));
        invalidate_unearned_approval(verdict, tasks, gaps);
    }
}

fn invalidate_unearned_approval(verdict: &mut Verdict, tasks: &[Task], gaps: &[String]) {
    if !verdict.approved
        || (gaps.is_empty() && tasks.iter().all(|task| task.status == status::APPROVED))
    {
        return;
    }
    verdict.approved = false;
    verdict.findings.push(json!({
        "severity":"high",
        "issue":format!("Tasks in plan were not approved: {}", unapproved_task_ids(tasks)),
        "where":"plan"
    }));
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

fn audit_text(audit: &std::result::Result<Verdict, String>) -> String {
    match audit {
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

fn append_compliance_gaps(text: &mut String, gaps: &[String]) {
    if gaps.is_empty() {
        return;
    }
    text.push_str(&format!("\nRuntime requirements_complete gate is CLOSED. Do not claim success. Open obligations:\n{}\nPropose executor work for missing evidence; verifiers stay read-only.\n", gaps.join("\n")));
}
