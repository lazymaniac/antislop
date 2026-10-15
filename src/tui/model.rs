use super::sanitize::sanitize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const AGENT_TABS: [&str; 8] = [
    "Overview",
    "Session",
    "Artifacts",
    "Findings",
    "Messages",
    "Evidence",
    "Checks",
    "Comments",
];
pub const RUN_TABS: [&str; 5] = ["Requirements", "Options", "Risks", "Quality gates", "Diff"];
#[derive(Debug, Clone, Default)]
pub struct Agent {
    pub id: String,
    pub parent: Option<String>,
    pub role: String,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub task: String,
    pub group: String,
    pub relations: Vec<(String, String)>,
    pub details: BTreeMap<String, String>,
}
/// One planned team task, in persisted plan order.
#[derive(Debug, Clone, Default)]
pub struct Task {
    pub id: String,
    pub description: String,
    pub state: String,
    pub owner: String,
    pub verifier: String,
    pub stage: String,
}
#[derive(Debug, Clone, Default)]
pub struct Message {
    pub id: String,
    pub sender: String,
    pub target: String,
    pub text: String,
    /// Sent/accepted/delivered/replied/failed are separate observed states.
    pub state: String,
    pub session: String,
}
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub run_id: String,
    pub intake: bool,
    pub chat_origin: bool,
    /// Latest provisional interview turn awaiting a user answer.
    pub awaiting_user_reply: Option<String>,
    pub queued_orchestrator_turn: bool,
    pub reviewed_intake_proposal: Option<(String, String)>,
    pub intake_proposal_details: String,
    pub provider_catalogue_details: String,
    pub discovery_status: String,
    /// Actual received/total native reports in the current review partition.
    pub review_progress: Option<(usize, usize)>,
    /// Unique bytes in committed and native-observed context responses.
    pub context_progress: Vec<(String, usize, usize)>,
    pub workspace: String,
    pub workspace_required: bool,
    pub delivery_inspect_available: bool,
    pub delivery_scope: Option<String>,
    pub delivery_details: String,
    pub delivery_recovery: Option<crate::runtime::delivery::Recovery>,
    pub workspace_inspect_available: bool,
    pub workspace_phase: String,
    pub workspace_preparation: Option<(BTreeSet<String>, String)>,
    pub workspace_preparation_details: String,
    pub workspace_activation: Option<String>,
    pub workspace_activation_details: String,
    pub mode: String,
    pub policy_revision: u64,
    pub stage: String,
    pub status: String,
    pub diagnostic: String,
    pub agents: Vec<Agent>,
    pub tasks: Vec<Task>,
    pub running_checks: Vec<(String, String)>,
    pub baseline_active: bool,
    pub messages: Vec<Message>,
    pub run_details: BTreeMap<String, String>,
    pub scope_hashes: BTreeMap<String, String>,
    pub task_verification_retries: BTreeMap<String, String>,
    pub task_session_resumes: BTreeMap<String, crate::runtime::team::resume::Pending>,
    pub task_session_replacements: BTreeMap<String, crate::runtime::team::replacement::Pending>,
    pub task_session_replacement_details: BTreeMap<String, String>,
    pub task_check_restores: BTreeMap<String, crate::runtime::team::check_recovery::Pending>,
    pub task_check_restore_details: BTreeMap<String, String>,
    pub orchestration_recoveries: BTreeMap<String, crate::runtime::orchestration_recovery::Pending>,
    pub orchestration_recovery_preparing: bool,
    pub orchestration_recovery_unavailable: bool,
    pub orchestration_recovery_details: BTreeMap<String, String>,
    pub task_session_resume_details: BTreeMap<String, String>,
    pub task_source_refreshes: BTreeMap<String, String>,
    pub task_result_restores: BTreeMap<String, String>,
    pub task_effect_reconciliations: BTreeMap<String, String>,
    pub wave_effect_reconciliations:
        BTreeMap<String, crate::runtime::team::adjudicate::PendingWave>,
    pub wave_effect_reconciliation_details: BTreeMap<String, String>,
    pub task_effect_reconciliation_details: BTreeMap<String, String>,
    pub task_result_restore_details: BTreeMap<String, String>,
    pub team_stage_results: BTreeMap<String, crate::runtime::team::stage::PendingResult>,
    pub team_stage_result_details: BTreeMap<String, String>,
    pub reconciliation_input_hash: Option<String>,
    pub sequence: u64,
    pub reviewed_team_round: Option<String>,
    pub reviewed_team_details: String,
    pub pending_wave: Option<crate::runtime::team::wave::Pending>,
    pub pending_wave_details: String,
    pub comment_targets: Vec<crate::comments::Element>,
    pub element_comments: Vec<crate::comments::Comment>,
}
fn string(value: &Value) -> String {
    value.as_str().map(sanitize).unwrap_or_else(|| {
        if value.is_null() {
            String::new()
        } else {
            sanitize(&value.to_string())
        }
    })
}
fn field(value: &Value, name: &str) -> String {
    string(&value[name])
}
fn pretty(value: &Value) -> String {
    if value.is_null() {
        return "No recorded data available.".into();
    }
    sanitize(&serde_json::to_string_pretty(value).unwrap_or_default())
}
fn values(value: &Value) -> Vec<&Value> {
    value
        .as_object()
        .map(|o| o.values().collect())
        .unwrap_or_else(|| array(value).iter().collect())
}
fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn select_session(items: &Value, session: &str, key: &str) -> Value {
    Value::Array(
        array(items)
            .iter()
            .filter(|i| i[key].as_str() == Some(session))
            .cloned()
            .collect(),
    )
}
fn latest_session<'a>(run: &Value, sessions: &[&'a Value]) -> Option<&'a Value> {
    if let Some(active) = run["active_session"].as_str()
        && let Some(session) = sessions.iter().find(|s| s["id"] == active)
    {
        return Some(session);
    }
    for turn_id in array(&run["session_ledger"]["turn_order"]).iter().rev() {
        let turn = run["session_ledger"]["turns"].get(turn_id.as_str()?);
        if let Some(turn) = turn
            && let Some(session) = sessions.iter().find(|s| s["id"] == turn["session_id"])
        {
            return Some(session);
        }
    }
    (sessions.len() == 1).then(|| sessions[0])
}
fn session_details(agent: &mut Agent, run: &Value, sessions: &[&Value], current: Option<&Value>) {
    let ids: BTreeSet<_> = sessions.iter().filter_map(|s| s["id"].as_str()).collect();
    let turns: Vec<_> = values(&run["session_ledger"]["turns"])
        .into_iter()
        .filter(|turn| {
            turn["session_id"]
                .as_str()
                .is_some_and(|id| ids.contains(id))
        })
        .collect();
    let artifacts: Vec<_> = array(&run["artifacts"])
        .iter()
        .filter(|artifact| {
            artifact["session_id"]
                .as_str()
                .is_some_and(|id| ids.contains(id))
        })
        .collect();
    let reviews: Vec<_> = array(&run["reviews"])
        .iter()
        .filter(|review| {
            ["executor_session", "verifier_session"]
                .iter()
                .any(|key| review[*key].as_str().is_some_and(|id| ids.contains(id)))
        })
        .collect();
    let artifact_hashes: BTreeSet<_> = artifacts
        .iter()
        .filter_map(|a| a["hash"].as_str())
        .chain(reviews.iter().filter_map(|r| r["artifact_hash"].as_str()))
        .collect();
    let review_evidence: BTreeSet<_> = reviews
        .iter()
        .flat_map(|r| array(&r["checks"]))
        .flat_map(|check| {
            array(&check["evidence_ids"])
                .iter()
                .filter_map(Value::as_str)
        })
        .collect();
    let evidence: Vec<_> = array(&run["evidence"])
        .iter()
        .filter(|e| {
            e["artifact_hash"]
                .as_str()
                .is_some_and(|h| artifact_hashes.contains(h))
                || e["id"]
                    .as_str()
                    .is_some_and(|id| review_evidence.contains(id))
        })
        .collect();
    agent.details.insert("Session".into(), pretty(&serde_json::json!({
        "current_session":current.map(|s| &s["id"]),"sessions":sessions,"turns":turns,
        "selection":if current.is_some() {"Persisted session identity or reservation order."} else if sessions.is_empty() {"No session observed."} else {"Latest session order unavailable in this legacy record."},
        "transcript_limit":"Only adapter-provided output/receipts are available; hidden reasoning is never requested."})));
    agent
        .details
        .insert("Artifacts".into(), pretty(&serde_json::json!(artifacts)));
    agent.details.insert("Findings".into(), pretty(&serde_json::json!(reviews.iter().map(|r|
        serde_json::json!({"artifact_hash":r["artifact_hash"],"findings":r["findings"]})).collect::<Vec<_>>())));
    agent.details.insert(
        "Checks".into(),
        pretty(
            &serde_json::json!({"checklist_version":run["checklist_version"],"reviews":reviews}),
        ),
    );
    agent
        .details
        .insert("Evidence".into(), pretty(&serde_json::json!(evidence)));
    agent.details.insert(
        "Messages".into(),
        pretty(&serde_json::json!(
            values(&run["communication_ledger"]["messages"])
                .into_iter()
                .filter(
                    |m| m["recipient_agent_id"] == agent.id || m["sender"]["agent_id"] == agent.id
                )
                .collect::<Vec<_>>()
        )),
    );
}
fn messages_queued_after(run: &Value, round: &Value) -> bool {
    values(&run["communication_ledger"]["messages"])
        .iter()
        .any(|message| {
            message["recipient_agent_id"] == "orchestrator"
                && message["sender"]["kind"] == "user"
                && message["sequence"].as_u64().unwrap_or(0)
                    > round["request_sequence"].as_u64().unwrap_or(0)
        })
}

