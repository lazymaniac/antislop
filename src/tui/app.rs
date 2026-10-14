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
    pub fn update(&mut self, snapshot: Snapshot) {
        let changed_run = self.snapshot.run_id != snapshot.run_id;
        let new_question = snapshot.awaiting_user_reply.is_some()
            && self.snapshot.awaiting_user_reply != snapshot.awaiting_user_reply;
        self.snapshot = snapshot;
        self.palette_index = self
            .palette_index
            .min(self.palette().len().saturating_sub(1));
        if changed_run {
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
    fn active_editor_mut(&mut self) -> &mut Editor {
        if self.view == View::CreateRun {
            &mut self.spec_editor
        } else if self.comment_target.is_some() {
            &mut self.comment_editor
        } else {
            &mut self.editor
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
        if let Some((round_id, scope_hash)) = &self.snapshot.reviewed_intake_proposal {
            items.push((
                "Accept interview contract and agent routing; plan team".into(),
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
        if let Some(scope_hash) = self.snapshot.scope_hashes.get("review_diff").cloned() {
            items.push((
                "Trigger code review on diff candidate".into(),
                Some(Action::ReviewDiff { scope_hash }),
            ));
        }
        if let Some(scope_hash) = self
            .snapshot
            .scope_hashes
            .get("step_team")
            .filter(|h| !h.is_empty())
            .cloned()
        {
            items.push((
                "Step team execution (run next ready wave)".into(),
                Some(Action::StepTeam { scope_hash }),
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
    fn confirmation(&self, action: Action, label: String) -> View {
        let result = match &action {
            Action::AcceptIntake { .. } => self.snapshot.intake_proposal_details.clone(),
            Action::AcceptTeam { .. } => self.snapshot.reviewed_team_details.clone(),
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
                                effects.push(Effect::BeginConversation(self.editor.text.clone()));
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
                                effects.push(Effect::Action(action));
                            }
                        }
                        _ => self.active_editor_mut().key(key),
                    }
                    return effects;
                }
                if key.code == KeyCode::Esc {
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
                let agent_view = matches!(self.view, View::Agent { .. });
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
                    View::Agent { tab } | View::Run { tab } => {
                        let count = if agent_view {
                            AGENT_TABS.len()
                        } else {
                            RUN_TABS.len()
                        };
                        // Reborrow below because the enum determines the tab count.
                        let _ = tab;
                        match key.code {
                            KeyCode::Right | KeyCode::Tab | KeyCode::Char(']') => {
                                if let View::Agent { tab } | View::Run { tab } = &mut self.view {
                                    *tab = (*tab + 1) % count;
                                }
                                self.scroll = 0;
                            }
                            KeyCode::Left | KeyCode::BackTab | KeyCode::Char('[') => {
                                if let View::Agent { tab } | View::Run { tab } = &mut self.view {
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
                            KeyCode::Char('m') if matches!(self.view, View::Agent { .. }) => {
                                if let Some(id) = &self.selected_agent_id {
                                    self.comment_target = None;
                                    self.message_target = id.clone();
                                    self.editing = true;
                                    self.focus = Focus::Chat;
                                }
                            }
                            KeyCode::Char('r')
                                if !agent_view
                                    && matches!(self.view, View::Run { tab } if tab == 4) =>
                            {
                                if let Some(scope_hash) =
                                    self.snapshot.scope_hashes.get("review_diff").cloned()
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
                            if let Some((round_id, scope_hash)) =
                                &self.snapshot.reviewed_intake_proposal
                            {
                                self.view = self.confirmation(
                                    Action::AcceptIntake {
                                        round_id: round_id.clone(),
                                        scope_hash: scope_hash.clone(),
                                    },
                                    "Accept reviewed interview contract and plan team".into(),
                                );
                                self.scroll = 0;
                            } else if let Some(round_id) = &self.snapshot.reviewed_team_round {
                                self.view = self.confirmation(
                                    Action::AcceptTeam {
                                        round_id: round_id.clone(),
                                        scope_hash: self
                                            .snapshot
                                            .scope_hashes
                                            .get("accept_team")
                                            .cloned()
                                            .unwrap_or_default(),
                                    },
                                    "Accept reviewed agent team".into(),
                                );
                                self.scroll = 0;
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
