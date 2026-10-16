use super::model::Snapshot;
use crate::{
    Error, Result,
    ipc::{self, Operation, PROTOCOL_VERSION, Request},
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    InspectDelivery,
    DeliverSource {
        scope_hash: String,
    },
    ReconcileSourceDelivery {
        scope_hash: String,
        observation_hash: String,
    },
    RestoreSourceDeliveryBackup {
        path: String,
        scope_hash: String,
    },
    InspectWorkspace,
    PrepareWorkspace {
        selected: std::collections::BTreeSet<String>,
        scope_hash: String,
    },
    ActivateWorkspace {
        scope_hash: String,
    },
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
    decoded_run: Option<Arc<DecodedRun>>,
}
/// Presentation data for one exact downloaded document. Reuse requires the
/// daemon's fresh verification of the complete actual database bytes, not just
/// stored hash metadata. Never serialized or used to authorize a mutation.
struct DecodedRun {
    text: String,
    hash: String,
    document: Value,
    captured: OnceLock<crate::domain::Run>,
    management: OnceLock<crate::runtime::orchestration_recovery::authenticated::Set>,
}
impl DecodedRun {
    fn captured(&self) -> Result<&crate::domain::Run> {
        if self.captured.get().is_none() {
            let run = serde_json::from_str(&self.text)?;
            let _ = self.captured.set(run);
        }
        Ok(self.captured.get().expect("decoded run initialized"))
    }
    fn validate_management(
        &self,
        packages: &std::collections::BTreeMap<
            String,
            crate::runtime::orchestration_recovery::Pending,
        >,
    ) -> Result<()> {
        let run = self.captured()?;
        // The proof belongs to this immutable decoded document, not a mutable
        // backend slot. Replacing any original byte constructs a new owner and
        // discards this proof. Package JSON alone never initializes the cell.
        if let Some(proof) = self.management.get()
            && proof.packages_match(packages)?
        {
            return crate::runtime::orchestration_recovery::sources_current(run);
        }
        let proof = crate::runtime::orchestration_recovery::authenticated::Set::build_for_packages(
            run, packages,
        )?;
        let _ = self.management.set(proof);
        Ok(())
    }
}
impl IpcBackend {
    pub fn new(state: &Path) -> Self {
        Self {
            state: state.into(),
            bootstrap_provider: None,
            decoded_run: None,
        }
    }
    fn decode_run(&mut self, text: String) -> Result<Arc<DecodedRun>> {
        if let Some(cached) = &self.decoded_run
            && cached.text == text
        {
            return Ok(Arc::clone(cached));
        }
        let decoded = Arc::new(DecodedRun {
            hash: crate::hash(&text),
            document: serde_json::from_str(&text)?,
            text,
            captured: OnceLock::new(),
            management: OnceLock::new(),
        });
        self.decoded_run = Some(Arc::clone(&decoded));
        Ok(decoded)
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
        // New daemons verify actual complete SQLite bytes in the same capture
        // as their decoded control state. Only that explicit guarantee permits
        // reuse. Older daemons still require a complete hash-pinned download.
        let expected_hash = control["document_hash"].as_str().ok_or_else(|| {
            Error::Invalid("snapshot document hash unavailable; update daemon".into())
        })?;
        let cached = self.decoded_run.as_ref().filter(|cached| {
            control["document_hash_verified"] == true && cached.hash == expected_hash
        });
        let decoded = if let Some(cached) = cached {
            Arc::clone(cached)
        } else {
            let text = ipc::read_document(&self.state, run_id, "run")?;
            crate::require(
                expected_hash == crate::hash(&text),
                "run changed between control snapshot and document read; refresh again",
            )?;
            self.decode_run(text)?
        };
        let document = &decoded.document;
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
            crate::require(
                control["task_recovery_document_hash"]
                    .as_str()
                    .is_none_or(|expected| crate::hash(&text) == expected),
                "task recovery document differs from control snapshot",
            )?;
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
            let captured = decoded.captured()?;
            for (id, value) in recovery["check_restores"].as_object().unwrap() {
                let package: crate::runtime::team::check_recovery::Pending =
                    serde_json::from_value(value.clone())?;
                package.validate_for_run(captured)?;
                crate::require(
                    package.invocation_id == *id,
                    "captured task check package identity changed",
                )?;
            }
            for (id, value) in recovery["replacements"].as_object().unwrap() {
                let package: crate::runtime::team::replacement::Pending =
                    serde_json::from_value(value.clone())?;
                package.validate()?;
                package.validate_for_run(captured)?;
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
                control["orchestration_recovery_document_hash"]
                    .as_str()
                    .is_none_or(|expected| crate::hash(&text) == expected),
                "management recovery document differs from control snapshot",
            )?;
            crate::require(
                recovery["packages"].as_object().is_some_and(|p| {
                    Some(p.len() as u64) == control["orchestration_recovery_count"].as_u64()
                }),
                "management recovery package count changed",
            )?;
            control["orchestration_recoveries"] = recovery["packages"].clone();
            let packages = serde_json::from_value(recovery["packages"].clone())?;
            decoded.validate_management(&packages)?;
        }
        let snapshot = Snapshot::from_document(document, &control);
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
            let accepted: crate::domain::Run = crate::transport::decode(
                ipc::read_document(&self.state, run_id, "run")?.as_bytes(),
            )?;
            if accepted
                .spec
                .config
                .as_ref()
                .is_some_and(|p| p.config.workspace.mode == crate::config::WorkspaceMode::Worktree)
            {
                return self.act(run_id,Action::InspectWorkspace)
                    .map(|_| "Contract accepted; observing inputs for a private workspace. Review and approve preparation next.".into());
            }
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
                Err(e) if e.to_string().contains(crate::runtime::OVERLAPPING_EFFECTS) => format!("Message queued; turn NOT started: {e}. Once that run is reconciled, Ctrl+P here starts the queued turn."),
                Err(e) => format!("Message queued; turn start unconfirmed: {e}. Check status; Ctrl+P can start a queued turn when ready."),
            });
        }
        let mut command = json!({"run_id":run_id});
        let kind = match &action {
            Action::InspectDelivery => "inspect_delivery",
            Action::DeliverSource { scope_hash } => {
                command["scope_hash"] = json!(scope_hash);
                "deliver_source"
            }
            Action::ReconcileSourceDelivery {
                scope_hash,
                observation_hash,
            } => {
                command["scope_hash"] = json!(scope_hash);
                command["observation_hash"] = json!(observation_hash);
                "reconcile_source_delivery"
            }
            Action::RestoreSourceDeliveryBackup { path, scope_hash } => {
                command["path"] = json!(path);
                command["scope_hash"] = json!(scope_hash);
                "restore_source_delivery_backup"
            }
            Action::InspectWorkspace => "inspect_workspace",
            Action::PrepareWorkspace {
                selected,
                scope_hash,
            } => {
                command["selected"] = json!(selected);
                command["scope_hash"] = json!(scope_hash);
                "prepare_workspace"
            }
            Action::ActivateWorkspace { scope_hash } => {
                command["scope_hash"] = json!(scope_hash);
                "activate_workspace"
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_snapshots_skip_repeat_downloads_and_older_daemons_still_download() {
        use std::{
            io::{BufReader, Write},
            os::unix::{fs::PermissionsExt, net::UnixListener},
            time::{Duration, Instant},
        };
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        ipc::private_directory(root.path()).unwrap();
        let socket = root.path().join("architect.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let before = json!({"id":"run","diagnostic":"before"}).to_string();
        let after = json!({"id":"run","diagnostic":"after"}).to_string();
        let control = |body: &str, verified: Option<bool>| {
            let mut value = json!({"document_hash":crate::hash(body)});
            if let Some(verified) = verified {
                value["document_hash_verified"] = json!(verified);
            }
            value
        };
        let chunk = |body: &str| json!({"hash":crate::hash(body),"chunk":body,"next_offset":null});
        let responses = [
            ("snapshot", control(&before, Some(true))),
            ("read", chunk(&before)),
            ("snapshot", control(&before, Some(true))),
            ("snapshot", control(&before, Some(false))),
            ("read", chunk(&before)),
            ("snapshot", control(&before, None)),
            ("read", chunk(&before)),
            ("snapshot", control(&after, Some(true))),
            ("read", chunk(&after)),
        ];
        let server = std::thread::spawn(move || {
            for (expected, data) in responses {
                let start = Instant::now();
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(start.elapsed() < Duration::from_secs(5));
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                // Darwin may inherit the listener's O_NONBLOCK. The server
                // must wait for the client's first bytes, not race its write.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let request: Request = crate::transport::read(&mut BufReader::new(&stream))
                    .unwrap()
                    .unwrap();
                match (&request.operation, expected) {
                    (Operation::Snapshot { run_id }, "snapshot") => assert_eq!(run_id, "run"),
                    (
                        Operation::Read {
                            run_id,
                            document,
                            offset,
                            ..
                        },
                        "read",
                    ) => {
                        assert_eq!(run_id, "run");
                        assert_eq!(document, "run");
                        assert_eq!(*offset, 0);
                    }
                    _ => panic!("unexpected document transfer: {:?}", request.operation),
                }
                let response = ipc::Response {
                    version: PROTOCOL_VERSION,
                    request_id: request.request_id,
                    data: Some(data),
                    error: None,
                };
                serde_json::to_writer(&mut stream, &response).unwrap();
                stream.write_all(b"\n").unwrap();
            }
        });
        let mut backend = IpcBackend::new(root.path());
        for _ in 0..4 {
            assert_eq!(backend.snapshot("run").unwrap().diagnostic, "before");
        }
        assert_eq!(backend.snapshot("run").unwrap().diagnostic, "after");
        server.join().unwrap();
    }

    #[test]
    fn decoded_documents_reuse_only_exact_bytes_and_keep_one_current_run() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("source"), "original").unwrap();
        let mut spec: crate::domain::RunSpec =
            serde_json::from_str(include_str!("../../examples/fake-spec.json")).unwrap();
        spec.workspace = root.path().to_string_lossy().into();
        spec.input_files = vec!["source".into()];
        let mut run = crate::domain::Run::new(crate::id(), spec).unwrap();
        run.diagnostic = Some("before".into());
        run.orchestration_rounds.push(
            serde_json::from_value(json!({
                "id":"round","spec_revision":run.spec_revision,"source_hashes":{},
                "input_hash":crate::hash("inputs"),"approval_scope":crate::hash("scope"),
                "previous_status":"blocked","message_ids":[],"request_sequence":0,
                "request_context":"{}","status":"failed","executor_session":null,
                "verifier_session":null,"output":null,"review":null,"diagnostic":null
            }))
            .unwrap(),
        );
        let text = serde_json::to_string(&run).unwrap();
        let mut backend = IpcBackend::new(root.path());
        let first = backend.decode_run(text.clone()).unwrap();
        let same = backend.decode_run(text.clone()).unwrap();
        assert!(Arc::ptr_eq(&first, &same));
        assert!(std::ptr::eq(
            first.captured().unwrap(),
            same.captured().unwrap()
        ));
        let empty = std::collections::BTreeMap::new();
        first.validate_management(&empty).unwrap();
        assert!(same.management.get().is_some());
        std::fs::write(root.path().join("source"), "changed live source").unwrap();
        assert!(same.validate_management(&empty).is_err());
        std::fs::write(root.path().join("source"), "original").unwrap();
        same.validate_management(&empty).unwrap();
        let forged = std::collections::BTreeMap::from([(
            "round:Replacement".into(),
            serde_json::from_value(json!({"round_id":"round","kind":"replacement",
                "role":"verifier","handoff":{},"handoff_hash":crate::hash("forged"),
                "scope_hash":crate::hash("forged scope")}))
            .unwrap(),
        )]);
        assert!(same.validate_management(&forged).is_err());
        same.validate_management(&empty).unwrap();

        // A same-length change in canonical bytes must replace both decoded
        // views; a prior snapshot continues to describe its original version.
        run.diagnostic = Some("after!".into());
        let changed_text = serde_json::to_string(&run).unwrap();
        assert_eq!(changed_text.len(), text.len());
        let changed = backend.decode_run(changed_text).unwrap();
        assert!(!Arc::ptr_eq(&first, &changed));
        assert!(changed.management.get().is_none());
        assert_eq!(changed.document["diagnostic"], "after!");
        assert_eq!(changed.captured().unwrap().diagnostic, run.diagnostic);
        assert_eq!(
            first.captured().unwrap().diagnostic.as_deref(),
            Some("before")
        );

        assert!(backend.decode_run("{".into()).is_err());
        assert!(Arc::ptr_eq(&changed, backend.decoded_run.as_ref().unwrap()));
        run.id = crate::id();
        let foreign = backend
            .decode_run(serde_json::to_string(&run).unwrap())
            .unwrap();
        assert_eq!(foreign.captured().unwrap().id, run.id);
        let restored = backend.decode_run(text.clone()).unwrap();
        assert!(!Arc::ptr_eq(&first, &restored));
        let whitespace = backend.decode_run(format!("{text}\n")).unwrap();
        assert!(!Arc::ptr_eq(&restored, &whitespace));
        assert_eq!(restored.document, whitespace.document);
    }
}
