use super::*;

struct FinishLease<'a> {
    core: &'a Core,
    conversation: &'a str,
}

impl Drop for FinishLease<'_> {
    fn drop(&mut self) {
        self.core
            .activity
            .finishing
            .lock()
            .expect("session finish mutex")
            .remove(self.conversation);
    }
}

impl Core {
    /// Read a completed result without recreating its disposable checkout.
    pub(in crate::lean) fn session_result_fingerprint(
        &self,
        conversation: &str,
        project: &Path,
    ) -> Result<String> {
        if !self.session_branch_path(conversation).try_exists()? {
            return git::integrated_fingerprint(&self.session_worktree(conversation, project)?);
        }
        let record = self.load_session_branch(conversation)?;
        self.validate_session_record(conversation, &project.canonicalize()?, &record)?;
        if record.worktree.exists() {
            require(
                !std::fs::symlink_metadata(&record.worktree)?
                    .file_type()
                    .is_symlink(),
                "session checkout is a symlink; preserved",
            )?;
            self.ensure_session_worktree(conversation, &project.canonicalize()?, &record)?;
            git::integrated_fingerprint(&record.worktree)
        } else {
            git::ok(
                project,
                &[
                    "rev-parse",
                    "--verify",
                    &format!("refs/heads/{}^{{tree}}", record.branch),
                ],
            )
        }
    }

    /// Streaming clients stay until the final branch and cleanup summary are recorded.
    pub(in crate::lean) fn frontend_idle(&self, conversation: &str) -> Result<bool> {
        if !self.is_idle(conversation) {
            return Ok(false);
        }
        Ok(self
            .pending_session_completion(&self.store.conversation(conversation)?)?
            .is_none())
    }

    pub(in crate::lean) fn finish_idle_sessions(self: &Arc<Self>) {
        let conversations = match self.store.conversations() {
            Ok(rows) => rows,
            Err(error) => {
                eprintln!("antislop: could not list sessions for cleanup: {error}");
                return;
            }
        };
        for conversation in conversations {
            if !self.is_idle(&conversation.id) {
                continue;
            }
            match self.pending_session_completion(&conversation) {
                Ok(Some(_)) => {}
                Ok(None) => continue,
                Err(error) => {
                    eprintln!("antislop: could not inspect session completion: {error}");
                    continue;
                }
            }
            let result = {
                // Match plan publication and deletion: plans precede the lifecycle fence.
                let plans = self.activity.plans.lock().expect("plans mutex");
                let lifecycle = self
                    .locks
                    .session_lifecycle
                    .lock()
                    .expect("session lifecycle mutex");
                if !self.is_idle_with_plans(&conversation.id, &plans) {
                    continue;
                }
                self.activity
                    .finishing
                    .lock()
                    .expect("session finish mutex")
                    .insert(conversation.id.clone());
                let lease = FinishLease {
                    core: self,
                    conversation: &conversation.id,
                };
                drop(lifecycle);
                drop(plans);
                let result = self.finish_session_result(&conversation);
                drop(lease);
                result
            };
            if let Err(error) = result {
                let text = format!("Session cleanup is pending: {error}");
                let previous: Option<String> = self.store.db().query_row(
                    "SELECT json_extract(payload,'$.text') FROM events WHERE conversation_id=?1 AND kind='session_cleanup_pending' ORDER BY seq DESC LIMIT 1",
                    [&conversation.id], |row| row.get(0)).optional().ok().flatten();
                if previous.as_deref() != Some(&text) {
                    let _ = self.store.append_event(
                        &conversation.id,
                        None,
                        None,
                        "session_cleanup_pending",
                        &json!({"text":text}),
                    );
                }
            }
            // Comments arriving during cleanup remain queued and wake afterwards.
            if self.unanswered(&conversation.id).ok().flatten().is_some() {
                self.wake(&conversation.id);
            }
        }
    }

