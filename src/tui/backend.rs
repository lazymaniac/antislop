use super::model::Snapshot;
use crate::{
    Error, Result,
    ipc::{self, Operation, PROTOCOL_VERSION, Request},
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    ChatMessage {
        text: String,
    },
    Orchestrate {
        scope_hash: String,
    },
    RecoverOrchestration {
        round_id: String,
        kind: crate::runtime::orchestration_recovery::Kind,
        scope_hash: String,
    },
    AcceptIntake {
        round_id: String,
        scope_hash: String,
    },
    AcceptTeam {
        round_id: String,
        scope_hash: String,
    },
    StepTeam {
        scope_hash: String,
    },
    AcceptTeamStage {
        stage: crate::team::TaskStage,
        task_ids: Vec<String>,
        scope_hash: String,
    },
    ReconcileWaveEffects {
        task_ids: Vec<String>,
        scope_hash: String,
    },
    ReconcileTaskEffects {
        task_id: String,
        scope_hash: String,
    },
    RestoreTaskResult {
        task_id: String,
        scope_hash: String,
    },
    RestoreTaskCheckResult {
        invocation_id: String,
        scope_hash: String,
    },
    RefreshTaskSources {
        task_id: String,
        scope_hash: String,
    },
    RetryTaskVerification {
        task_id: String,
        scope_hash: String,
    },
    ResumeTaskSession {
        task_id: String,
        scope_hash: String,
    },
    ReplaceTaskSession {
        task_id: String,
        scope_hash: String,
    },
    RepairTask {
        task_id: String,
        plan: String,
        scope_hash: String,
    },
    SendMessage {
        target_agent_id: String,
        text: String,
    },
    Pause,
    Resume,
    Cancel,
    Advance,
    ApproveStart {
        scope_hash: String,
    },
    AcceptResult {
        scope_hash: String,
    },
    ApproveRepair {
        scope_hash: String,
    },
    Reconcile {
        scope_hash: String,
        input_hash: String,
    },
    ChangeMode {
        mode: String,
        expected_policy_revision: u64,
    },
    AddElementComment {
        target: crate::comments::ElementRef,
        text: String,
    },
    DeleteElementComment {
        comment_id: String,
    },
    ResolveElementComment {
        comment_id: String,
        resolved: bool,
    },
    AddDiffComment {
        path: String,
        line_number: Option<usize>,
        text: String,
        author: String,
    },
    DeleteDiffComment {
        comment_id: String,
    },
    ResolveDiffComment {
        comment_id: String,
        resolved: bool,
    },
    ReviewDiff {
        scope_hash: String,
    },
}
#[derive(Debug, Clone, Default)]
pub struct RunChoice {
    pub id: String,
    pub status: String,
    pub stage: String,
}
#[derive(Debug, Clone)]
pub struct ConversationStarted {
    pub run_id: String,
    pub notice: String,
    pub message_accepted: bool,
}
/// The UI emits validated control intents separately from natural language.
/// Implementations must preserve runtime authorization and delivery receipts.
pub trait Backend: Send + 'static {
    fn list_runs(&mut self) -> Result<Vec<RunChoice>>;
    fn snapshot(&mut self, run_id: &str) -> Result<Snapshot>;
    fn act(&mut self, run_id: &str, action: Action) -> Result<String>;
    fn create_run(&mut self, spec_path: &Path) -> Result<String>;
    fn begin_conversation(&mut self, text: &str) -> Result<ConversationStarted>;
}
pub struct IpcBackend {
    state: PathBuf,
    bootstrap_provider: Option<crate::config::ProviderKind>,
}
impl IpcBackend {
    pub fn new(state: &Path) -> Self {
        Self {
            state: state.into(),
            bootstrap_provider: None,
        }
    }
    pub fn with_bootstrap_provider(mut self, provider: crate::config::ProviderKind) -> Self {
        self.bootstrap_provider = Some(provider);
        self
    }
    pub fn begin_conversation_in(
        &mut self,
        text: &str,
        workspace: &Path,
    ) -> Result<ConversationStarted> {
        let project_config = workspace.join("architect.toml");
        let project_exists = match std::fs::symlink_metadata(&project_config) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        let config = if project_exists {
            crate::require(
                self.bootstrap_provider.is_none(),
                "--orchestrator applies only without architect.toml; configure the project's initial orchestrator there",
            )?;
            crate::require(
                project_config.is_file(),
                "architect.toml is not a readable regular file",
            )?;
            crate::config::Config::load(&project_config)?.pin()?
        } else {
            let kind = self
                .bootstrap_provider
                .unwrap_or(crate::config::ProviderKind::Codex);
            let name = serde_json::to_value(kind)?.as_str().unwrap().to_owned();
            crate::runtime::intake::bootstrap_for(kind, PathBuf::from(name))?
        };
        let data = self.call(
            Operation::Mutate {
                command: Box::new(crate::runtime::Command::BeginIntake {
                    objective: text.into(),
                    workspace: workspace.to_string_lossy().into(),
                    config,
                }),
            },
            true,
        )?;
        let id = data["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| Error::Invalid("daemon returned no intake ID".into()))?
            .to_owned();
        // Once created, always return the ID even if delivery/start fails. The UI
        // must attach to the retained conversation instead of creating it twice.
        let (notice, message_accepted) = match self
            .act(&id, Action::ChatMessage { text: text.into() })
        {
            Ok(notice) => (notice, true),
            Err(error) => (
                format!(
                    "Conversation created; message acceptance unconfirmed: {error}. Draft retained."
                ),
                false,
            ),
        };
        Ok(ConversationStarted {
            run_id: id,
            notice,
            message_accepted,
        })
    }

    fn call(&self, operation: Operation, mutation: bool) -> Result<Value> {
        if !self.state.join("architect.sock").exists() {
            return Err(Error::Invalid(format!(
                "Daemon unavailable. Start `antislop daemon {}` in another terminal.",
                self.state.display()
            )));
        }
        let response = ipc::call(
            &self.state,
            &Request {
                version: PROTOCOL_VERSION,
                request_id: crate::id(),
                idempotency_key: mutation.then(crate::id),
                operation,
            },
        )?;
        if let Some(error) = response.error {
            return Err(Error::Invalid(error));
        }
        response
            .data
            .ok_or_else(|| Error::Invalid("daemon returned no data".into()))
    }
}
impl Backend for IpcBackend {
    fn begin_conversation(&mut self, text: &str) -> Result<ConversationStarted> {
        self.begin_conversation_in(text, &std::env::current_dir()?)
    }
    fn create_run(&mut self, spec_path: &Path) -> Result<String> {
        let spec: crate::domain::RunSpec = crate::transport::decode(&std::fs::read(spec_path)?)?;
        spec.validate()?;
        let data = self.call(
            Operation::Mutate {
                command: Box::new(crate::runtime::Command::Create { spec }),
            },
            true,
        )?;
        data["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| Error::Invalid("daemon returned no created run ID".into()))
    }

    fn list_runs(&mut self) -> Result<Vec<RunChoice>> {
        let mut runs = vec![];
        let mut after = None;
        loop {
            let data = self.call(
                after
                    .as_ref()
                    .map_or(Operation::List, |after: &String| Operation::ListPage {
                        after: after.clone(),
                    }),
                false,
            )?;
            for run in data["runs"].as_array().into_iter().flatten() {
                runs.push(RunChoice {
                    id: run["id"].as_str().unwrap_or_default().into(),
                    status: run["status"].as_str().unwrap_or_default().into(),
                    stage: run["stage"].as_str().unwrap_or_default().into(),
                });
            }
            match data["next_cursor"].as_str() {
                Some(next) if after.as_deref() != Some(next) => after = Some(next.into()),
                None => break,
                _ => return Err(Error::Invalid("daemon repeated pagination cursor".into())),
            }
        }
        Ok(runs)
    }
    fn snapshot(&mut self, run_id: &str) -> Result<Snapshot> {
        let mut control = self.call(
            Operation::Snapshot {
                run_id: run_id.into(),
            },
            false,
        )?;
        // Hash-pinned chunk reads reject concurrent document changes; the worker
        // reports that error and tries again without blocking terminal input.
        let expected_hash = control["document_hash"].as_str().ok_or_else(|| {
            Error::Invalid("snapshot document hash unavailable; update daemon".into())
        })?;
        let text = ipc::read_document(&self.state, run_id, "run")?;
        crate::require(
            expected_hash == crate::hash(&text),
            "run changed between control snapshot and document read; refresh again",
        )?;
        let document: Value = serde_json::from_str(&text)?;
        crate::require(
            document["id"].as_str() == Some(run_id),
            "mismatched run document",
        )?;
        if control["team_stage_results"]
            .as_object()
            .is_some_and(|packages| !packages.is_empty())
        {
            let text = ipc::read_document(&self.state, run_id, "stage-results")?;
            let results: Value = serde_json::from_str(&text)?;
            crate::require(
                results["run_document_hash"] == control["document_hash"],
                "run changed before stage result read; refresh again",
            )?;
            let packages = results["packages"]
                .as_object()
                .ok_or_else(|| Error::Invalid("stage result document is malformed".into()))?;
            let summaries = control["team_stage_results"].as_object().unwrap();
            crate::require(
                packages.len() == summaries.len()
                    && packages.iter().all(|(key, package)| {
                        summaries.get(key).is_some_and(|summary| {
                            summary["stage"] == package["stage"]
                                && summary["scope_hash"] == package["scope_hash"]
                                && package["task_ids"].as_array().is_some_and(|tasks| {
                                    summary["task_count"].as_u64() == Some(tasks.len() as u64)
                                })
                        })
                    }),
                "stage result package changed since the control snapshot",
            )?;
            control["team_stage_results"] = results["packages"].clone();
        }
        if control["task_result_restore_count"]
            .as_u64()
            .is_some_and(|count| count > 0)
            || control["task_effect_reconciliation_count"]
                .as_u64()
                .is_some_and(|count| count > 0)
            || control["wave_effect_reconciliation_count"]
                .as_u64()
                .is_some_and(|count| count > 0)
            || control["task_session_resume_count"]
                .as_u64()
                .is_some_and(|count| count > 0)
            || control["task_session_replacement_count"]
                .as_u64()
                .is_some_and(|count| count > 0)
            || control["task_check_restore_count"]
                .as_u64()
                .is_some_and(|count| count > 0)
        {
            let text = ipc::read_document(&self.state, run_id, "task-recovery")?;
            let recovery: Value = serde_json::from_str(&text)?;
            crate::require(
                recovery["run_document_hash"] == control["document_hash"],
                "run changed before task recovery read; refresh again",
            )?;
            crate::require(
                recovery["result_restores"]
                    .as_object()
                    .is_some_and(|scopes| {
                        Some(scopes.len() as u64) == control["task_result_restore_count"].as_u64()
                    }),
                "task recovery package count changed",
            )?;
            crate::require(
                recovery["effect_reconciliations"]
                    .as_object()
                    .is_some_and(|scopes| {
                        Some(scopes.len() as u64)
                            == control["task_effect_reconciliation_count"].as_u64()
                    }),
                "effect reconciliation package count changed",
            )?;
            crate::require(
                recovery["wave_effect_reconciliations"]
                    .as_object()
                    .is_some_and(|waves| {
                        Some(waves.len() as u64)
                            == control["wave_effect_reconciliation_count"].as_u64()
                    }),
                "wave effect package count changed",
            )?;
            control["wave_effect_reconciliations"] =
                recovery["wave_effect_reconciliations"].clone();
            crate::require(
                recovery["native_resumes"].as_object().is_some_and(|p| {
                    Some(p.len() as u64) == control["task_session_resume_count"].as_u64()
                }),
                "native continuation package count changed",
            )?;
            control["task_session_resumes"] = recovery["native_resumes"].clone();
            crate::require(
                recovery["replacements"].as_object().is_some_and(|p| {
                    Some(p.len() as u64) == control["task_session_replacement_count"].as_u64()
                }),
                "session replacement package count changed",
            )?;
            control["task_session_replacements"] = recovery["replacements"].clone();
            crate::require(
                recovery["check_restores"].as_object().is_some_and(|p| {
                    Some(p.len() as u64) == control["task_check_restore_count"].as_u64()
                }),
                "task check recovery package count changed",
            )?;
            control["task_check_restores"] = recovery["check_restores"].clone();
            let captured: crate::domain::Run = serde_json::from_value(document.clone())?;
            for (id, value) in recovery["check_restores"].as_object().unwrap() {
                let package: crate::runtime::team::check_recovery::Pending =
                    serde_json::from_value(value.clone())?;
                package.validate_for_run(&captured)?;
                crate::require(
                    package.invocation_id == *id,
                    "captured task check package identity changed",
                )?;
            }
            for (id, value) in recovery["replacements"].as_object().unwrap() {
                let package: crate::runtime::team::replacement::Pending =
                    serde_json::from_value(value.clone())?;
                package.validate()?;
                let captured: crate::domain::Run = serde_json::from_value(document.clone())?;
                package.validate_for_run(&captured)?;
                crate::require(
                    package.task_id == *id,
                    "replacement task package identity changed",
                )?;
            }
            control["task_result_restores"] = recovery["result_restores"].clone();
            control["task_effect_reconciliations"] = recovery["effect_reconciliations"].clone();
        }
        if control["orchestration_recovery_count"]
            .as_u64()
            .is_some_and(|count| count > 0)
        {
            let text = ipc::read_document(&self.state, run_id, "orchestration-recovery")?;
            let recovery: Value = serde_json::from_str(&text)?;
            crate::require(
                recovery["run_document_hash"] == control["document_hash"],
                "management recovery snapshot changed",
            )?;
            crate::require(
                recovery["packages"].as_object().is_some_and(|p| {
                    Some(p.len() as u64) == control["orchestration_recovery_count"].as_u64()
                }),
                "management recovery package count changed",
            )?;
            control["orchestration_recoveries"] = recovery["packages"].clone();
            let captured: crate::domain::Run = serde_json::from_value(document.clone())?;
            for value in recovery["packages"].as_object().unwrap().values() {
                let package: crate::runtime::orchestration_recovery::Pending =
                    serde_json::from_value(value.clone())?;
                package.validate_for_run(&captured)?;
            }
        }
        let snapshot = Snapshot::from_document(&document, &control);
        Ok(snapshot)
    }
    fn act(&mut self, run_id: &str, action: Action) -> Result<String> {
        if let Action::AcceptIntake {
            round_id,
            scope_hash,
        } = &action
        {
            self.call(
                Operation::Mutate {
                    command: Box::new(crate::runtime::Command::AcceptIntake {
                        run_id: run_id.into(),
                        round_id: round_id.clone(),
                        scope_hash: scope_hash.clone(),
                    }),
                },
                true,
            )?;
            let planning = self.act(run_id, Action::ChatMessage { text:
                "The displayed task contract and routing have been approved. Propose the appropriate full team with predefined roles, dedicated verifiers, its task DAG and risk owners for this approved scope. This requests read-only planning and review, not task execution.".into() });
            return Ok(planning.unwrap_or_else(|e| format!("Contract accepted; team planning submission unconfirmed: {e}. Inspect status before retrying.")));
        }
        if let Action::ChatMessage { text } = action {
            self.act(
                run_id,
                Action::SendMessage {
                    target_agent_id: "orchestrator".into(),
                    text,
                },
            )?;
            let start = (|| {
                let run: crate::domain::Run = crate::transport::decode(
                    ipc::read_document(&self.state, run_id, "run")?.as_bytes(),
                )?;
                crate::require(run.id == run_id, "mismatched conversation document")?;
                self.act(
                    run_id,
                    Action::Orchestrate {
                        scope_hash: crate::orchestration::planning::start_scope(&run)?,
                    },
                )
            })();
            return Ok(match start {
                Ok(_) => "Message accepted. Orchestrator turn started; awaiting its response. Questions wait for your answer; complete proposals receive independent review.".into(),
                Err(e) => format!("Message queued; turn start unconfirmed: {e}. Check status; Ctrl+P can start a queued turn when ready."),
            });
        }
        let mut command = json!({"run_id":run_id});
        let kind = match &action {
            Action::ChatMessage { .. } | Action::AcceptIntake { .. } => {
                unreachable!("handled above")
            }
            Action::Orchestrate { scope_hash } => {
                command["scope_hash"] = json!(scope_hash);
                "orchestrate"
            }
            Action::RecoverOrchestration {
                round_id,
                kind,
                scope_hash,
            } => {
                command["round_id"] = json!(round_id);
                command["kind"] = json!(kind);
                command["scope_hash"] = json!(scope_hash);
                "recover_orchestration"
            }
            Action::AcceptTeam {
                round_id,
                scope_hash,
            } => {
                command["round_id"] = json!(round_id);
                command["scope_hash"] = json!(scope_hash);
                "accept_team"
            }
            Action::AcceptTeamStage {
                stage,
                task_ids,
                scope_hash,
            } => {
                command["stage"] = json!(stage);
                command["task_ids"] = json!(task_ids);
                command["scope_hash"] = json!(scope_hash);
                "accept_team_stage"
            }
            Action::StepTeam { scope_hash } => {
                command["scope_hash"] = json!(scope_hash);
                "step_team"
            }
            Action::ReconcileWaveEffects {
                task_ids,
                scope_hash,
            } => {
                command["task_ids"] = json!(task_ids);
                command["scope_hash"] = json!(scope_hash);
                "reconcile_wave_effects"
            }
            Action::ReconcileTaskEffects {
                task_id,
                scope_hash,
            } => {
                command["task_id"] = json!(task_id);
                command["scope_hash"] = json!(scope_hash);
                "reconcile_task_effects"
            }
            Action::RestoreTaskResult {
                task_id,
                scope_hash,
            } => {
                command["task_id"] = json!(task_id);
                command["scope_hash"] = json!(scope_hash);
                "restore_task_result"
            }
            Action::RestoreTaskCheckResult {
                invocation_id,
                scope_hash,
            } => {
                command["invocation_id"] = json!(invocation_id);
                command["scope_hash"] = json!(scope_hash);
                "restore_task_check_result"
            }
            Action::RefreshTaskSources {
                task_id,
                scope_hash,
            } => {
                command["task_id"] = json!(task_id);
                command["scope_hash"] = json!(scope_hash);
                "refresh_task_sources"
            }
            Action::RetryTaskVerification {
                task_id,
                scope_hash,
            } => {
                command["task_id"] = json!(task_id);
                command["scope_hash"] = json!(scope_hash);
                "retry_task_verification"
            }
            Action::ResumeTaskSession {
                task_id,
                scope_hash,
            } => {
                command["task_id"] = json!(task_id);
                command["scope_hash"] = json!(scope_hash);
                "resume_task_session"
            }
            Action::ReplaceTaskSession {
                task_id,
                scope_hash,
            } => {
                command["task_id"] = json!(task_id);
                command["scope_hash"] = json!(scope_hash);
                "replace_task_session"
            }
            Action::RepairTask {
                task_id,
                plan,
                scope_hash,
            } => {
                command["task_id"] = json!(task_id);
                command["plan"] = json!(plan);
                command["scope_hash"] = json!(scope_hash);
                "repair_task"
            }
            Action::SendMessage {
                target_agent_id,
                text,
            } => {
                command["target_agent_id"] = json!(target_agent_id);
                command["text"] = json!(text);
                "send_message"
            }
            Action::AddElementComment { target, text } => {
                command["target"] = json!(target);
                command["text"] = json!(text);
                "add_element_comment"
            }
            Action::DeleteElementComment { comment_id } => {
                command["comment_id"] = json!(comment_id);
                "delete_element_comment"
            }
            Action::ResolveElementComment {
                comment_id,
                resolved,
            } => {
                command["comment_id"] = json!(comment_id);
                command["resolved"] = json!(resolved);
                "resolve_element_comment"
            }
            Action::Pause => "pause",
            Action::Resume => "resume",
            Action::Cancel => "cancel",
            Action::Advance => "advance",
            Action::ApproveStart { scope_hash } => {
                command["scope_hash"] = json!(scope_hash);
                "approve_start"
            }
            Action::AcceptResult { scope_hash } => {
                command["scope_hash"] = json!(scope_hash);
                "accept_result"
            }
            Action::ApproveRepair { scope_hash } => {
                command["scope_hash"] = json!(scope_hash);
                "approve_repair"
            }
            Action::Reconcile {
                scope_hash,
                input_hash,
            } => {
                command["scope_hash"] = json!(scope_hash);
                command["input_hash"] = json!(input_hash);
                "reconcile"
            }
            Action::ChangeMode {
                mode,
                expected_policy_revision,
            } => {
                command["mode"] = json!(mode);
                command["expected_policy_revision"] = json!(expected_policy_revision);
                "change_mode"
            }
            Action::AddDiffComment {
                path,
                line_number,
                text,
                author,
            } => {
                command["path"] = json!(path);
                command["line_number"] = json!(line_number);
                command["text"] = json!(text);
                command["author"] = json!(author);
                "add_diff_comment"
            }
            Action::DeleteDiffComment { comment_id } => {
                command["comment_id"] = json!(comment_id);
                "delete_diff_comment"
            }
            Action::ResolveDiffComment {
                comment_id,
                resolved,
            } => {
                command["comment_id"] = json!(comment_id);
                command["resolved"] = json!(resolved);
                "resolve_diff_comment"
            }
            Action::ReviewDiff { scope_hash } => {
                command["scope_hash"] = json!(scope_hash);
                "review_diff"
            }
        };
        command["type"] = json!(kind);
        let command = serde_json::from_value(command)?;
        self.call(
            Operation::Mutate {
                command: Box::new(command),
            },
            true,
        )?;
        Ok(match action {
            Action::SendMessage { .. } => {
                "Message queued. Ctrl+P > Run queued orchestrator turn starts a reply (configured runs). Delivery requires a receipt.".into()
            }
            _ => "Runtime accepted control action.".into(),
        })
    }
}
