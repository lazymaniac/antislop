//! Explicit user review requests use normal review context and durable delivery.
use super::*;

impl Core {
    pub(in crate::lean) fn resume_agent_reviews(self: &Arc<Self>, conversation: &str) {
        let mut jobs = self.agent_reviews.lock().expect("agent reviews mutex");
        if jobs.contains_key(conversation) {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        jobs.insert(conversation.into(), cancel.clone());
        drop(jobs);
        let core = self.clone();
        let conversation = conversation.to_owned();
        std::thread::spawn(move || {
            loop {
                let role = {
                    let mut jobs = core.agent_reviews.lock().expect("agent reviews mutex");
                    let role = [PLAN_VERIFIER, AUDITOR].into_iter().find(|role| {
                        core.store
                            .pending_mail(&conversation, role)
                            .is_ok_and(|m| !m.is_empty())
                    });
                    if role.is_none() || cancel.load(Ordering::SeqCst) {
                        jobs.remove(&conversation);
                        return;
                    }
                    role.expect("checked role")
                };
                let result: Result<()> = (|| {
                    let tasks = core.store.latest_plan(&conversation)?;
                    let plan = tasks.first().map(|t| t.plan_id.as_str());
                    let project = PathBuf::from(core.store.conversation(&conversation)?.project);
                    if role == AUDITOR {
                        if let Some(plan) = plan {
                            core.plan_report(&conversation, plan, &cancel)?;
                        } else {
                            core.review(
                                &conversation,
                                role,
                                None,
                                &project,
                                &cancel,
                                |_, request| {
                                    tasks::audit_prompt(
                                        &project.to_string_lossy(),
                                        request,
                                        "No plan exists.",
                                    )
                                },
                            )?
                            .map_err(Error::Invalid)?;
                        }
                    } else {
                        let specs = tasks
                            .iter()
                            .map(|t| serde_json::from_value(t.spec.clone()))
                            .collect::<std::result::Result<Vec<TaskContract>, _>>()?;
                        core.review(
                            &conversation,
                            role,
                            plan,
                            &project,
                            &cancel,
                            |initial, request| {
                                if initial {
                                    tasks::plan_review_prompt(
                                        &project.to_string_lossy(),
                                        request,
                                        &specs,
                                    )
                                } else {
                                    tasks::plan_review_followup(request, &specs)
                                }
                            },
                        )?
                        .map_err(Error::Invalid)?;
                    }
                    Ok(())
                })();
                let result = result.and_then(|()| {
                    require(
                        core.store.pending_mail(&conversation, role)?.is_empty(),
                        "review did not consume the request; automatic retry disabled",
                    )
                });
                if let Err(error) = result {
                    core.notice(&conversation, &format!("{role}: {error}"));
                    core.agent_reviews
                        .lock()
                        .expect("agent reviews mutex")
                        .remove(&conversation);
                    return;
                }
            }
        });
    }
}
