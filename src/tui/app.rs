use super::{
    backend::{Action, RunChoice},
    editor::Editor,
    model::{AGENT_TABS, RUN_TABS, Snapshot},
    sanitize::sanitize,
};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Chat,
    Team,
    Tasks,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Main,
    Agent {
        tab: usize,
    },
    Run {
        tab: usize,
    },
    Help,
    CommentTargets,
    Annotations,
    Palette,
    Runs,
    CreateRun,
    Confirm {
        action: Action,
        label: String,
        details: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Action(Action),
    SelectRun(String),
    CreateRun(String),
    BeginConversation(String),
    Detach,
}
#[derive(Debug, Clone)]
pub struct Node {
    pub id: String,
    pub depth: usize,
    pub children: bool,
    pub group: String,
}
#[derive(Debug)]
pub struct App {
    pub snapshot: Snapshot,
    pub graph: super::graph::Layout,
    pub runs: Vec<RunChoice>,
    pub selected_agent_id: Option<String>,
    pub focus: Focus,
    pub view: View,
    pub editor: Editor,
    pub comment_editor: Editor,
    pub spec_editor: Editor,
    /// Messages sent from the chat editor this session, oldest first.
    pub history: Vec<String>,
    /// Entry recalled with Up/Down; `None` while editing a fresh draft.
    history_index: Option<usize>,
    /// Draft stashed when history browsing starts, restored past the newest entry.
    history_draft: String,
    pub comment_target: Option<crate::comments::ElementRef>,
    pub annotation_index: usize,
    pub annotation_targets: Vec<crate::comments::Element>,
    pub editing: bool,
    pub message_target: String,
    pub team_filter: String,
    pub chat_filter: String,
    pub searching: bool,
    pub collapsed: BTreeSet<String>,
    pub grouped: bool,
    pub relations: bool,
    pub scroll: usize,
    /// Reading position and query belong to an agent and tab, not the overlay.
    pub agent_inspectors: BTreeMap<(String, usize), super::inspector::State>,
    pub chat_scroll: usize,
    pub list_offset: usize,
    /// Selected task while the task list has focus.
    pub task_index: usize,
    pub palette_index: usize,
    pub run_index: usize,
    pub notice: String,
    pub no_color: bool,
    pub ascii: bool,
    pub width: u16,
    pub height: u16,
    pub sending: bool,
}
impl Default for App {
    fn default() -> Self {
        Self {
            snapshot: Snapshot::default(),
            graph: super::graph::Layout::default(),
            runs: vec![],
            selected_agent_id: None,
            focus: Focus::Chat,
            view: View::Main,
            editor: Editor::default(),
            comment_editor: Editor::default(),
            spec_editor: Editor::default(),
            history: vec![],
            history_index: None,
            history_draft: String::new(),
            comment_target: None,
            annotation_index: 0,
            annotation_targets: vec![],
            editing: false,
            message_target: "orchestrator".into(),
            team_filter: String::new(),
            chat_filter: String::new(),
            searching: false,
            collapsed: BTreeSet::new(),
            grouped: false,
            relations: false,
            scroll: 0,
            agent_inspectors: BTreeMap::new(),
            chat_scroll: 0,
            list_offset: 0,
            task_index: 0,
            palette_index: 0,
            run_index: 0,
            notice: "Connecting to local daemon…".into(),
            no_color: std::env::var_os("NO_COLOR").is_some(),
            ascii: std::env::var_os("ARCHITECT_ASCII").is_some()
                || std::env::var("TERM").is_ok_and(|t| t == "dumb"),
            width: 0,
            height: 0,
            sending: false,
        }
    }
}
impl App {
    pub fn inspector_mut(&mut self, tab: usize) -> &mut super::inspector::State {
        self.agent_inspectors
            .entry((self.selected_agent_id.clone().unwrap_or_default(), tab))
            .or_default()
    }

