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
        let mut results = String::new();
        for task in &tasks {
            let report = task.report.as_deref().unwrap_or("(no report)");
            let findings = if task.status == status::APPROVED {
                "none".into()
            } else if let Some(verdict) = task
                .verdict
                .as_ref()
                .and_then(|verdict| serde_json::from_value::<Verdict>(verdict.clone()).ok())
            {
                if verdict.approved {
                    format!(
                        "the verifier approved this task's worktree, but the task ended as {}",
                        task.status
                    )
                } else {
                    let text = tasks::findings_text(&verdict.findings);
                    if text.is_empty() { "none".into() } else { text }
                }
            } else {
                format!("task ended as {} without approval", task.status)
            };
            results.push_str(&format!(
                "\n## {} — {} ({}, {} round(s))\nGoal: {}\nWorker report:\n{report}\nOpen verifier findings:\n{findings}\n",
                task.spec["id"].as_str().unwrap_or_default(),
                task.spec["title"].as_str().unwrap_or_default(),
                task.status,
                task.round,
                task.spec["goal"].as_str().unwrap_or_default(),
            ));
        }
        let compliance_gaps = if cancel.load(Ordering::SeqCst) {
            vec!["compliance audit cancelled".to_owned()]
        } else {
            self.compliance_audit(conversation, plan_id, cancel)
                .unwrap_or_else(|error| vec![format!("compliance audit failed: {error}")])
        };
        let mut text = format!(
            "The plan finished. Results of each task follow. Answer the user's \
             request using them; verify anything doubtful in the files yourself.\n{results}"
        );
        if !cancel.load(Ordering::SeqCst) {
            let project = PathBuf::from(self.store.conversation(conversation)?.project);
            let project_text = project.to_string_lossy().into_owned();
            let mut audit = self.review(
                conversation,
                AUDITOR,
                Some(plan_id),
                &project,
                cancel,
                |first, request| {
                    if first {
                        tasks::audit_prompt(&project_text, request, &results)
                    } else {
                        tasks::audit_followup(request, &results)
                    }
                },
            )?;
            let has_unapproved =
                tasks.iter().any(|t| t.status != status::APPROVED) || !compliance_gaps.is_empty();
            if let Ok(ref mut verdict) = audit {
                for gap in &compliance_gaps {
                    verdict
                        .findings
                        .push(json!({"severity":"high", "issue":gap,"where":"requirement ledger"}));
                }
            }
            if has_unapproved
                && let Ok(ref mut verdict) = audit
                && verdict.approved
            {
                verdict.approved = false;
                let unapproved_ids: Vec<_> = tasks
                    .iter()
                    .filter(|t| t.status != status::APPROVED)
                    .map(|t| {
                        let id = t.spec["id"].as_str().unwrap_or(&t.id);
                        format!("{}: {}", id, t.status)
                    })
                    .collect();
                verdict.findings.push(serde_json::json!({
                    "severity": "high",
                    "issue": format!(
                        "Tasks in plan were not approved: {}",
                        unapproved_ids.join(", ")
                    ),
                    "where": "plan"
                }));
            }
            let mut payload = review_json(&audit);
            payload["plan"] = json!(plan_id);
            self.store
                .append_event(conversation, None, Some(plan_id), "audit", &payload)?;
            text.push_str(&match &audit {
                Ok(verdict) if verdict.approved => {
                    "\n## Independent audit\nThe request is fully delivered.\n".to_owned()
                }
                Ok(verdict) => format!(
                    "\n## Independent audit\nThe request is not fully delivered:\n{}\n\
                     Propose a plan for what is missing, or tell the user why it stays open.\n",
                    tasks::findings_text(&verdict.findings)
                ),
                Err(error) => format!(
                    "\n## Independent audit\nThe audit could not run ({error}); tell the \
                     user the result is unaudited.\n"
                ),
            });
        }
        if !compliance_gaps.is_empty() {
            text.push_str(&format!("\nRuntime requirements_complete gate is CLOSED. Do not claim success. Open obligations:\n{}\nPropose executor work for missing evidence; verifiers stay read-only.\n", compliance_gaps.join("\n")));
        }
        Ok(text)
    }
}