impl Snapshot {
    /// Project captured records and explicit daemon control state. Missing
    /// transcripts and native activity stay explicit.
    pub fn from_document(run: &Value, control: &Value) -> Self {
        let mut snapshot = Self {
            orchestration_recovery_unavailable: control["orchestration_recovery_state"]
                == "unavailable",
            orchestration_recovery_preparing: control["orchestration_recovery_state"]
                == "preparing",
            context_progress: values(&run["session_ledger"]["sessions"])
                .into_iter()
                .filter_map(|session| {
                    if !matches!(
                        session["state"].as_str(),
                        Some("sent" | "pending" | "pending_unknown")
                    ) {
                        return None;
                    }
                    let id = session["current_turn_id"].as_str()?;
                    let transport: crate::context::transport::Transport = serde_json::from_value(
                        run["session_ledger"]["turns"][id]["context"].clone(),
                    )
                    .ok()?;
                    let (read, total) = transport.progress().ok()?;
                    Some((field(&session["scope"], "agent_id"), read, total))
                })
                .collect(),
            run_id: field(run, "id"),
            intake: run["intake"].as_bool().unwrap_or(false),
            chat_origin: run["chat_origin"].as_bool().unwrap_or(false),
            discovery_status: array(&run["orchestration_rounds"])
                .last()
                .map(|r| field(r, "discovery_status"))
                .unwrap_or_default(),
            review_progress: array(&run["orchestration_rounds"])
                .last()
                .filter(|r| r["status"] == "running")
                .and_then(|r| {
                    let parts = array(&r["review_parts"]);
                    (!parts.is_empty()).then(|| {
                        (
                            parts.iter().filter(|p| p["output"].is_string()).count(),
                            parts.len(),
                        )
                    })
                }),
            reviewed_intake_proposal: serde_json::from_value(
                control["reviewed_intake_proposal"].clone(),
            )
            .unwrap_or_default(),
            workspace: field(&run["spec"], "workspace"),
            mode: run
                .pointer("/mode")
                .or_else(|| run.pointer("/spec/config/config/policy/mode"))
                .map(string)
                .unwrap_or_else(|| "supervised".into()),
            policy_revision: run
                .pointer("/spec/config/config/policy/revision")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            stage: if run["intake"].as_bool().unwrap_or(false) {
                "intake".into()
            } else {
                field(run, "stage")
            },
            status: field(run, "status"),
            diagnostic: field(run, "diagnostic"),
            sequence: control["sequence"].as_u64().unwrap_or(0),
            reviewed_team_round: control["reviewed_team_round"].as_str().map(String::from),
            reconciliation_input_hash: control
                .pointer("/recovery/observed_input_hash")
                .and_then(Value::as_str)
                .map(str::to_owned),
            ..Self::default()
        };
        snapshot.workspace_required = !snapshot.intake
            && run["spec"]["config"]["config"]["workspace"]["mode"] == "worktree"
            && run["managed_workspace"]["state"] != "active";
        if let Ok(document) = serde_json::from_value::<crate::domain::Run>(run.clone()) {
            snapshot.workspace_inspect_available =
                crate::workspace::managed::eligible(&document).is_ok();
        }
        snapshot.workspace_phase = if run["workspace_inspection"]["state"] == "running" {
            "Observe workspace inputs and local changes"
        } else if run["workspace_activation_attempt"]["state"] == "running" {
            "Validate and activate private workspace"
        } else if run["managed_workspace"]["state"] == "intent" {
            "Prepare private workspace candidate"
        } else {
            ""
        }
        .into();
        if run["delivery"]["state"] == "inspecting" {
            snapshot.workspace_phase = "Inspect verified result for source delivery".into();
        } else if run["delivery"]["state"] == "applying" {
            snapshot.workspace_phase = "Transfer approved changes to original checkout".into();
        }
        snapshot.delivery_inspect_available = control["source_delivery_inspection_available"]
            .as_bool()
            .unwrap_or(false);
        snapshot.delivery_scope = control["source_delivery_scope"].as_str().map(str::to_owned);
        snapshot.delivery_recovery =
            serde_json::from_value(control["source_delivery_recovery"].clone()).ok();
        let delivery = &run["delivery"];
        let changes:Vec<_>=array(&delivery["plan"]["changes"]).iter().map(|c|serde_json::json!({"path":c["path"],"before":c["before"],"after":c["after"]["stamp"]})).collect();
        snapshot.delivery_details = format!(
            "Transfer only the displayed verified candidate changes to the original checkout. Previous entries remain in private delivery backups. This approval does not commit, push or publish the project.\n\n{}",
            pretty(
                &serde_json::json!({"state":delivery["state"],"source_checkout":delivery["plan"]["source_root"],"candidate_version":delivery["plan"]["candidate"]["content_hash"],"changes":changes,"conflicts":delivery["plan"]["conflicts"],"effects":delivery["effects"],"diagnostic":delivery["diagnostic"],"recovery":control["source_delivery_recovery"]})
            )
        );
        if let Some(scope) = control["workspace_preparation_scope"].as_str() {
            let plan = &run["workspace_plan"];
            if plan["scope_hash"] == scope {
                let selected: BTreeSet<String> = array(&plan["selectors"])
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                snapshot.workspace_preparation = Some((selected, scope.into()));
                snapshot.workspace_preparation_details = format!(
                    "Create only a private candidate from the displayed source version. The source checkout is preserved. Included local working copies/deletions become task-owned in the candidate; other local changes remain outside it. Activation and task execution require separate approvals.\n\n{}",
                    pretty(
                        &serde_json::json!({"source_root":plan["source"]["root"],"source_version":plan["source"]["content_hash"],"repository_commit":plan["source"]["repository"]["head"],"declared_scopes":plan["selectors"],"included_local_changes":plan["selected"],"all_observed_local_changes":plan["source"]["local_changes"],"candidate_file_count":plan["files"].as_object().map_or(0,|files|files.len()),"candidate_bytes":plan["files"].as_object().map(|files|files.values().filter_map(|f|f["length"].as_u64()).sum::<u64>()),"directory_inputs":plan["directory_inputs"],"project_prefix":plan["project_prefix"]})
                    )
                );
            }
        }
        if let Some(scope) = control["workspace_activation_scope"].as_str() {
            snapshot.workspace_activation = Some(scope.into());
            snapshot.workspace_activation_details = format!(
                "Validate and activate this captured private workspace in a new task revision. Accepted input contents must match. No provider, check command, source transfer or task execution is authorized by this action.\n\n{}",
                pretty(
                    &serde_json::json!({"workspace_id":run["managed_workspace"]["workspace_id"],"private_project":run["managed_workspace"]["receipt"]["project_root"],"candidate_commit":run["managed_workspace"]["receipt"]["commit"],"candidate_version":run["managed_workspace"]["receipt"]["observed"]["content_hash"],"source_workspace":run["spec"]["workspace"]})
                )
            );
        }
        snapshot.baseline_active = array(&run["task_checks"])
            .iter()
            .any(|i| i["state"] == "intent" && i["plan"]["phase"] == "before_execution");
        snapshot.running_checks = array(&run["task_checks"])
            .iter()
            .filter(|i| i["state"] == "intent")
            .map(|i| {
                (
                    field(i, "task_id"),
                    if i["plan"]["phase"] == "before_execution" {
                        format!("baseline:{}", field(&i["plan"], "id"))
                    } else {
                        field(&i["plan"], "id")
                    },
                )
            })
            .collect();
        if let Ok(document) = serde_json::from_value::<crate::domain::Run>(run.clone()) {
            snapshot.comment_targets = crate::comments::elements(&document)
                .into_iter()
                .map(|mut e| {
                    e.context = Value::Null;
                    e
                })
                .collect();
            snapshot.element_comments = document.element_comments;
        }
        if let Some(hashes) = control["scope_hashes"].as_object() {
            snapshot.scope_hashes = hashes.iter().map(|(k, v)| (k.clone(), string(v))).collect();
        }
        if let Some(hashes) = control["task_verification_retries"].as_object() {
            snapshot.task_verification_retries =
                hashes.iter().map(|(k, v)| (k.clone(), string(v))).collect();
        }
        if let Some(packages) = control["task_session_resumes"].as_object() {
            for (id, package) in packages {
                if let Ok(pending) =
                    serde_json::from_value::<crate::runtime::team::resume::Pending>(package.clone())
                {
                    snapshot.task_session_resume_details.insert(pending.scope_hash.clone(),
                        format!("Continue the exact observed native session, preserving this task attempt.\nThe previous interrupted/unknown turn remains history. This is a new native turn, not task acceptance, effect replay or a replacement session.\n\n{}",pretty(package)));
                    snapshot.task_session_resumes.insert(id.clone(), pending);
                }
            }
        }
        if let Some(packages) = control["task_session_replacements"].as_object() {
            for (id, package) in packages {
                if let Ok(p) = serde_json::from_value::<crate::runtime::team::replacement::Pending>(
                    package.clone(),
                ) {
                    snapshot.task_session_replacement_details.insert(p.scope_hash.clone(),format!("Start a DISTINCT NEW SESSION with this captured handoff. This does not resume or impersonate the old native session. Preserve this task attempt, old sessions, unresolved findings and unknown delivery/effect history. Independent verification and stage acceptance remain required.\n\n{}",pretty(package)));
                    snapshot.task_session_replacements.insert(id.clone(), p);
                }
            }
        }
        if let Ok(captured) = serde_json::from_value::<crate::domain::Run>(run.clone())
            && let Ok(wave) = crate::runtime::team::wave::pending(&captured)
            && control["scope_hashes"]["step_team"].as_str() == Some(wave.scope_hash.as_str())
        {
            snapshot.pending_wave_details = format!(
                "Authorize only the displayed scheduler wave. Executors and independent reviewers run in separate waves in supervised mode. Later tasks and project effects require their own approvals.\n\n{}",
                pretty(&serde_json::to_value(&wave).unwrap_or(Value::Null))
            );
            snapshot.pending_wave = Some(wave);
        }
        snapshot.queued_orchestrator_turn = !run["reconciliation_required"]
            .as_bool()
            .unwrap_or(false)
            && !matches!(
                snapshot.status.as_str(),
                "running" | "paused" | "completed" | "cancelled"
            )
            && values(&run["communication_ledger"]["messages"])
                .iter()
                .any(|message| {
                    message["recipient_agent_id"] == "orchestrator"
                        && matches!(message["sender"]["kind"].as_str(), Some("user" | "runtime"))
                        && matches!(
                            message["state"].as_str(),
                            Some("accepted_runtime" | "queued_next_turn")
                        )
                });
        if let Some(packages) = control["orchestration_recoveries"].as_object() {
            for (key, value) in packages {
                if let Ok(p) = serde_json::from_value::<
                    crate::runtime::orchestration_recovery::Pending,
                >(value.clone())
                {
                    snapshot.orchestration_recovery_details.insert(p.scope_hash.clone(),format!("Read-only management recovery. Resume keeps the exact native ID; replacement starts a DISTINCT NEW SESSION; restore uses already committed output without native replay. Completed orchestrator output is retained. This does not install or execute a task team.\n\n{}",pretty(value)));
                    snapshot.orchestration_recoveries.insert(key.clone(), p);
                }
            }
        }
        if let Some(packages) = control["task_check_restores"].as_object() {
            for (id, value) in packages {
                if let Ok(p) = serde_json::from_value::<crate::runtime::team::check_recovery::Pending>(
                    value.clone(),
                ) {
                    let mut metadata = value.clone();
                    metadata["outcome"]
                        .as_object_mut()
                        .unwrap()
                        .remove("output");
                    snapshot.task_check_restore_details.insert(p.scope_hash.clone(),sanitize(&format!("Restore only this actually captured runner result. The original command is NOT run again; its observed exit/signal/output is retained. Independent task review remains required.\n\n{}\n\nActual runner receipt:\n{}",pretty(&metadata),p.outcome.output)));
                    snapshot.task_check_restores.insert(id.clone(), p);
                }
            }
        }
        if let Some((round_id, _)) = &snapshot.reviewed_intake_proposal
            && let Some(round) = array(&run["orchestration_rounds"])
                .iter()
                .find(|r| r["id"] == *round_id)
        {
            let response: Value =
                serde_json::from_str(round["output"].as_str().unwrap_or_default())
                    .unwrap_or(Value::Null);
            let proposal = response["intake_proposal_json"]
                .as_str()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                .unwrap_or(Value::Null);
            let approach = proposal["task"]["team_template"]
                .as_str()
                .map(|name| format!("Selected team approach: {name}\n\n"))
                .unwrap_or_default();
            snapshot.intake_proposal_details = format!(
                    "Approve the following task contract and routing selected by the orchestrator.
This replaces the provisional interview scope and authorizes one read-only team-planning turn with independent review.
It does not start task executors, run project checks, write files or approve quality gates.

{}Proposal (requirements, profiles, local permissions, exact files/check argv):
{}

Proposed source hashes:
{}

Independent review:
{}",
                    approach,pretty(&proposal),
                    pretty(&serde_json::to_value(round["proposed_sources"].as_object().map(|sources| sources.iter().map(|(p, text)| (p.clone(),crate::hash(text.as_str().unwrap_or_default()))).collect::<BTreeMap<_,_>>())).unwrap_or(Value::Null)),
                    round["review"].as_str().unwrap_or_default());
        }
        if let Some(round_id) = &snapshot.reviewed_team_round
            && let Some(round) = array(&run["orchestration_rounds"])
                .iter()
                .find(|r| r["id"] == *round_id)
        {
            let response: Value =
                serde_json::from_str(round["output"].as_str().unwrap_or_default())
                    .unwrap_or(Value::Null);
            let raw = response["team_plan_json"].as_str().unwrap_or_default();
            let resolved =
                (|| -> crate::Result<crate::orchestration::planning::amendment::Resolved> {
                    let value: Value = crate::transport::decode(raw.as_bytes())?;
                    if value.get("format").is_some() {
                        let captured: crate::domain::Run = serde_json::from_value(run.clone())?;
                        let response = serde_json::from_value(response.clone())?;
                        crate::orchestration::planning::amendment::resolve(&captured, &response)
                    } else {
                        Ok(crate::orchestration::planning::amendment::Resolved {
                            team: serde_json::from_value(value)?,
                            risk_owners: serde_json::from_value(response["risk_owners"].clone())
                                .unwrap_or_default(),
                            bases: vec![],
                        })
                    }
                })();
            let assignments = (|| -> crate::Result<Vec<Value>> {
                let team = &resolved
                    .as_ref()
                    .map_err(|e| crate::Error::Invalid(e.to_string()))?
                    .team;
                let spec: crate::domain::RunSpec = serde_json::from_value(run["spec"].clone())?;
                let config = &spec
                    .config
                    .ok_or_else(|| crate::Error::Invalid("configuration missing".into()))?
                    .config;
                team.agents
                    .iter()
                    .map(|agent| {
                        Ok(serde_json::json!({"agent":agent.id,"role":agent.role,
                    "configuration":config.resolve(agent.role.work_kind(),agent.kind)?}))
                    })
                    .collect()
            })();
            snapshot.reviewed_team_details = format!(
                "Install only the reviewed proposed team. Tasks remain planned; starting a wave requires a separate displayed approval.\n\nAgent roles and effective configurations:\n{}\n\nTask DAG, gates and permissions:\n{}\n\nIndependent review:\n{}",
                pretty(
                    &serde_json::to_value(assignments.unwrap_or_default()).unwrap_or(Value::Null)
                ),
                pretty(&serde_json::to_value(resolved.as_ref().ok()).unwrap_or(Value::Null)),
                round["review"].as_str().unwrap_or_default()
            );
            if !array(&round["review_parts"]).is_empty() {
                let provenance:Vec<_>=array(&round["review_parts"]).iter().map(|p|serde_json::json!({
                    "part":p["id"],"check_ids":p["check_ids"],"session":p["session_id"],"output_hash":p["output"].as_str().map(crate::hash)
                })).collect();
                snapshot.reviewed_team_details.push_str(&format!("\n\nThe report above is derived from all individually authenticated native parts. Every original check remains required; no single part approves this proposal.\n{}",pretty(&serde_json::json!(provenance))));
            }
        }
        snapshot.team_stage_results =
            serde_json::from_value(control["team_stage_results"].clone()).unwrap_or_default();
        for package in snapshot.team_stage_results.values() {
            let mut details = String::from(
                "Accepts only the listed verified results. Starting the next wave requires a separate scope approval.\n",
            );
            for result in &package.results {
                let task = array(&run["team"]["tasks"])
                    .iter()
                    .find(|t| t["id"] == result.task_id);
                let artifact = array(&run["artifacts"])
                    .iter()
                    .find(|a| a["id"] == result.artifact_id && a["hash"] == result.artifact_hash);
                if let (Some(task), Some(artifact)) = (task, artifact) {
                    let output: Value =
                        serde_json::from_str(artifact["content"].as_str().unwrap_or_default())
                            .unwrap_or(Value::Null);
                    details.push_str(&format!("\nTask: {} — {}\nExecutor: {} / verifier: {}\nArtifact: {}\nArtifact hash: {}\nReview hash: {}\n\nResult:\n{}\n\nProposed edits:\n{}\n\nAccepted criteria:\n{}\n",
                        result.task_id, field(task,"description"), field(task,"owner_id"), field(task,"verifier_id"), result.artifact_id, result.artifact_hash, result.review_hash,
                        field(&output,"artifact"), pretty(&output["edits"]), pretty(&task["review"]["criteria"])));
                }
            }
            details.push_str("\nFull models and proof are available in the agents' Artifacts, Evidence and Checks tabs.");
            snapshot
                .team_stage_result_details
                .insert(package.scope_hash.clone(), sanitize(&details));
        }
        snapshot.wave_effect_reconciliations =
            serde_json::from_value(control["wave_effect_reconciliations"].clone())
                .unwrap_or_default();
        for wave in snapshot.wave_effect_reconciliations.values() {
            let mut details = format!(
                "Observed effects from original wave {}\nExact selected tasks: {}\n\n",
                wave.wave_attempt,
                wave.task_ids.join(", ")
            );
            for task_id in &wave.task_ids {
                for execution in array(&run["task_executions"]).iter().filter(|e| {
                    e["task_id"] == *task_id
                        && e["wave_attempt"].as_u64() == Some(wave.wave_attempt)
                        && e["state"] == "intent"
                }) {
                    details.push_str(&format!("Task: {task_id} / execution: {}\nNative session: {} / output hash: {}\nOriginal retained intent:\n{}\n\n", field(execution,"id"),field(execution,"session_id"),field(execution,"artifact_hash"),pretty(&execution["receipts"])));
                }
            }
            details.push_str("Every selected proposed after-state matches the observed pinned files. This acknowledges the entire displayed cohort atomically without claiming who wrote the files or repeating any write. Results still need separate restoration, source refresh and independent verification.");
            snapshot
                .wave_effect_reconciliation_details
                .insert(wave.scope_hash.clone(), sanitize(&details));
        }
        if let Some(scopes) = control["task_effect_reconciliations"].as_object() {
            snapshot.task_effect_reconciliations = scopes
                .iter()
                .map(|(id, scope)| (id.clone(), string(scope)))
                .collect();
            for (task_id, scope) in &snapshot.task_effect_reconciliations {
                if let Some(execution) = array(&run["task_executions"])
                    .iter()
                    .find(|e| e["task_id"] == *task_id && e["state"] == "intent")
                {
                    snapshot.task_effect_reconciliation_details.insert(scope.clone(), sanitize(&format!("Task: {task_id}\nExecution: {} / native session: {}\nNative output hash: {}\n\nOriginal retained intent:\n{}\n\nThe observed files exactly match these proposed after-hashes. This records a user-confirmed observation, without claiming who performed the write and without writing again. It does not verify or approve the result.", field(execution,"id"), field(execution,"session_id"), field(execution,"artifact_hash"), pretty(&execution["receipts"]))));
                }
            }
        }
        if let Some(scopes) = control["task_result_restores"].as_object() {
            snapshot.task_result_restores = scopes
                .iter()
                .map(|(task, scope)| (task.clone(), string(scope)))
                .collect();
            for (task_id, scope) in &snapshot.task_result_restores {
                if let Some(task) = array(&run["team"]["tasks"])
                    .iter()
                    .find(|t| t["id"] == *task_id)
                    && let Some(execution) = array(&run["task_executions"]).iter().find(|e| {
                        e["task_id"] == *task_id
                            && e["attempt"] == task["attempt"]
                            && (e["state"] == "applied" || e["state"] == "observed_applied")
                    })
                {
                    let session_id = execution["session_id"].as_str().unwrap_or_default();
                    let turn_id = run["session_ledger"]["sessions"][session_id]["current_turn_id"]
                        .as_str()
                        .unwrap_or_default();
                    let output: Value = serde_json::from_str(
                        run["session_ledger"]["turns"][turn_id]["output"]
                            .as_str()
                            .unwrap_or_default(),
                    )
                    .unwrap_or(Value::Null);
                    snapshot.task_result_restore_details.insert(scope.clone(), sanitize(&format!("Task: {task_id} — {}\nCompleted executor session: {session_id} / turn: {turn_id}\nOutput hash: {}\n\nResult:\n{}\n\nRetained effect receipts:\n{}\n\nRestores the retained result without repeating the agent, checks or file edits. Independent verification or source refresh is still required.", field(task,"description"), field(execution,"artifact_hash"), field(&output,"artifact"), pretty(&execution["receipts"]))));
                }
            }
        }
        if let Some(hashes) = control["task_source_refreshes"].as_object() {
            snapshot.task_source_refreshes =
                hashes.iter().map(|(k, v)| (k.clone(), string(v))).collect();
        }
        snapshot
            .run_details
            .insert("Requirements".into(), pretty(&run["spec"]["requirements"]));
        if let Some(round) = array(&run["orchestration_rounds"])
            .iter()
            .rev()
            .find(|r| !r["provider_catalogue"].is_null())
        {
            snapshot.provider_catalogue_details = pretty(&round["provider_catalogue"]);
        }
        let options = serde_json::json!({"provider_catalogue":snapshot.provider_catalogue_details,"orchestration_rounds":run["orchestration_rounds"],"accepted_team":run["team"],"risk_owners":run["team_risk_owners"],"planning_audit":run.pointer("/quality/planning"), "solution_models":run["provenance"].as_object().map(|nodes| nodes.iter().filter(|(_, node)| node["kind"] == "solution").collect::<BTreeMap<_,_>>())});
        snapshot
            .run_details
            .insert("Options".into(), pretty(&options));
        snapshot
            .run_details
            .insert("Risks".into(), pretty(&run["risk_coverage"]));
        snapshot
            .run_details
            .insert("Quality gates".into(), pretty(&run["quality_gates"]));
        let diff_report: Option<crate::workspace::DiffReport> =
            if snapshot.intake || snapshot.workspace_required {
                None
            } else {
                run.get("spec")
                    .and_then(|spec| spec.get("workspace"))
                    .and_then(|ws| ws.as_str())
                    .and_then(|ws| {
                        let baseline: Option<std::collections::BTreeMap<String, String>> =
                            serde_json::from_value(
                                run.get("diff_baseline")
                                    .filter(|value| !value.is_null())
                                    .unwrap_or(&run["input_snapshot"])
                                    .clone(),
                            )
                            .ok();
                        baseline.and_then(|b| {
                            crate::workspace::compute_diff(std::path::Path::new(ws), &b).ok()
                        })
                    })
            };
        let diff_text = if snapshot.intake {
            "Diff becomes available after the interview contract selects the task inputs.".into()
        } else if snapshot.workspace_required {
            "The candidate diff becomes available after the private workspace is prepared and activated.".into()
        } else if let Some(diff) = &diff_report {
            let mut s = format!(
                "Total files changed: {}\nTotal additions: +{}\nTotal deletions: -{}\n",
                diff.files.len(),
                diff.total_additions,
                diff.total_deletions
            );
            let comments = array(&run["diff_comments"]);
            for f in &diff.files {
                s.push_str(&format!(
                    "\n=== {} [{:?}] (+{} / -{}) ===\n",
                    f.path, f.status, f.additions, f.deletions
                ));
                if let Some(metadata) = &f.metadata_change {
                    s.push_str(metadata);
                    s.push('\n');
                }
                for c in comments {
                    if c["path"] == f.path && c["line_number"].is_null() {
                        s.push_str(&format!(
                            "  [File Comment by {} (id: {}, resolved: {})]: {}\n",
                            field(c, "author"),
                            field(c, "id"),
                            field(c, "resolved"),
                            field(c, "text")
                        ));
                    }
                }
                for hunk in &f.hunks {
                    s.push_str(&format!("{}\n", hunk.header));
                    for l in &hunk.lines {
                        let prefix = match l.kind {
                            crate::workspace::DiffLineKind::Addition => "+",
                            crate::workspace::DiffLineKind::Deletion => "-",
                            crate::workspace::DiffLineKind::Context => " ",
                        };
                        let num = l.new_line_number.or(l.old_line_number).unwrap_or(0);
                        s.push_str(&format!("{prefix} {:4} | {}\n", num, l.content));
                        if let Some(nl) = l.new_line_number {
                            for c in comments {
                                if c["path"] == f.path
                                    && c["line_number"].as_u64() == Some(nl as u64)
                                {
                                    s.push_str(&format!(
                                        "    >> [Line Comment by {} (id: {}, resolved: {})]: {}\n",
                                        field(c, "author"),
                                        field(c, "id"),
                                        field(c, "resolved"),
                                        field(c, "text")
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            s
        } else {
            "Diff inspection unavailable or workspace unmodified.".into()
        };
        let reviews = array(&run["diff_reviews"])
            .iter()
            .map(|review| {
                serde_json::json!({
                    "id":review["id"],"state":review["state"],"diff_hash":review["diff_hash"],
                    "session_id":review["session_id"],"profile":review["profile"],
                    "response":review["response"],"diagnostic":review["diagnostic"]
                })
            })
            .collect::<Vec<_>>();
        let prospective = serde_json::from_value::<crate::domain::Run>(run.clone())
            .ok()
            .and_then(|run| crate::runtime::diff_review::reviewer_profile(&run).ok());
        let diff_text = if let Some(profile) = prospective {
            format!(
                "Independent diff reviewer profile:\n{}\n\n{diff_text}",
                pretty(&serde_json::json!(profile))
            )
        } else {
            diff_text
        };
        let diff_text = if reviews.is_empty() {
            diff_text
        } else {
            format!(
                "{diff_text}\n\nIndependent candidate reviews (separate from stage acceptance):\n{}",
                pretty(&serde_json::json!(reviews))
            )
        };
        snapshot.run_details.insert("Diff".into(), diff_text);
        let mut messages = values(&run["communication_ledger"]["messages"]);
        if messages.is_empty() {
            messages = values(&run["messages"]);
        }
        messages.sort_by_key(|m| m["sequence"].as_u64().unwrap_or(0));
        for message in messages {
            snapshot.messages.push(Message {
                id: field(message, "id"),
                sender: if message["sender"].is_object() {
                    match message["sender"]["kind"].as_str() {
                        Some("agent") => field(&message["sender"], "agent_id"),
                        Some(kind) => kind.into(),
                        None => "unknown sender".into(),
                    }
                } else {
                    field(message, "sender")
                },
                target: message
                    .get("recipient_agent_id")
                    .or_else(|| message.get("target_agent_id"))
                    .or_else(|| message.get("target"))
                    .map(string)
                    .unwrap_or_default(),
                text: message
                    .get("text")
                    .or_else(|| message.get("content"))
                    .or_else(|| run["message_bodies"].get(message["id"].as_str().unwrap_or("")))
                    .map(string)
                    .unwrap_or_else(|| {
                        format!(
                            "Content reference: {} (body unavailable)",
                            pretty(&message["content_ref"])
                        )
                    }),
                state: message
                    .get("delivery")
                    .or_else(|| message.get("state"))
                    .map(string)
                    .unwrap_or_else(|| "state unavailable".into()),
                session: field(message, "session_id"),
            });
        }
        let mut ordering: BTreeMap<String, (u64, bool)> =
            values(&run["communication_ledger"]["messages"])
                .into_iter()
                .filter_map(|m| {
                    Some((
                        m["id"].as_str()?.to_owned(),
                        (m["sequence"].as_u64()?, false),
                    ))
                })
                .collect();
        if snapshot.intake
            && !matches!(
                snapshot.status.as_str(),
                "running" | "paused" | "cancelled" | "completed"
            )
            && let Some(round) = array(&run["orchestration_rounds"]).last()
            && round["status"] == "awaiting_user"
            && !messages_queued_after(run, round)
        {
            snapshot.awaiting_user_reply = Some(field(round, "id"));
        }
        for round in array(&run["orchestration_rounds"]) {
            let response = round["output"]
                .as_str()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
            if let Some(text) = response.as_ref().and_then(|r| r["message"].as_str()) {
                let id = format!("orchestration:{}", field(round, "id"));
                ordering.insert(
                    id.clone(),
                    (round["request_sequence"].as_u64().unwrap_or(0), true),
                );
                snapshot.messages.push(Message {
                    id,
                    sender: "orchestrator".into(),
                    target: "user".into(),
                    text: sanitize(text),
                    state: format!("orchestration_{}", field(round, "status")),
                    session: field(round, "executor_session"),
                });
            }
        }
        snapshot
            .messages
            .sort_by_key(|m| ordering.get(&m.id).copied().unwrap_or((0, false)));
        let team = run
            .get("team")
            .or_else(|| run.get("team_plan"))
            .unwrap_or(&Value::Null);
        snapshot.tasks = array(&team["tasks"])
            .iter()
            .map(|task| Task {
                id: field(task, "id"),
                description: field(task, "description"),
                state: field(task, "state"),
                owner: field(task, "owner_id"),
                verifier: field(task, "verifier_id"),
                stage: field(task, "stage"),
            })
            .collect();
        for record in array(&team["agents"]) {
            let id = field(record, "id");
            let session = field(record, "session_id");
            let tasks: Vec<_> = array(&team["tasks"])
                .iter()
                .filter(|task| task["owner_id"] == id || task["verifier_id"] == id)
                .cloned()
                .collect();
            let task = tasks
                .iter()
                .find(|task| matches!(task["state"].as_str(), Some("running" | "verifying")))
                .or_else(|| tasks.last());
            let status = task
                .map(
                    |task| match (task["state"].as_str(), task["owner_id"] == id) {
                        (Some("running"), false) => "queued".into(),
                        (Some("running"), true) if snapshot.baseline_active => {
                            "waiting_agent".into()
                        }
                        (Some("verifying"), true) => "awaiting_verification".into(),
                        (Some("verifying"), false)
                            if snapshot.baseline_active
                                || snapshot
                                    .running_checks
                                    .iter()
                                    .any(|(id, _)| Some(id.as_str()) == task["id"].as_str()) =>
                        {
                            "waiting_agent".into()
                        }
                        _ => field(task, "state"),
                    },
                )
                .unwrap_or_else(|| "queued".into());
            let mut agent = Agent {
                id: id.clone(),
                parent: record["supervisor_id"].as_str().map(str::to_owned),
                role: field(record, "role"),
                provider: field(&record["profile"], "provider"),
                model: field(&record["profile"], "model"),
                status,
                task: task
                    .map(|task| {
                        let description = field(task, "description");
                        if matches!(
                            task["state"].as_str(),
                            Some("running" | "verifying" | "awaiting_verification")
                        ) && array(&task["history"])
                            .last()
                            .is_some_and(|h| !h["source_refresh"].is_null())
                        {
                            format!("Refresh sources: {description}")
                        } else {
                            description
                        }
                    })
                    .unwrap_or_default(),
                group: task
                    .map(|task| field(task, "stage"))
                    .unwrap_or_else(|| "unassigned".into()),
                ..Agent::default()
            };
            if matches!(run["status"].as_str(), Some("running" | "paused"))
                && let Some(round) = array(&run["orchestration_rounds"]).last()
                && round["status"] == "running"
            {
                let reviewing = !round["executor_session"].is_null();
                let active = if reviewing {
                    "orchestrator:verifier"
                } else {
                    "orchestrator"
                };
                if id == active {
                    agent.group = "orchestration".into();
                    agent.task = if round["discovery_status"] == "running" {
                        "Discover CLI capabilities"
                    } else if round["discovery_status"] == "observing_sources" {
                        "Observe project sources for the interview"
                    } else if round["discovery_status"] == "queued" {
                        "CLI capability discovery queued"
                    } else if reviewing {
                        "Verify orchestrator reply"
                    } else {
                        "Respond to user comments/context"
                    }
                    .into();
                    agent.status = if run["status"] == "paused" {
                        "paused"
                    } else if reviewing {
                        "verifying"
                    } else {
                        "running"
                    }
                    .into();
                }
            }
            if matches!(
                run["status"].as_str(),
                Some("paused" | "blocked" | "cancelled")
            ) && matches!(agent.status.as_str(), "running" | "verifying" | "starting")
            {
                agent.status = field(run, "status");
            }
            if let Some(verifier) = record["verifier_id"].as_str() {
                agent
                    .relations
                    .push(("verified by".into(), verifier.into()));
            }
            if let Some(parent) = &agent.parent {
                agent
                    .relations
                    .push(("supervised by".into(), parent.clone()));
            }
            for task in &tasks {
                for dependency in array(&task["dependencies"]) {
                    if let Some(dep) = array(&team["tasks"])
                        .iter()
                        .find(|t| t["id"] == *dependency)
                    {
                        agent
                            .relations
                            .push(("depends on".into(), field(dep, "owner_id")));
                    }
                }
            }
            for edge in array(&team["communicates_with"]) {
                let pair = array(edge);
                if pair.len() == 2 && (pair[0] == id || pair[1] == id) {
                    agent.relations.push((
                        "communicates with".into(),
                        string(if pair[0] == id { &pair[1] } else { &pair[0] }),
                    ));
                }
            }
            agent.details.insert("Overview".into(), pretty(&serde_json::json!({"agent":record,"tasks":tasks,"relations":agent.relations,"activity":"Only persisted task/session observations are shown; silence is not failure."})));
            let sessions: Vec<_> = values(&run["session_ledger"]["sessions"])
                .into_iter()
                .filter(|s| s["scope"]["agent_id"] == id)
                .collect();
            let current = sessions
                .iter()
                .find(|s| s["id"] == session)
                .copied()
                .or_else(|| latest_session(run, &sessions));
            session_details(&mut agent, run, &sessions, current);
            snapshot.agents.push(agent);
        }
        // Old runs still expose their actually observed executor/verifier sessions.
        if snapshot.agents.is_empty() {
            let mut agents: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
            for record in values(&run["session_ledger"]["sessions"]) {
                agents
                    .entry(field(&record["scope"], "agent_id"))
                    .or_default()
                    .push(record);
            }
            for (id, sessions) in agents {
                let current = latest_session(run, &sessions);
                let scope = current.map(|s| &s["scope"]);
                let mut agent = Agent {
                    id,
                    role: scope.map(|s| field(s, "role")).unwrap_or_default(),
                    provider: scope
                        .map(|s| field(s, "provider"))
                        .unwrap_or_else(|| "unknown".into()),
                    status: current
                        .map(|s| field(s, "state"))
                        .unwrap_or_else(|| "unknown".into()),
                    group: scope.map(|s| field(s, "stage")).unwrap_or_default(),
                    ..Agent::default()
                };
                agent.details.insert("Overview".into(), pretty(&serde_json::json!({"current_session":current,"sessions":sessions,
                    "activity":"Session completion is provider output availability, not task verification."})));
                session_details(&mut agent, run, &sessions, current);
                snapshot.agents.push(agent);
            }
            // Ledger scopes name a verifier after its subject (`<agent>:verifier`).
            let ids: BTreeSet<String> = snapshot.agents.iter().map(|a| a.id.clone()).collect();
            for agent in &mut snapshot.agents {
                let verifier = format!("{}:verifier", agent.id);
                if ids.contains(&verifier) {
                    agent.relations.push(("verified by".into(), verifier));
                }
            }
        }
        if snapshot.agents.is_empty() {
            let mut observed = BTreeMap::new();
            for artifact in array(&run["artifacts"]) {
                let session = field(artifact, "session_id");
                if !session.is_empty() {
                    observed.insert(session, "executor");
                }
            }
            for review in array(&run["reviews"]) {
                let session = field(review, "verifier_session");
                if !session.is_empty() {
                    observed.insert(session, "verifier");
                }
            }
            if let Some(session) = run["active_session"].as_str() {
                observed.entry(session.into()).or_insert("executor");
            }
            for (session, role) in observed {
                let mut agent = Agent {
                    id: session.clone(),
                    role: role.into(),
                    provider: field(&run["spec"], "provider"),
                    model: field(&run["spec"], "model"),
                    status: if run["active_session"] == session {
                        snapshot.status.clone()
                    } else {
                        "historical".into()
                    },
                    ..Agent::default()
                };
                agent.details.insert("Overview".into(),format!("Observed {role} session: {session}\nNo delegation record available for this historical session."));
                agent.details.insert(
                    "Session".into(),
                    "Transcript/tool events unavailable from this adapter.".into(),
                );
                agent.details.insert(
                    "Artifacts".into(),
                    pretty(&select_session(&run["artifacts"], &session, "session_id")),
                );
                agent
                    .details
                    .insert("Findings".into(), pretty(&run["reviews"]));
                agent.details.insert("Checks".into(),pretty(&serde_json::json!({"checklist_version":run["checklist_version"],"reviews":run["reviews"],"risks":run["risk_coverage"]})));
                agent
                    .details
                    .insert("Evidence".into(), pretty(&run["evidence"]));
                snapshot.agents.push(agent);
            }
        }
        for review in array(&run["diff_reviews"]) {
            let id = format!("diff-review:{}", field(review, "id"));
            snapshot.agents.retain(|agent| agent.id != id);
            let sessions: Vec<_> = values(&run["session_ledger"]["sessions"])
                .into_iter()
                .filter(|s| s["scope"]["agent_id"] == id)
                .collect();
            let current = latest_session(run, &sessions);
            let mut agent = Agent {
                id,
                role: "Diff reviewer".into(),
                provider: field(&review["profile"], "provider"),
                model: field(&review["profile"], "model"),
                status: if review["state"] == "running" {
                    current
                        .map(|session| field(session, "state"))
                        .unwrap_or_else(|| "starting".into())
                } else {
                    field(review, "state")
                },
                group: "Code review".into(),
                task: format!("Inspect candidate {}", field(review, "diff_hash")),
                ..Agent::default()
            };
            agent.details.insert("Overview".into(), pretty(&serde_json::json!({
                "state":review["state"],"profile":review["profile"],"diff_hash":review["diff_hash"],
                "approval_scope":review["approval_scope"],"diagnostic":review["diagnostic"]
            })));
            session_details(&mut agent, run, &sessions, current);
            agent
                .details
                .insert("Findings".into(), pretty(&review["response"]));
            snapshot.agents.push(agent);
        }
        snapshot.agents.sort_by(|a, b| a.id.cmp(&b.id));
        snapshot
    }
}