    pub fn update(&mut self, snapshot: Snapshot) {
        let changed_run = self.snapshot.run_id != snapshot.run_id;
        let new_question = snapshot.awaiting_user_reply.is_some()
            && self.snapshot.awaiting_user_reply != snapshot.awaiting_user_reply;
        self.snapshot = snapshot;
        self.palette_index = self
            .palette_index
            .min(self.palette().len().saturating_sub(1));
        if changed_run {
            self.agent_inspectors.clear();
            self.collapsed.clear();
            self.graph = super::graph::Layout::default();
            self.comment_target = None;
            self.comment_editor = Editor::default();
            self.scroll = 0;
            self.chat_scroll = 0;
            self.list_offset = 0;
            self.task_index = 0;
            self.view = View::Main;
        }
        if new_question
            && self.view == View::Main
            && (changed_run || self.focus == Focus::Chat)
            && !self.searching
            && self.comment_target.is_none()
            && (!self.editing || self.message_target == "orchestrator")
        {
            self.focus = Focus::Chat;
            self.message_target = "orchestrator".into();
            self.editing = true;
            self.chat_scroll = 0;
        }
        self.task_index = self
            .task_index
            .min(self.snapshot.tasks.len().saturating_sub(1));
        self.graph.update(&self.snapshot.agents, self.grouped);
        // Incoming/reordered records cannot move selection to another agent.
        if changed_run
            || self
                .selected_agent_id
                .as_ref()
                .is_none_or(|id| !self.snapshot.agents.iter().any(|a| &a.id == id))
        {
            self.selected_agent_id = self.snapshot.agents.first().map(|a| a.id.clone());
        }
    }
    pub fn active_editor(&self) -> &Editor {
        if self.view == View::CreateRun {
            &self.spec_editor
        } else if self.comment_target.is_some() {
            &self.comment_editor
        } else {
            &self.editor
        }
    }
    pub(crate) fn active_editor_mut(&mut self) -> &mut Editor {
        if self.view == View::CreateRun {
            &mut self.spec_editor
        } else if self.comment_target.is_some() {
            &mut self.comment_editor
        } else {
            &mut self.editor
        }
    }
    /// Shell-style recall of sent messages; comments and the spec path keep plain arrows.
    fn history_applies(&self) -> bool {
        self.view != View::CreateRun && self.comment_target.is_none()
    }
    fn recall_history(&mut self, older: bool) {
        let index = match (self.history_index, older) {
            (None, true) if !self.history.is_empty() => {
                self.history_draft = self.editor.text.clone();
                Some(self.history.len() - 1)
            }
            (None, _) => return,
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) if i + 1 < self.history.len() => Some(i + 1),
            (Some(_), false) => None,
        };
        self.history_index = index;
        let text = match index {
            Some(i) => self.history[i].clone(),
            None => std::mem::take(&mut self.history_draft),
        };
        self.editor.set(text);
    }
    fn record_history(&mut self, text: &str) {
        self.history_index = None;
        self.history_draft.clear();
        if self.history.last().map(String::as_str) != Some(text) {
            self.history.push(text.to_owned());
        }
    }
    pub fn comment_targets(&self) -> Vec<&crate::comments::Element> {
        self.snapshot
            .comment_targets
            .iter()
            .filter(|e| {
                self.selected_agent_id
                    .as_ref()
                    .is_none_or(|id| e.agent_ids.contains(id) || e.agent_ids.is_empty())
            })
            .collect()
    }
    pub fn selected(&self) -> Option<&super::model::Agent> {
        self.selected_agent_id
            .as_ref()
            .and_then(|id| self.snapshot.agents.iter().find(|a| &a.id == id))
    }
    pub fn nodes(&self) -> Vec<Node> {
        let filter = self.team_filter.to_lowercase();
        let matches = |agent: &super::model::Agent| {
            filter.is_empty()
                || format!(
                    "{} {} {} {} {} {}",
                    agent.id, agent.role, agent.provider, agent.model, agent.status, agent.task
                )
                .to_lowercase()
                .contains(&filter)
        };
        let mut nodes = vec![];
        let ids: BTreeSet<_> = self.snapshot.agents.iter().map(|a| a.id.as_str()).collect();
        let mut children: BTreeMap<&str, Vec<&super::model::Agent>> = BTreeMap::new();
        for agent in &self.snapshot.agents {
            if let Some(parent) = agent.parent.as_deref() {
                children.entry(parent).or_default().push(agent);
            }
        }
        let mut visited = BTreeSet::new();
        let mut stack: Vec<_> = self
            .snapshot
            .agents
            .iter()
            .rev()
            .filter(|a| a.parent.as_deref().is_none_or(|p| !ids.contains(p)))
            .map(|a| (a, 0, false))
            .collect();
        while let Some((agent, depth, hidden)) = stack.pop() {
            if !visited.insert(agent.id.as_str()) {
                continue;
            }
            let descendants = children.get(agent.id.as_str());
            if (!hidden || !filter.is_empty()) && matches(agent) {
                nodes.push(Node {
                    id: agent.id.clone(),
                    depth: if filter.is_empty() { depth } else { 0 },
                    children: descendants.is_some_and(|c| !c.is_empty()),
                    group: agent.group.clone(),
                });
            }
            let hidden = hidden || (filter.is_empty() && self.collapsed.contains(&agent.id));
            if let Some(descendants) = descendants {
                for child in descendants.iter().rev() {
                    stack.push((child, depth + 1, hidden));
                }
            }
        }
        // Persisted malformed/cyclic graphs remain inspectable; never recurse.
        for agent in &self.snapshot.agents {
            if !visited.contains(agent.id.as_str()) && matches(agent) {
                nodes.push(Node {
                    id: agent.id.clone(),
                    depth: 0,
                    children: false,
                    group: agent.group.clone(),
                });
            }
        }
        if self.grouped {
            nodes.sort_by(|a, b| a.group.cmp(&b.group));
        }
        nodes
    }
    /// The task the list keeps in view: the selection while focused, otherwise
    /// the first observed active task, then the first unfinished one.
    pub fn followed_task(&self) -> usize {
        if self.focus == Focus::Tasks {
            return self
                .task_index
                .min(self.snapshot.tasks.len().saturating_sub(1));
        }
        self.active_task()
    }
    fn active_task(&self) -> usize {
        let tasks = &self.snapshot.tasks;
        tasks
            .iter()
            .position(|t| super::theme::tone(&t.state) == super::theme::Tone::Active)
            .or_else(|| {
                tasks.iter().position(|t| {
                    !matches!(
                        super::theme::tone(&t.state),
                        super::theme::Tone::Done | super::theme::Tone::Failed
                    )
                })
            })
            .unwrap_or(tasks.len().saturating_sub(1))
    }
    fn select_task(&mut self, index: usize) {
        self.task_index = index.min(self.snapshot.tasks.len().saturating_sub(1));
        // The graph follows the owner of the selected task.
        if let Some(task) = self.snapshot.tasks.get(self.task_index)
            && self.snapshot.agents.iter().any(|a| a.id == task.owner)
        {
            self.selected_agent_id = Some(task.owner.clone());
        }
    }
    fn select_direction(&mut self, dx: i32, dy: i32) {
        let visible: Vec<_> = self.nodes().into_iter().map(|n| n.id).collect();
        if let Some(selected) = &self.selected_agent_id {
            if let Some(id) = self.graph.neighbor(selected, &visible, dx, dy) {
                self.selected_agent_id = Some(id.into());
            }
        } else {
            self.selected_agent_id = visible.first().cloned();
        }
    }
    pub fn palette(&self) -> Vec<(String, Option<Action>)> {
        let mut items = vec![
            ("Pause run".into(), Some(Action::Pause)),
            ("Resume run".into(), Some(Action::Resume)),
            ("Cancel run".into(), Some(Action::Cancel)),
            (
                "Start approved stage / repair".into(),
                Some(Action::Advance),
            ),
            (
                "Approve stage start".into(),
                Some(Action::ApproveStart {
                    scope_hash: self
                        .snapshot
                        .scope_hashes
                        .get("start")
                        .cloned()
                        .unwrap_or_default(),
                }),
            ),
            (
                "Accept verified result".into(),
                Some(Action::AcceptResult {
                    scope_hash: self
                        .snapshot
                        .scope_hashes
                        .get("accept")
                        .cloned()
                        .unwrap_or_default(),
                }),
            ),
            (
                "Approve repair round".into(),
                Some(Action::ApproveRepair {
                    scope_hash: self
                        .snapshot
                        .scope_hashes
                        .get("repair")
                        .cloned()
                        .unwrap_or_default(),
                }),
            ),
            (
                "Reconcile interrupted effects".into(),
                Some(Action::Reconcile {
                    scope_hash: self
                        .snapshot
                        .scope_hashes
                        .get("reconcile")
                        .cloned()
                        .unwrap_or_default(),
                    input_hash: self
                        .snapshot
                        .reconciliation_input_hash
                        .clone()
                        .unwrap_or_default(),
                }),
            ),
            (
                "Switch to supervised mode".into(),
                Some(Action::ChangeMode {
                    mode: "supervised".into(),
                    expected_policy_revision: self.snapshot.policy_revision,
                }),
            ),
            (
                "Switch to autonomous mode".into(),
                Some(Action::ChangeMode {
                    mode: "autonomous".into(),
                    expected_policy_revision: self.snapshot.policy_revision,
                }),
            ),
            ("Requirements".into(), None),
            ("Options / plan".into(), None),
            ("Risks".into(), None),
            ("Quality gates".into(), None),
            ("Diff / comments".into(), None),
            ("Select run".into(), None),
            (
                "Run queued orchestrator turn and independent review".into(),
                Some(Action::Orchestrate {
                    scope_hash: self
                        .snapshot
                        .scope_hashes
                        .get("orchestrate")
                        .cloned()
                        .unwrap_or_default(),
                }),
            ),
        ];
        items.push(("Create run from SPEC.json".into(), None));
        if self.snapshot.delivery_inspect_available {
            items.push((
                "Inspect completed result for source delivery".into(),
                Some(Action::InspectDelivery),
            ));
        }
        if let Some(scope_hash) = &self.snapshot.delivery_scope {
            items.push((
                "Deliver approved changes to original checkout".into(),
                Some(Action::DeliverSource {
                    scope_hash: scope_hash.clone(),
                }),
            ));
        }
        if let Some(recovery) = &self.snapshot.delivery_recovery {
            for path in &recovery.restorable {
                items.push((
                    format!("Restore missing source entry from backup: {path}"),
                    Some(Action::RestoreSourceDeliveryBackup {
                        path: path.clone(),
                        scope_hash: recovery.scope_hash.clone(),
                    }),
                ));
            }
            if recovery.restorable.is_empty() {
                items.push((
                    "Acknowledge observed delivery state without replay".into(),
                    Some(Action::ReconcileSourceDelivery {
                        scope_hash: recovery.scope_hash.clone(),
                        observation_hash: recovery.observation_hash.clone(),
                    }),
                ));
            }
        }
        if self.snapshot.workspace_inspect_available {
            items.push((
                "Observe workspace inputs for private candidate".into(),
                Some(Action::InspectWorkspace),
            ));
        }
        if let Some((selected, scope_hash)) = &self.snapshot.workspace_preparation {
            items.push((
                "Prepare reviewed private workspace candidate".into(),
                Some(Action::PrepareWorkspace {
                    selected: selected.clone(),
                    scope_hash: scope_hash.clone(),
                }),
            ));
        }
        if let Some(scope_hash) = &self.snapshot.workspace_activation {
            items.push((
                "Validate and activate captured private workspace".into(),
                Some(Action::ActivateWorkspace {
                    scope_hash: scope_hash.clone(),
                }),
            ));
        }
        if let Some((round_id, scope_hash)) = &self.snapshot.reviewed_intake_proposal {
            items.push((
                "Accept reviewed interview contract and agent routing".into(),
                Some(Action::AcceptIntake {
                    round_id: round_id.clone(),
                    scope_hash: scope_hash.clone(),
                }),
            ));
        }
        if let Some(round_id) = self.snapshot.reviewed_team_round.as_ref() {
            items.push((
                "Accept reviewed team proposal (tasks remain planned)".into(),
                Some(Action::AcceptTeam {
                    round_id: round_id.clone(),
                    scope_hash: self
                        .snapshot
                        .scope_hashes
                        .get("accept_team")
                        .cloned()
                        .unwrap_or_default(),
                }),
            ));
        }
        if let Some(scope_hash) = self
            .snapshot
            .scope_hashes
            .get("review_diff")
            .filter(|s| !s.is_empty())
            .cloned()
        {
            items.push((
                "Trigger code review on diff candidate".into(),
                Some(Action::ReviewDiff { scope_hash }),
            ));
        }
        if let Some(wave) = &self.snapshot.pending_wave {
            items.push((
                "Run next ready team wave".into(),
                Some(Action::StepTeam {
                    scope_hash: wave.scope_hash.clone(),
                }),
            ));
        }
        for (key, package) in &self.snapshot.team_stage_results {
            items.push((
                format!(
                    "Accept {key} stage results: {}",
                    package.task_ids.join(", ")
                ),
                Some(Action::AcceptTeamStage {
                    stage: package.stage,
                    task_ids: package.task_ids.clone(),
                    scope_hash: package.scope_hash.clone(),
                }),
            ));
        }
        for (task_id, scope_hash) in &self.snapshot.task_verification_retries {
            items.push((
                format!("Retry verifier only: {task_id}"),
                Some(Action::RetryTaskVerification {
                    task_id: task_id.clone(),
                    scope_hash: scope_hash.clone(),
                }),
            ));
        }
        for (task_id, package) in &self.snapshot.task_session_resumes {
            items.push((
                format!("Resume exact native session: {task_id}"),
                Some(Action::ResumeTaskSession {
                    task_id: task_id.clone(),
                    scope_hash: package.scope_hash.clone(),
                }),
            ));
        }
        for p in self.snapshot.task_check_restores.values() {
            items.push((
                format!(
                    "Restore captured check result: {} / {}",
                    p.task_id, p.invocation_id
                ),
                Some(Action::RestoreTaskCheckResult {
                    invocation_id: p.invocation_id.clone(),
                    scope_hash: p.scope_hash.clone(),
                }),
            ));
        }
        for package in self.snapshot.orchestration_recoveries.values() {
            items.push((
                format!(
                    "Recover management {:?} / {:?}: {}",
                    package.kind, package.role, package.round_id
                ),
                Some(Action::RecoverOrchestration {
                    round_id: package.round_id.clone(),
                    kind: package.kind,
                    scope_hash: package.scope_hash.clone(),
                }),
            ));
        }
        for (task_id, package) in &self.snapshot.task_session_replacements {
            items.push((
                format!("New replacement session with handoff: {task_id}"),
                Some(Action::ReplaceTaskSession {
                    task_id: task_id.clone(),
                    scope_hash: package.scope_hash.clone(),
                }),
            ));
        }
        for wave in self.snapshot.wave_effect_reconciliations.values() {
            items.push((
                format!(
                    "Confirm observed wave {} effects: {}",
                    wave.wave_attempt,
                    wave.task_ids.join(", ")
                ),
                Some(Action::ReconcileWaveEffects {
                    task_ids: wave.task_ids.clone(),
                    scope_hash: wave.scope_hash.clone(),
                }),
            ));
        }
        for (task_id, scope_hash) in &self.snapshot.task_effect_reconciliations {
            items.push((
                format!("Confirm observed task file after-state (no replay): {task_id}"),
                Some(Action::ReconcileTaskEffects {
                    task_id: task_id.clone(),
                    scope_hash: scope_hash.clone(),
                }),
            ));
        }
        for (task_id, scope_hash) in &self.snapshot.task_result_restores {
            items.push((
                format!("Restore completed executor result (no replay): {task_id}"),
                Some(Action::RestoreTaskResult {
                    task_id: task_id.clone(),
                    scope_hash: scope_hash.clone(),
                }),
            ));
        }
        for (task_id, scope_hash) in &self.snapshot.task_source_refreshes {
            items.push((
                format!("Refresh sources read-only: {task_id}"),
                Some(Action::RefreshTaskSources {
                    task_id: task_id.clone(),
                    scope_hash: scope_hash.clone(),
                }),
            ));
        }
        items
    }
    /// One explicit next step shared by the chat guide and its approval shortcut.
    pub fn next_approval(&self) -> Option<(Action, String)> {
        if let Some(recovery) = &self.snapshot.delivery_recovery {
            if let Some(path) = recovery.restorable.first() {
                return Some((
                    Action::RestoreSourceDeliveryBackup {
                        path: path.clone(),
                        scope_hash: recovery.scope_hash.clone(),
                    },
                    "Restore a missing source entry from its retained backup".into(),
                ));
            }
            return Some((
                Action::ReconcileSourceDelivery {
                    scope_hash: recovery.scope_hash.clone(),
                    observation_hash: recovery.observation_hash.clone(),
                },
                "Acknowledge observed source delivery versions without replay".into(),
            ));
        }
        if self.snapshot.status == "completed" {
            if let Some(scope_hash) = &self.snapshot.delivery_scope {
                return Some((
                    Action::DeliverSource {
                        scope_hash: scope_hash.clone(),
                    },
                    "Transfer separately approved changes to the original checkout".into(),
                ));
            }
            if self.snapshot.delivery_inspect_available {
                return Some((
                    Action::InspectDelivery,
                    "Inspect the verified candidate for source delivery".into(),
                ));
            }
        }
        if matches!(
            self.snapshot.status.as_str(),
            "running" | "paused" | "completed" | "cancelled"
        ) {
            return None;
        }
        // Preserve captured writes/cohorts before generic reconciliation can
        // invalidate their original attempt. Every choice still opens a pinned
        // confirmation card and is revalidated by the runtime on admission.
        if self.snapshot.task_recovery_preparing || self.snapshot.task_recovery_unavailable {
            return None;
        }
        if let Some(wave) = self.snapshot.wave_effect_reconciliations.values().next() {
            return Some((
                Action::ReconcileWaveEffects {
                    task_ids: wave.task_ids.clone(),
                    scope_hash: wave.scope_hash.clone(),
                },
                "Confirm the observed interrupted wave without repeating writes".into(),
            ));
        }
        if let Some((task_id, scope_hash)) = self.snapshot.task_effect_reconciliations.iter().next()
        {
            return Some((
                Action::ReconcileTaskEffects {
                    task_id: task_id.clone(),
                    scope_hash: scope_hash.clone(),
                },
                "Confirm the observed task effects without repeating writes".into(),
            ));
        }
        if self.snapshot.reconciliation_required {
            return self
                .snapshot
                .reconciliation_input_hash
                .as_ref()
                .filter(|_| self.snapshot.reconciliation_available)
                .zip(
                    self.snapshot
                        .scope_hashes
                        .get("reconcile")
                        .filter(|s| !s.is_empty()),
                )
                .map(|(input_hash, scope_hash)| {
                    (
                        Action::Reconcile {
                            scope_hash: scope_hash.clone(),
                            input_hash: input_hash.clone(),
                        },
                        "Inspect interrupted work before continuing".into(),
                    )
                });
        }
        if let Some((round_id, scope_hash)) = &self.snapshot.reviewed_intake_proposal {
            return Some((
                Action::AcceptIntake {
                    round_id: round_id.clone(),
                    scope_hash: scope_hash.clone(),
                },
                "Accept reviewed interview contract and agent routing".into(),
            ));
        }
        if let Some(round_id) = &self.snapshot.reviewed_team_round
            && !self.snapshot.workspace_required
            && let Some(scope_hash) = self
                .snapshot
                .scope_hashes
                .get("accept_team")
                .filter(|h| !h.is_empty())
        {
            return Some((
                Action::AcceptTeam {
                    round_id: round_id.clone(),
                    scope_hash: scope_hash.clone(),
                },
                "Accept reviewed agent team".into(),
            ));
        }
        if self.snapshot.workspace_required {
            if let Some(scope_hash) = &self.snapshot.workspace_activation {
                return Some((
                    Action::ActivateWorkspace {
                        scope_hash: scope_hash.clone(),
                    },
                    "Validate and activate the captured private workspace".into(),
                ));
            }
            if let Some((selected, scope_hash)) = &self.snapshot.workspace_preparation {
                return Some((
                    Action::PrepareWorkspace {
                        selected: selected.clone(),
                        scope_hash: scope_hash.clone(),
                    },
                    "Approve task-owned local changes and prepare the private workspace".into(),
                ));
            }
            if self.snapshot.workspace_inspect_available {
                return Some((
                    Action::InspectWorkspace,
                    "Observe inputs for a private workspace proposal".into(),
                ));
            }
            return None;
        }
        if let Some(package) = self
            .snapshot
            .team_stage_results
            .values()
            .min_by_key(|p| p.stage)
        {
            return Some((
                Action::AcceptTeamStage {
                    stage: package.stage,
                    task_ids: package.task_ids.clone(),
                    scope_hash: package.scope_hash.clone(),
                },
                format!(
                    "Accept verified {} results",
                    crate::runtime::team::stage::stage_key(package.stage)
                ),
            ));
        }
        if self.snapshot.orchestration_recovery_preparing {
            return None;
        }
        if let Some(package) = self.snapshot.orchestration_recoveries.values().find(|p| {
            p.role == crate::config::RoleKind::Verifier
                && (p.kind == crate::runtime::orchestration_recovery::Kind::Restore
                    || (p.kind == crate::runtime::orchestration_recovery::Kind::Replacement
                        && (p.previous_session.is_none()
                            || p.review_part.is_some()
                            || p.handoff["incomplete_positive_review"] == serde_json::json!(true))))
                && p.retained_output.is_some()
        }) {
            return Some((
                Action::RecoverOrchestration {
                    round_id: package.round_id.clone(),
                    kind: package.kind,
                    scope_hash: package.scope_hash.clone(),
                },
                if package.kind == crate::runtime::orchestration_recovery::Kind::Restore {
                    "Restore captured independent review reports without repeating a native turn"
                } else if package.review_part.is_some() {
                    "Continue the partitioned independent review in a distinct session"
                } else if package.previous_session.is_some() {
                    "Retry the incomplete independent review with its retained evidence"
                } else {
                    "Start the retained proposal's missing independent review"
                }
                .into(),
            ));
        }
        if self.snapshot.queued_orchestrator_turn
            && let Some(scope_hash) = self
                .snapshot
                .scope_hashes
                .get("orchestrate")
                .filter(|h| !h.is_empty())
        {
            return Some((
                Action::Orchestrate {
                    scope_hash: scope_hash.clone(),
                },
                "Continue the queued conversation with the orchestrator".into(),
            ));
        }
        if let Some(package) = self.snapshot.task_check_restores.values().next() {
            return Some((
                Action::RestoreTaskCheckResult {
                    invocation_id: package.invocation_id.clone(),
                    scope_hash: package.scope_hash.clone(),
                },
                "Restore the captured check result without running it again".into(),
            ));
        }
        if let Some((task_id, scope_hash)) = self.snapshot.task_result_restores.iter().next() {
            return Some((
                Action::RestoreTaskResult {
                    task_id: task_id.clone(),
                    scope_hash: scope_hash.clone(),
                },
                "Restore the captured task result without repeating its agent".into(),
            ));
        }
        if let Some(package) = self.snapshot.task_session_resumes.values().next() {
            return Some((
                Action::ResumeTaskSession {
                    task_id: package.task_id.clone(),
                    scope_hash: package.scope_hash.clone(),
                },
                "Continue the interrupted task in its exact native session".into(),
            ));
        }
        if let Some(package) = self.snapshot.task_session_replacements.values().next() {
            return Some((
                Action::ReplaceTaskSession {
                    task_id: package.task_id.clone(),
                    scope_hash: package.scope_hash.clone(),
                },
                "Continue the interrupted task in a new session with its retained handoff".into(),
            ));
        }
        if let Some((task_id, scope_hash)) = self.snapshot.task_verification_retries.iter().next() {
            return Some((
                Action::RetryTaskVerification {
                    task_id: task_id.clone(),
                    scope_hash: scope_hash.clone(),
                },
                "Retry the incomplete independent verification of the retained result".into(),
            ));
        }
        if let Some((task_id, scope_hash)) = self.snapshot.task_source_refreshes.iter().next() {
            return Some((
                Action::RefreshTaskSources {
                    task_id: task_id.clone(),
                    scope_hash: scope_hash.clone(),
                },
                "Refresh the stale task against current sources without repeating writes".into(),
            ));
        }
        self.snapshot.pending_wave.as_ref().map(|wave| {
            (
                Action::StepTeam {
                    scope_hash: wave.scope_hash.clone(),
                },
                format!("Run next wave: {} ready actions", wave.actions.len()),
            )
        })
    }
    fn confirmation(&self, action: Action, label: String) -> View {
        let result = match &action {
            Action::InspectDelivery => "Read-only inspection of the completed, independently verified candidate and the current original checkout.".into(),
            Action::DeliverSource {..} | Action::ReconcileSourceDelivery {..} | Action::RestoreSourceDeliveryBackup {..} => self.snapshot.delivery_details.clone(),
            Action::InspectWorkspace => "Read-only source observation in the background. No candidate, provider or task is started.".into(),
            Action::PrepareWorkspace {..} => self.snapshot.workspace_preparation_details.clone(),
            Action::ActivateWorkspace {..} => self.snapshot.workspace_activation_details.clone(),
            Action::AcceptIntake { .. } => self.snapshot.intake_proposal_details.clone(),
            Action::AcceptTeam { .. } => self.snapshot.reviewed_team_details.clone(),
            Action::StepTeam { .. } => self.snapshot.pending_wave_details.clone(),
            Action::ReviewDiff { .. } => self
                .snapshot
                .run_details
                .get("Diff")
                .cloned()
                .unwrap_or_default(),
            Action::AcceptTeamStage { scope_hash, .. } => self
                .snapshot
                .team_stage_result_details
                .get(scope_hash)
                .cloned()
                .unwrap_or_default(),
            Action::ReconcileWaveEffects { scope_hash, .. } => self
                .snapshot
                .wave_effect_reconciliation_details
                .get(scope_hash)
                .cloned()
                .unwrap_or_default(),
            Action::ReconcileTaskEffects { scope_hash, .. } => self
                .snapshot
                .task_effect_reconciliation_details
                .get(scope_hash)
                .cloned()
                .unwrap_or_default(),
            Action::RestoreTaskResult { scope_hash, .. } => self
                .snapshot
                .task_result_restore_details
                .get(scope_hash)
                .cloned()
                .unwrap_or_default(),
            Action::ResumeTaskSession { scope_hash, .. } => self
                .snapshot
                .task_session_resume_details
                .get(scope_hash)
                .cloned()
                .unwrap_or_default(),
            Action::ReplaceTaskSession { scope_hash, .. } => self
                .snapshot
                .task_session_replacement_details
                .get(scope_hash)
                .cloned()
                .unwrap_or_default(),
            Action::RecoverOrchestration { scope_hash, .. } => self
                .snapshot
                .orchestration_recovery_details
                .get(scope_hash)
                .cloned()
                .unwrap_or_default(),
            Action::RestoreTaskCheckResult { scope_hash, .. } => self
                .snapshot
                .task_check_restore_details
                .get(scope_hash)
                .cloned()
                .unwrap_or_default(),
            Action::Reconcile { .. } => self.snapshot.reconciliation_details.clone(),
            Action::RefreshTaskSources { scope_hash, .. } => self.snapshot.task_source_refresh_details.get(scope_hash).cloned().unwrap_or_default(),
            _ => String::new(),
        };
        let stage = match &action {
            Action::AcceptTeamStage { stage, .. } => crate::runtime::team::stage::stage_key(*stage),
            _ => self.snapshot.stage.clone(),
        };
        View::Confirm {
            action,
            label,
            details: format!(
                "Run: {}\nStage: {} / status: {}\n{}\n\n{}",
                self.snapshot.run_id, stage, self.snapshot.status, self.snapshot.diagnostic, result
            ),
        }
    }
    pub fn handle(&mut self, event: Event) -> Vec<Effect> {
        let mut effects = vec![];
        match event {
            Event::Resize(w, h) => {
                self.width = w;
                self.height = h;
                return effects;
            }
            Event::Paste(text) if self.editing => {
                self.active_editor_mut().insert(&sanitize(&text));
                return effects;
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    return vec![Effect::Detach];
                }
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
                    self.editing = false;
                    self.searching = false;
                    self.view = View::Palette;
                    return effects;
                }
                if self.view == View::CreateRun {
                    match key.code {
                        KeyCode::Esc => {
                            self.view = View::Main;
                            self.editing = false;
                        }
                        KeyCode::Enter | KeyCode::Char('s')
                            if key.code == KeyCode::Enter
                                || key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            if self.sending {
                                self.notice = "Creation already pending; draft retained.".into();
                            } else if self.spec_editor.text.trim().is_empty() {
                                self.notice = "Enter the path to a SPEC.json file.".into();
                            } else {
                                self.sending = true;
                                self.notice = "Creating run from specification…".into();
                                effects
                                    .push(Effect::CreateRun(self.spec_editor.text.trim().into()));
                            }
                        }
                        _ => self.spec_editor.key(key),
                    }
                    return effects;
                }
                if self.searching {
                    if let View::Agent { tab } = self.view {
                        let state = self.inspector_mut(tab);
                        match key.code {
                            KeyCode::Esc => {
                                state.query.clear();
                                self.searching = false;
                            }
                            KeyCode::Enter => self.searching = false,
                            KeyCode::Backspace => {
                                state.query.pop();
                                state.offset = 0;
                            }
                            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                                state.query.push(c);
                                state.offset = 0;
                            }
                            _ => (),
                        }
                        return effects;
                    }
                    let filter = if self.focus == Focus::Team {
                        &mut self.team_filter
                    } else {
                        &mut self.chat_filter
                    };
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => self.searching = false,
                        KeyCode::Backspace => {
                            filter.pop();
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                            filter.push(c)
                        }
                        _ => (),
                    }
                    return effects;
                }
                if self.editing {
                    match key.code {
                        KeyCode::Esc => self.editing = false,
                        KeyCode::Tab | KeyCode::BackTab => {
                            self.editing = false;
                            self.focus = Focus::Team;
                        }
                        KeyCode::Char('s') | KeyCode::Enter
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            if self.sending {
                                self.notice = "Send already pending; draft retained.".into();
                            } else if self.active_editor().text.trim().is_empty() {
                                self.notice =
                                    "Message is empty. Type a message before sending.".into();
                            } else if self.snapshot.run_id.is_empty() {
                                self.sending = true;
                                self.notice = "Starting conversation with the orchestrator…".into();
                                let text = self.editor.text.clone();
                                self.record_history(&text);
                                effects.push(Effect::BeginConversation(text));
                            } else {
                                self.sending = true;
                                self.notice = "Sending message to runtime…".into();
                                let action = if let Some(target) = &self.comment_target {
                                    Action::AddElementComment {
                                        target: target.clone(),
                                        text: self.comment_editor.text.clone(),
                                    }
                                } else if (self.snapshot.intake || self.snapshot.chat_origin)
                                    && self.message_target == "orchestrator"
                                {
                                    Action::ChatMessage {
                                        text: self.editor.text.clone(),
                                    }
                                } else {
                                    Action::SendMessage {
                                        target_agent_id: self.message_target.clone(),
                                        text: self.editor.text.clone(),
                                    }
                                };
                                if self.comment_target.is_none() {
                                    let text = self.editor.text.clone();
                                    self.record_history(&text);
                                }
                                effects.push(Effect::Action(action));
                            }
                        }
                        KeyCode::Up if self.history_applies() && self.editor.on_first_row() => {
                            self.recall_history(true)
                        }
                        KeyCode::Down
                            if self.history_applies()
                                && self.history_index.is_some()
                                && self.editor.on_last_row() =>
                        {
                            self.recall_history(false)
                        }
                        _ => self.active_editor_mut().key(key),
                    }
                    return effects;
                }
                if key.code == KeyCode::Esc {
                    if let View::Agent { tab } = self.view {
                        let state = self.inspector_mut(tab);
                        if !state.query.is_empty() {
                            state.query.clear();
                            state.offset = 0;
                            return effects;
                        }
                    }
                    self.view = View::Main;
                    self.scroll = 0;
                    return effects;
                }
                if key.code == KeyCode::Char('q') {
                    return vec![Effect::Detach];
                }
                if key.code == KeyCode::Char('?') {
                    self.view = View::Help;
                    return effects;
                }
                if key.code == KeyCode::Char('c')
                    && (matches!(self.view, View::Agent { .. })
                        || (self.view == View::Main && self.focus == Focus::Team))
                {
                    self.annotation_targets = self.comment_targets().into_iter().cloned().collect();
                    self.view = View::CommentTargets;
                    self.annotation_index = 0;
                    return effects;
                }
                if key.code == KeyCode::Char('u')
                    && (matches!(self.view, View::Agent { .. })
                        || (self.view == View::Main && self.focus == Focus::Team))
                {
                    self.view = View::Annotations;
                    self.annotation_index = 0;
                    return effects;
                }
                if let View::Agent { tab } = self.view {
                    match key.code {
                        KeyCode::Right | KeyCode::Tab | KeyCode::Char(']') => {
                            self.view = View::Agent {
                                tab: (tab + 1) % AGENT_TABS.len(),
                            };
                        }
                        KeyCode::Left | KeyCode::BackTab | KeyCode::Char('[') => {
                            self.view = View::Agent {
                                tab: (tab + AGENT_TABS.len() - 1) % AGENT_TABS.len(),
                            };
                        }
                        KeyCode::Char('/') => self.searching = true,
                        KeyCode::Char('v') => {
                            let state = self.inspector_mut(tab);
                            state.raw = !state.raw;
                            state.offset = 0;
                        }
                        KeyCode::Char('m') => {
                            if let Some(id) = &self.selected_agent_id {
                                self.comment_target = None;
                                self.message_target = id.clone();
                                self.editing = true;
                                self.focus = Focus::Chat;
                            }
                        }
                        _ => {
                            let state = self.inspector_mut(tab);
                            let page = state.page.max(1);
                            state.offset = match key.code {
                                KeyCode::Down => state.offset.saturating_add(1),
                                KeyCode::Up => state.offset.saturating_sub(1),
                                KeyCode::PageDown => state.offset.saturating_add(page),
                                KeyCode::PageUp => state.offset.saturating_sub(page),
                                KeyCode::Home => 0,
                                KeyCode::End => state.max_offset(),
                                _ => state.offset,
                            }
                            .min(state.max_offset());
                        }
                    }
                    return effects;
                }
                match &mut self.view {
                    View::Confirm { action, .. } => match key.code {
                        KeyCode::Enter => {
                            effects.push(Effect::Action(action.clone()));
                            self.view = View::Main;
                            self.scroll = 0;
                        }
                        KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
                        KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                        KeyCode::PageDown => {
                            self.scroll = self
                                .scroll
                                .saturating_add(self.height.saturating_sub(8) as usize)
                        }
                        KeyCode::PageUp => {
                            self.scroll = self
                                .scroll
                                .saturating_sub(self.height.saturating_sub(8) as usize)
                        }
                        _ => (),
                    },
                    View::CommentTargets => match key.code {
                        KeyCode::Up => {
                            self.annotation_index = self.annotation_index.saturating_sub(1)
                        }
                        KeyCode::Down => {
                            self.annotation_index = (self.annotation_index + 1)
                                .min(self.annotation_targets.len().saturating_sub(1))
                        }
                        KeyCode::Enter => {
                            if let Some(target) = self
                                .annotation_targets
                                .get(self.annotation_index)
                                .map(|e| e.reference.clone())
                            {
                                if self.comment_target.as_ref() != Some(&target) {
                                    self.comment_editor = Editor::default();
                                }
                                self.comment_target = Some(target);
                                self.editing = true;
                                self.focus = Focus::Chat;
                                self.view = View::Main;
                            }
                        }
                        _ => (),
                    },
                    View::Annotations => match key.code {
                        KeyCode::Up => {
                            self.annotation_index = self.annotation_index.saturating_sub(1)
                        }
                        KeyCode::Down => {
                            self.annotation_index = (self.annotation_index + 1).min(
                                self.snapshot
                                    .element_comments
                                    .iter()
                                    .filter(|c| !c.deleted)
                                    .count()
                                    .saturating_sub(1),
                            )
                        }
                        KeyCode::Char('r') | KeyCode::Char('d') => {
                            if let Some(comment) = self
                                .snapshot
                                .element_comments
                                .iter()
                                .filter(|c| !c.deleted)
                                .nth(self.annotation_index)
                            {
                                let action = if key.code == KeyCode::Char('d') {
                                    Action::DeleteElementComment {
                                        comment_id: comment.id.clone(),
                                    }
                                } else {
                                    Action::ResolveElementComment {
                                        comment_id: comment.id.clone(),
                                        resolved: !comment.resolved,
                                    }
                                };
                                effects.push(Effect::Action(action));
                            }
                        }
                        _ => (),
                    },
                    View::Palette => match key.code {
                        KeyCode::Up => self.palette_index = self.palette_index.saturating_sub(1),
                        KeyCode::Down => {
                            self.palette_index =
                                (self.palette_index + 1).min(self.palette().len() - 1)
                        }
                        KeyCode::Enter => {
                            let (label, action) = self.palette()[self.palette_index].clone();
                            if let Some(action) = action {
                                self.view = self.confirmation(action, label);
                                self.scroll = 0;
                            } else if label == "Create run from SPEC.json" {
                                self.view = View::CreateRun;
                                self.editing = true;
                            } else if label == "Select run" {
                                self.view = View::Runs;
                            } else if label == "Diff / comments" {
                                self.view = View::Run { tab: 4 };
                                self.scroll = 0;
                            } else {
                                self.view = View::Run {
                                    tab: self.palette_index.saturating_sub(10),
                                };
                                self.scroll = 0;
                            }
                        }
                        _ => (),
                    },
                    View::Runs => match key.code {
                        KeyCode::Up => self.run_index = self.run_index.saturating_sub(1),
                        KeyCode::Down => {
                            self.run_index =
                                (self.run_index + 1).min(self.runs.len().saturating_sub(1))
                        }
                        KeyCode::Enter => {
                            if let Some(run) = self.runs.get(self.run_index) {
                                effects.push(Effect::SelectRun(run.id.clone()));
                                self.view = View::Main;
                            }
                        }
                        _ => (),
                    },
                    View::Agent { .. } => unreachable!("handled above"),
                    View::Run { .. } => {
                        let count = RUN_TABS.len();
                        match key.code {
                            KeyCode::Right | KeyCode::Tab | KeyCode::Char(']') => {
                                if let View::Run { tab } = &mut self.view {
                                    *tab = (*tab + 1) % count;
                                }
                                self.scroll = 0;
                            }
                            KeyCode::Left | KeyCode::BackTab | KeyCode::Char('[') => {
                                if let View::Run { tab } = &mut self.view {
                                    *tab = (*tab + count - 1) % count;
                                }
                                self.scroll = 0;
                            }
                            KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
                            KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                            KeyCode::PageDown => {
                                self.scroll = self
                                    .scroll
                                    .saturating_add(self.height.saturating_sub(8) as usize)
                            }
                            KeyCode::PageUp => {
                                self.scroll = self
                                    .scroll
                                    .saturating_sub(self.height.saturating_sub(8) as usize)
                            }
                            KeyCode::Char('r') if matches!(self.view, View::Run { tab } if tab == 4) => {
                                if let Some(scope_hash) = self
                                    .snapshot
                                    .scope_hashes
                                    .get("review_diff")
                                    .filter(|s| !s.is_empty())
                                    .cloned()
                                {
                                    self.view = self.confirmation(Action::ReviewDiff { scope_hash }, "Authorize on-demand code review of workspace diff candidate".into());
                                    self.scroll = 0;
                                } else {
                                    self.notice =
                                        "Diff review scope not available for this run state."
                                            .into();
                                }
                            }
                            _ => (),
                        }
                    }
                    View::CreateRun => unreachable!("handled above"),
                    View::Main => match key.code {
                        KeyCode::Char('a') => {
                            if let Some((action, label)) = self.next_approval() {
                                self.view = self.confirmation(action, label);
                                self.scroll = 0;
                            } else {
                                self.notice = "No current approval is ready. Check the chat and task findings.".into();
                            }
                        }
                        KeyCode::Char('n') => {
                            self.view = View::CreateRun;
                            self.editing = true;
                        }
                        KeyCode::Tab => {
                            self.focus = match self.focus {
                                Focus::Chat => Focus::Team,
                                Focus::Team => Focus::Tasks,
                                Focus::Tasks => Focus::Chat,
                            };
                            if self.focus == Focus::Tasks {
                                self.task_index = self.active_task();
                            }
                        }
                        KeyCode::BackTab => {
                            self.focus = match self.focus {
                                Focus::Chat => Focus::Tasks,
                                Focus::Team => Focus::Chat,
                                Focus::Tasks => Focus::Team,
                            };
                            if self.focus == Focus::Tasks {
                                self.task_index = self.active_task();
                            }
                        }
                        KeyCode::Char('/') if self.focus != Focus::Tasks => self.searching = true,
                        KeyCode::Enter
                            if self.focus == Focus::Chat
                                && self.snapshot.awaiting_user_reply.is_some() =>
                        {
                            self.comment_target = None;
                            self.message_target = "orchestrator".into();
                            self.editing = true;
                            self.chat_scroll = 0;
                        }
                        KeyCode::Char('i') => {
                            self.comment_target = None;
                            self.message_target = "orchestrator".into();
                            self.editing = true;
                            self.focus = Focus::Chat;
                        }
                        KeyCode::Char('b') => {
                            self.comment_target = None;
                            self.message_target = "*".into();
                            self.editing = true;
                            self.focus = Focus::Chat;
                        }
                        KeyCode::Char('m') if self.focus == Focus::Team => {
                            if let Some(id) = &self.selected_agent_id {
                                self.comment_target = None;
                                self.message_target = id.clone();
                                self.editing = true;
                                self.focus = Focus::Chat;
                            }
                        }
                        KeyCode::Up if self.focus == Focus::Team => self.select_direction(0, -1),
                        KeyCode::Down if self.focus == Focus::Team => self.select_direction(0, 1),
                        KeyCode::Left if self.focus == Focus::Team => self.select_direction(-1, 0),
                        KeyCode::Right if self.focus == Focus::Team => self.select_direction(1, 0),
                        KeyCode::Enter if self.focus == Focus::Team => {
                            if self.selected().is_some() {
                                self.view = View::Agent { tab: 0 };
                                self.scroll = 0;
                            }
                        }
                        KeyCode::Char('g') if self.focus == Focus::Team => {
                            self.grouped = !self.grouped
                        }
                        KeyCode::Char('r') if self.focus == Focus::Team => {
                            self.relations = !self.relations
                        }
                        KeyCode::Char('C') if self.focus == Focus::Team => {
                            for agent in &self.snapshot.agents {
                                if matches!(
                                    agent.status.as_str(),
                                    "verified" | "completed" | "cancelled"
                                ) {
                                    self.collapsed.insert(agent.id.clone());
                                }
                            }
                        }
                        KeyCode::Up if self.focus == Focus::Tasks => {
                            self.select_task(self.task_index.saturating_sub(1))
                        }
                        KeyCode::Down if self.focus == Focus::Tasks => {
                            self.select_task(self.task_index + 1)
                        }
                        KeyCode::PageUp if self.focus == Focus::Tasks => {
                            self.select_task(self.task_index.saturating_sub(5))
                        }
                        KeyCode::PageDown if self.focus == Focus::Tasks => {
                            self.select_task(self.task_index + 5)
                        }
                        KeyCode::Home if self.focus == Focus::Tasks => self.select_task(0),
                        KeyCode::End if self.focus == Focus::Tasks => self.select_task(usize::MAX),
                        KeyCode::Enter if self.focus == Focus::Tasks => {
                            self.select_task(self.task_index);
                            if self.selected().is_some() {
                                self.view = View::Agent { tab: 0 };
                                self.scroll = 0;
                            }
                        }
                        KeyCode::Up => self.chat_scroll = self.chat_scroll.saturating_add(1),
                        KeyCode::Down => self.chat_scroll = self.chat_scroll.saturating_sub(1),
                        KeyCode::PageUp => self.chat_scroll = self.chat_scroll.saturating_add(10),
                        KeyCode::PageDown => self.chat_scroll = self.chat_scroll.saturating_sub(10),
                        _ => (),
                    },
                    View::Help => (),
                }
            }
            _ => (),
        }
        effects
    }
    pub fn action_result(&mut self, action: &Action, result: std::result::Result<String, String>) {
        if let Action::AddElementComment { target, text } = action {
            self.sending = false;
            if result.is_ok()
                && self.comment_target.as_ref() == Some(target)
                && self.comment_editor.text == *text
            {
                self.comment_editor = Editor::default();
                self.comment_target = None;
                self.editing = false;
            }
        }
        if let Action::SendMessage { text, .. } | Action::ChatMessage { text } = action {
            self.sending = false;
            if result.is_ok() && self.editor.text == *text {
                self.editor.set(String::new());
                if matches!(action, Action::ChatMessage { .. }) {
                    self.snapshot.awaiting_user_reply = None;
                    self.chat_scroll = 0;
                }
            }
        }
        self.notice = sanitize(&result.unwrap_or_else(|error| format!("Action rejected: {error}")));
    }
}