    fn pending_session_completion(
        &self,
        conversation: &Conversation,
    ) -> Result<Option<(String, String)>> {
        if self.store.paused(&conversation.id)?
            || self.store.workflow(&conversation.id)?.phase != "implementation"
        {
            return Ok(None);
        }
        let Some(audit) = self.store.latest_audit_event(&conversation.id)? else {
            return Ok(None);
        };
        if audit.payload["approved"] != true {
            return Ok(None);
        }
        let plan: Option<String> = self.store.db().query_row(
            "SELECT plan_id FROM tasks WHERE conversation_id=?1 ORDER BY created_ms DESC,rowid DESC LIMIT 1",
            [&conversation.id], |row| row.get(0)).optional()?;
        let Some(plan) = plan.filter(|plan| audit.payload["plan"] == *plan) else {
            return Ok(None);
        };
        let key = format!(
            "{}:{}:{}",
            plan,
            self.store.plan_revision(&plan)?,
            audit.seq
        );
        let completed: Option<String> = self.store.db().query_row(
            "SELECT json_extract(payload,'$.key') FROM events WHERE conversation_id=?1 AND kind='session_completed' ORDER BY seq DESC LIMIT 1",
            [&conversation.id], |row| row.get(0)).optional()?;
        if completed.as_deref() == Some(key.as_str()) {
            return Ok(None);
        }
        let tasks = self.store.plan(&plan)?;
        if tasks.is_empty()
            || tasks
                .iter()
                .any(|task| task.status != status::APPROVED || task.spec["deferred"] == true)
            || self.store.pending_scope_revision(&plan)?.is_some()
            || self
                .store
                .open_user_decision(&conversation.id)?
                .flatten()
                .is_some()
            || self.unanswered(&conversation.id)?.is_some()
        {
            return Ok(None);
        }
        // Content lives outside SQLite: only message sequence metadata establishes the handoff.
        let answered: bool = self.store.db().query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE conversation_id=?1 AND author='orchestrator' AND seq>coalesce((SELECT max(seq) FROM messages WHERE conversation_id=?1 AND author='runtime'),9223372036854775807))",
            [&conversation.id], |row| row.get(0))?;
        if !answered {
            return Ok(None);
        }
        Ok(Some((plan, key)))
    }

    fn finish_session_result(&self, conversation: &Conversation) -> Result<()> {
        let Some((plan, key)) = self.pending_session_completion(conversation)? else {
            return Ok(());
        };
        require(
            self.final_audit_current(&conversation.id, &plan)?,
            "the final audit no longer matches the current result; review current changes before finalizing",
        )?;
        let finalizing: bool = self.store.db().query_row(
            "SELECT EXISTS(SELECT 1 FROM events WHERE conversation_id=?1 AND kind='session_finalizing' AND json_extract(payload,'$.key')=?2)",
            rusqlite::params![conversation.id,key], |row| row.get(0))?;
        if !finalizing {
            let branch = if git::is_repository(Path::new(&conversation.project)) {
                Some(self.load_session_branch(&conversation.id)?.branch)
            } else {
                None
            };
            self.store.append_event(
                &conversation.id,
                None,
                Some(&plan),
                "session_finalizing",
                &json!({"key":key,"branch":branch,"project":conversation.project}),
            )?;
        }
        if !self.native.retire_conversation(&conversation.id)? {
            return Ok(());
        }
        let all_tasks = {
            let ids = self
                .store
                .db()
                .prepare("SELECT id FROM tasks WHERE conversation_id=?1")?
                .query_map([&conversation.id], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            ids.iter()
                .map(|id| {
                    self.store
                        .task_by_id(id)?
                        .ok_or_else(|| Error::Invalid("session task disappeared".into()))
                })
                .collect::<Result<Vec<_>>>()?
        };
        let project = PathBuf::from(&conversation.project);
        let completed_tasks = self.store.plan(&plan)?;
        let summary = self.session_completion_summary(&completed_tasks)?;
        let (branch, commit) = if git::is_repository(&project) {
            let record = self.load_session_branch(&conversation.id)?;
            self.validate_session_record(&conversation.id, &project.canonicalize()?, &record)?;
            let commit = self.preserve_final_result(&record, &all_tasks)?;
            if !self.release_session_checkouts(&conversation.id, &project, &all_tasks)? {
                return Ok(());
            }
            if record.worktree.exists() {
                require(
                    !self.store.paused(&conversation.id)?,
                    "session cleanup is paused; continue to resume cleanup",
                )?;
                let Some(_lease) = self.try_lease_worktree(&record.worktree) else {
                    return Ok(());
                };
                require(
                    git::worktree_branch(&record.worktree)? == record.branch,
                    "session worktree branch changed; preserved",
                )?;
                git::ok(
                    &project,
                    &[
                        "worktree",
                        "remove",
                        "--force",
                        &record.worktree.to_string_lossy(),
                    ],
                )?;
            }
            git::prune_worktrees(&project)?;
            (Some(record.branch), Some(commit))
        } else {
            (None, None)
        };
        self.store.append_event(&conversation.id, None, Some(&plan), "session_completed", &json!({
            "key":key,"project":conversation.project,"branch":branch,"commit":commit,"summary":summary,"cleanup_complete":true
        }))?;
        Ok(())
    }

    fn session_completion_summary(&self, tasks: &[Task]) -> Result<String> {
        let mut summaries = Vec::with_capacity(tasks.len());
        for task in tasks {
            let summary = match self.store.task_summary(task) {
                Ok(summary) if !summary.trim().is_empty() => summary,
                Ok(_) => task.report.clone().unwrap_or_default(),
                Err(Error::Invalid(message))
                    if message == "completed worker result has no summary" =>
                {
                    task.report.clone().unwrap_or_default()
                }
                Err(Error::Sql(rusqlite::Error::QueryReturnedNoRows)) => {
                    task.report.clone().unwrap_or_default()
                }
                Err(error) => return Err(error),
            };
            let summary = if summary.trim().is_empty() {
                "Summary unavailable from the completed task report.".to_owned()
            } else {
                crate::lean::tasks::bound(summary.trim(), 500)
            };
            let title = task.spec["title"].as_str().unwrap_or(&task.id);
            summaries.push(format!("{title}: {summary}"));
        }
        Ok(crate::lean::tasks::bound(&summaries.join("\n"), 2_000))
    }

    fn preserve_final_result(&self, record: &SessionBranch, tasks: &[Task]) -> Result<String> {
        let project = Path::new(&record.project);
        if record.worktree.exists() {
            require(
                !std::fs::symlink_metadata(&record.worktree)?
                    .file_type()
                    .is_symlink(),
                "session checkout is a symlink; preserved",
            )?;
            require(
                git::worktree_branch(&record.worktree)? == record.branch,
                "session checkout branch changed; preserved",
            )?;
            let outputs = tasks
                .iter()
                .filter(|task| task.spec["delivery"] != "untracked")
                .flat_map(|task| task.spec["outputs"].as_array().into_iter().flatten())
                .filter_map(|path| path.as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            let head = git::head(&record.worktree)?;
            let snapshot = git::snapshot_with_artifacts(
                &record.worktree,
                &outputs,
                git::CommitPurpose::TaskResult,
            )?;
            let tree = git::tree_identity(&record.worktree, &snapshot)?;
            if tree != git::tree_identity(&record.worktree, &head)? {
                let commit = git::ok(
                    &record.worktree,
                    &[
                        "commit-tree",
                        &tree,
                        "-p",
                        &head,
                        "-m",
                        "chore: finalize session results",
                    ],
                )?;
                git::ok(
                    project,
                    &[
                        "update-ref",
                        &format!("refs/heads/{}", record.branch),
                        &commit,
                        &head,
                    ],
                )?;
                return Ok(commit);
            }
        }
        git::ok(
            project,
            &[
                "rev-parse",
                "--verify",
                &format!("refs/heads/{}", record.branch),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_preserves_the_final_branch_and_releases_only_its_owned_resources() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir(&project).unwrap();
        git::ok(&project, &["init", "-q", "-b", "main"]).unwrap();
        std::fs::write(project.join("source.txt"), "original").unwrap();
        let original = git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
        let config = Config::from_toml(include_str!("../../../../examples/antislop.toml"));
        let config = config.unwrap();
        let core = Core::open(&directory.path().join("state"), config).unwrap();
        let conversation = core.start(&project).unwrap();
        let foreign = core.start(&project).unwrap();
        core.activity
            .finishing
            .lock()
            .unwrap()
            .insert(conversation.id.clone());
        core.session_worktree(&conversation.id, &project).unwrap();
        core.session_worktree(&foreign.id, &project).unwrap();
        let result = core.load_session_branch(&conversation.id).unwrap();
        let foreign_record = core.load_session_branch(&foreign.id).unwrap();
        let spec = json!({"id":"write","title":"Write result","goal":"Write result","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"The result is written"}]});
        let mut retired = core
            .store
            .add_plan(&conversation.id, std::slice::from_ref(&spec))
            .unwrap()
            .remove(0);
        retired.status = "superseded".into();
        core.store.update_task(&retired).unwrap();
        let mut task = core
            .store
            .add_plan(&conversation.id, &[spec])
            .unwrap()
            .remove(0);
        task.status = status::APPROVED.into();
        task.report = Some("Wrote result.txt with the completed result.".into());
        core.store.update_task(&task).unwrap();
        core.store
            .test_completion(&task, task.report.as_deref().unwrap());
        let scope = core
            .store
            .create_scope(&conversation.id, "Write result")
            .unwrap();
        core.store
            .bind_plan_scope(&conversation.id, &task.plan_id, &scope.id)
            .unwrap();
        std::fs::write(result.worktree.join("result.txt"), "complete result").unwrap();
        let task_path = core.state.join("worktrees").join(&retired.id);
        let task_branch = format!("antislop/{}", retired.id);
        git::add_worktree(&project, &task_path, &task_branch, &original).unwrap();
        std::fs::write(task_path.join("unfinished.txt"), "retained checkpoint").unwrap();
        let orphan = format!("antislop/{}-g0", retired.id);
        git::ok(&project, &["branch", &orphan, &original]).unwrap();
        let scratch = crate::lean::core::plan::helpers::writer_base_scratch(
            &core.state,
            &task.plan_id,
            "write",
        );
        let scratch_branch = format!(
            "antislop/combine-{}",
            scratch.file_name().unwrap().to_string_lossy()
        );
        git::add_worktree(&project, &scratch, &scratch_branch, &original).unwrap();
        let evidence = crate::lean::core::task::runtime::directory(&core.state, &task)
            .unwrap()
            .join("measurements.txt");
        std::fs::write(&evidence, "retained evidence").unwrap();
        let binding = core
            .final_audit_binding(&conversation.id, &task.plan_id)
            .unwrap();
        core.store
            .append_event(
                &conversation.id,
                None,
                Some(&task.plan_id),
                "audit",
                &json!({"plan":task.plan_id,"approved":true,"findings":[],"binding":binding}),
            )
            .unwrap();
        let report = core
            .store
            .add_message(&conversation.id, RUNTIME, "## Independent audit\nApproved")
            .unwrap();
        let turn = core
            .store
            .start_turn(
                &conversation.id,
                ORCHESTRATOR,
                "fixture",
                report.seq,
                Some(&task.plan_id),
            )
            .unwrap();
        core.store
            .finish_turn(&turn.id, TurnStatus::Done, None)
            .unwrap();
        core.store
            .add_message(
                &conversation.id,
                ORCHESTRATOR,
                "audit status: complete - all requirements verified",
            )
            .unwrap();
        assert!(
            core.pending_session_completion(&conversation)
                .unwrap()
                .is_none(),
            "a new interview is not a completed execution"
        );
        core.store
            .db()
            .execute(
                "UPDATE session_workflow SET phase='implementation' WHERE conversation_id=?1",
                [&conversation.id],
            )
            .unwrap();
        core.cancel(&conversation.id);
        core.finish_session_result(&conversation).unwrap();
        assert!(task_path.exists(), "cancellation pauses cleanup");
        core.pause(&conversation.id, false).unwrap();
        assert!(
            core.pending_session_completion(&conversation)
                .unwrap()
                .is_some(),
            "pending messages: {:?}",
            core.store.messages_after(&conversation.id, 0).unwrap()
        );
        assert!(
            core.final_audit_current(&conversation.id, &task.plan_id)
                .unwrap()
        );
        let lease = core.try_lease_worktree(&task_path).unwrap();
        core.finish_session_result(&conversation).unwrap();
        assert!(task_path.exists());
        assert!(
            !core
                .store
                .events_since(&conversation.id, 0, 1000)
                .unwrap()
                .iter()
                .any(|event| event.kind == "session_completed")
        );
        drop(lease);
        assert!(
            core.pending_session_completion(&conversation)
                .unwrap()
                .is_some()
        );
        assert!(
            core.final_audit_current(&conversation.id, &task.plan_id)
                .unwrap(),
            "finalizing the same tree must preserve the audit binding"
        );
        core.finish_session_result(&conversation).unwrap();
        assert!(!result.worktree.exists());
        assert!(!task_path.exists());
        assert!(!scratch.exists());
        assert!(foreign_record.worktree.exists());
        assert_eq!(
            git::ok(
                &project,
                &["show", &format!("{}:result.txt", result.branch)]
            )
            .unwrap(),
            "complete result"
        );
        assert_eq!(git::head(&project).unwrap(), original);
        assert!(!project.join("result.txt").exists());
        assert_eq!(git::ok(&project, &["status", "--porcelain"]).unwrap(), "");
        let branches = git::branches(&project, "antislop/").unwrap();
        assert_eq!(branches.len(), 2);
        assert!(branches.contains(&result.branch));
        assert!(branches.contains(&foreign_record.branch));
        assert_eq!(
            std::fs::read_to_string(&evidence).unwrap(),
            "retained evidence"
        );
        let history = format!(
            "refs/antislop/view-history/{}",
            crate::hash(task_path.as_os_str().as_encoded_bytes())
        );
        assert_eq!(
            git::ok(&project, &["show", &format!("{history}:unfinished.txt")]).unwrap(),
            "retained checkpoint"
        );
        assert!(
            core.final_audit_current(&conversation.id, &task.plan_id)
                .unwrap()
        );
        assert!(
            !result.worktree.exists(),
            "reading completion status must not recreate its checkout"
        );
        core.finish_session_result(&conversation).unwrap();
        let events = core.store.events_since(&conversation.id, 0, 1000).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == "session_completed")
                .count(),
            1
        );
        let completed = events
            .iter()
            .find(|event| event.kind == "session_completed")
            .unwrap();
        assert_eq!(completed.payload["branch"], result.branch);
        assert!(
            completed.payload["summary"]
                .as_str()
                .unwrap()
                .contains("Wrote result.txt with the completed result.")
        );
        let mut view = crate::lean::tui::view::View::default();
        for event in events {
            view.apply(&serde_json::to_value(event).unwrap());
        }
        assert!(view.cards.iter().any(|card|matches!(card,crate::lean::tui::TranscriptCard::System{title,text,..}
            if title=="Session completed" && text.contains(&result.branch) && text.contains("git switch"))));
        core.activity
            .finishing
            .lock()
            .unwrap()
            .remove(&conversation.id);
        assert!(
            core.frontend_idle(&conversation.id).unwrap(),
            "streaming clients can exit after the completion summary"
        );
    }
}
