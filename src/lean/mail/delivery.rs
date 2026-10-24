use super::*;

impl Core {
    pub(in crate::lean) fn answer_waiting(
        self: &Arc<Self>,
        conversation: &str,
        _plan_id: &str,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let drain = self.turn_lock(conversation, "__mail_drain");
        let Some(_drain) = lock_cancellable(&drain, cancel) else {
            return Ok(());
        };
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        for target in self.ready_mail(conversation)? {
            let task = target
                .task_id
                .as_deref()
                .map(|id| self.store.task_by_id(id))
                .transpose()?
                .flatten();
            let spec = task
                .as_ref()
                .map(|task| serde_json::from_value::<TaskSpec>(task.spec.clone()))
                .transpose()?;
            let (kind, role_kind, sender, title) = if let Some(spec) = &spec {
                let verifier =
                    target.role.starts_with("verifier:") || target.role.starts_with("code-review:");
                (
                    spec.kind,
                    if verifier {
                        RoleKind::Verifier
                    } else {
                        RoleKind::Executor
                    },
                    if target.role.starts_with("code-review:") {
                        format!("{}/review", spec.id)
                    } else if verifier {
                        format!("{}/verifier", spec.id)
                    } else {
                        spec.id.clone()
                    },
                    spec.title.clone(),
                )
            } else {
                (
                    WorkKind::Orchestration,
                    if matches!(target.role.as_str(), ORCHESTRATOR | COMMENT_ORCHESTRATOR) {
                        RoleKind::Orchestrator
                    } else {
                        RoleKind::Verifier
                    },
                    target.role.clone(),
                    target.role.clone(),
                )
            };
            for _ in 0..MAIL_TURNS {
                if cancel.load(Ordering::SeqCst)
                    || self
                        .store
                        .pending_mail(conversation, &target.role)?
                        .is_empty()
                {
                    break;
                }
                let mut completed = false;
                for attempt in 0..DELIVERY_ATTEMPTS {
                    let setup = TurnSetup {
                        session_policy: match task
                            .as_ref()
                            .filter(|_| task_role(&target.role).is_some())
                        {
                            Some(task) => SessionPolicy::TaskRound {
                                task: task.id.clone(),
                                round: task.round,
                            },
                            None => SessionPolicy::Role,
                        },
                        conversation,
                        plan_id: target.plan_id.clone(),
                        role: target.role.clone(),
                        profile: self.config_snapshot().resolve(kind, role_kind),
                        cwd: project.clone(),
                        add_dirs: vec![],
                        access: agent::Access::ReadOnly,
                        upto_message: 0,
                    };
                    let direct = self
                        .store
                        .pending_mail(conversation, &target.role)?
                        .iter()
                        .any(|mail| mail.kind == "agent_work");
                    let context = if direct
                        && matches!(target.role.as_str(), AUDITOR | PLAN_VERIFIER)
                    {
                        let plan = self.store.latest_plan(conversation)?;
                        if target.role == PLAN_VERIFIER {
                            let specs = plan
                                .iter()
                                .map(|task| serde_json::from_value(task.spec.clone()))
                                .collect::<std::result::Result<Vec<TaskSpec>, _>>()?;
                            tasks::plan_review_prompt(
                                &project.to_string_lossy(),
                                "Review the current plan again, following the user's direct instructions.",
                                &specs,
                            )
                        } else {
                            let results = plan
                                .iter()
                                .map(|task| {
                                    format!(
                                        "{} · {}\n{}",
                                        task.spec["id"],
                                        task.status,
                                        task.report.as_deref().unwrap_or("(no report)")
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("\n\n");
                            tasks::audit_prompt(
                                &project.to_string_lossy(),
                                "Audit the current result again, following the user's direct instructions.",
                                &results,
                            )
                        }
                    } else {
                        String::new()
                    };
                    let answer = self.turn_inner(setup, cancel, |first| {
                        if direct && !context.is_empty() { context.clone() } else {
                            format!("Mailbox recipient: {}. This is a receipt-only turn; do not execute work, approve tasks or propose plans.\n{}",
                            target.role, tasks::answer_prompt(&title, first))
                        }
                    }, true)?;
                    if let Some(answer) = answer {
                        self.post(conversation, target.plan_id.as_deref(), &sender, &answer)?;
                        self.mail_failures
                            .lock()
                            .expect("mail failures mutex")
                            .remove(&(conversation.to_owned(), target.role.clone()));
                        completed = true;
                        break;
                    }
                    if cancel.load(Ordering::SeqCst)
                        || self
                            .store
                            .pending_mail(conversation, &target.role)?
                            .is_empty()
                    {
                        break;
                    }
                    let error = self
                        .store
                        .last_turn(conversation, &target.role)?
                        .and_then(|turn| turn.error)
                        .unwrap_or_else(|| "recipient turn ended without acknowledgement".into());
                    let retry = attempt + 1 < DELIVERY_ATTEMPTS && retryable(&error);
                    self.store.append_event(conversation, None, target.plan_id.as_deref(),
                        if retry { "mail_retry" } else { "mail_delivery_failed" },
                        &json!({"recipient": target.role, "attempt": attempt + 1, "reason": error, "state": "queued"}))?;
                    if !retry || !retry_delay(attempt, cancel) {
                        break;
                    }
                }
                if !completed {
                    self.mail_failures
                        .lock()
                        .expect("mail failures mutex")
                        .insert(
                            (conversation.to_owned(), target.role.clone()),
                            target.newest,
                        );
                    break;
                }
            }
        }
        Ok(())
    }
}
