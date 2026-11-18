use super::super::*;

mod recovery;

impl App {
    /// Runs a workflow action routed from the keyboard or command palette.
    /// Typed text is always chat.
    pub fn run_workflow(&mut self, command: Command) {
        match command {
            Command::InterviewQuestion => self.open_interview_answer(),
            Command::Plans => self.open_plan_history(),
            Command::Plan => self.open_current_plan(),
            Command::Reject => self.reject_plan(),
            Command::Resume => self.resume_plan(),
            Command::Cancel => self.request_cancel(),
            Command::Pause => self.set_paused(true),
            Command::Continue => self.set_paused(false),
            Command::PauseOrContinue => self.set_paused(!self.view.paused),
            Command::Config => self.open_config(),
            Command::TaskActions => self.open_unblock(None),
            Command::ApprovalsSupervised => self.set_approval_mode("supervised"),
            Command::ApprovalsAutonomous => self.set_approval_mode("autonomous"),
            Command::PendingApprovals => self.show_pending_approvals(),
            Command::TakeControl => self.take_control(),
            Command::NewConversation => self.new_conversation(),
            Command::SwitchConversation => self.open_conversation_picker(),
            Command::Projects => self.open_projects_picker(),
            Command::SelectTeam(team) => self.apply_rigor_team(team),
            Command::OpenRigorPicker => {
                if self.view.team_recommended {
                    self.rigor_recommendations_shown
                        .insert(self.conversation.clone());
                }
                self.picker = Some(Picker::rigor(&self.view));
            }
            Command::EditTask => self.open_flow_edit(),
            Command::AddDependentTask => self.open_flow_add_dependent(),
            Command::SplitTask => self.open_flow_split(),
            Command::HoldTask => self.toggle_flow_hold(),
            Command::RetireTask => self.open_flow_retire(),
            Command::SteerAgent => self.open_live_steer(),
            Command::AcceptPlan => self.accept_plan(),
            Command::DirectExecute => self.trigger_direct_execute(),
            Command::LinkTask => self.toggle_flow_link(),
            _ => {}
        }
    }

    pub fn trigger_direct_execute(&mut self) {
        let trimmed = self.input.text.trim().to_owned();
        if trimmed.is_empty() {
            self.input = InputBuffer::with_text("/direct ");
            self.focus = Focus::Chat;
            self.notice = Some("⚡ Wpisz polecenie po /direct i naciśnij Enter".into());
        } else if trimmed.starts_with("/direct") {
            self.submit();
        } else {
            self.input = InputBuffer::with_text(format!("/direct {trimmed}"));
            self.submit();
        }
    }

    pub fn toggle_flow_link(&mut self) {
        let Some(target) = self.active_graph_task().cloned() else {
            self.notice = Some("✗ brak wybranego zadania w grafie".into());
            return;
        };
        if let Some(source_id) = self.graph_link_source.take() {
            if source_id == target.id {
                self.notice = Some("Anulowano łączenie krawędzi (wybrano to samo zadanie).".into());
                return;
            }
            match flow_editor::toggle_dependency_edge_revision(&self.view, &source_id, &target.id) {
                Ok((revision, message)) => {
                    self.notice = Some(format!("✓ {message}"));
                    self.send_flow_revision(revision, false, target.id, target.title);
                }
                Err(error) => self.notice = Some(format!("✗ {error}")),
            }
        } else {
            self.focus = Focus::Graph;
            self.graph_link_source = Some(target.id.clone());
            self.notice = Some(format!(
                "🔗 Łączenie od {}: wybierz zadanie docelowe i wciśnij Enter/l (Esc anuluje)",
                target.id
            ));
        }
    }

    pub(in crate::lean::tui) fn sync_rigor_recommendation(&mut self) {
        if !self.view.team_recommended
            || self.observer
            || self
                .rigor_recommendations_shown
                .contains(&self.conversation)
            || self.native_focused()
            || self.picker.is_some()
            || self.help.is_some()
            || self.editor.is_some()
            || self
                .details
                .as_ref()
                .is_some_and(|details| details.editing || details.searching)
            || self.flow_editor.is_some()
            || self
                .config_editor
                .as_ref()
                .is_some_and(|editor| editor.visible)
            || self.plan_history.is_some()
            || self.unblock.is_some()
            || self.graph_filters.is_some()
            || self.approval.is_some()
            || self.interview_visible()
        {
            return;
        }
        self.rigor_recommendations_shown
            .insert(self.conversation.clone());
        self.picker = Some(Picker::rigor(&self.view));
    }

    pub fn apply_rigor_team(&mut self, team: crate::lean::tasks::Team) {
        let name = team.name().to_owned();
        self.send_command(Some("workflow rigor updated"), move |conversation| {
            Request::Conversation(crate::lean::daemon::ConversationRequest::SelectTeam {
                conversation,
                team: name,
                reason: "Selected by user in UI".into(),
            })
        });
    }

    pub fn active_graph_task(&self) -> Option<&view::TaskRow> {
        let id = self
            .selected
            .split_once(':')
            .map(|(_, id)| id)
            .unwrap_or(&self.selected);
        self.view
            .tasks
            .iter()
            .find(|task| task.id == id)
            .or_else(|| self.view.tasks.first())
    }

    pub fn open_flow_edit(&mut self) {
        let Some(task) = self.active_graph_task().cloned() else {
            self.notice = Some("✗ brak wybranego zadania w grafie do edycji".into());
            return;
        };
        self.flow_editor = Some(flow_editor::FlowEditor::edit_task(&self.view, &task));
    }

    pub fn open_flow_add_dependent(&mut self) {
        if self.view.plan.is_none() && self.view.tasks.is_empty() {
            self.notice = Some("✗ brak aktywnego planu do dodania zadania".into());
            return;
        };
        let after_id = self
            .active_graph_task()
            .map(|t| t.id.clone())
            .unwrap_or_default();
        self.flow_editor = Some(flow_editor::FlowEditor::add_dependent(
            &self.view, &after_id,
        ));
    }

    pub fn open_flow_split(&mut self) {
        let Some(task) = self.active_graph_task().cloned() else {
            self.notice = Some("✗ brak wybranego zadania w grafie do podziału".into());
            return;
        };
        self.flow_editor = Some(flow_editor::FlowEditor::split_task(&self.view, &task));
    }

    pub fn open_flow_retire(&mut self) {
        let Some(task) = self.active_graph_task().cloned() else {
            self.notice = Some("✗ brak wybranego zadania w grafie do wycofania".into());
            return;
        };
        self.flow_editor = Some(flow_editor::FlowEditor::retire_task(&task));
    }

    pub fn toggle_flow_hold(&mut self) {
        let Some(task) = self.active_graph_task().cloned() else {
            self.notice = Some("✗ brak wybranego zadania w grafie".into());
            return;
        };
        match flow_editor::toggle_hold_revision(&self.view, &task) {
            Ok((revision, request_ai_review)) => {
                self.send_flow_revision(revision, request_ai_review, task.id, task.title);
            }
            Err(error) => self.notice = Some(format!("✗ {error}")),
        }
    }

    pub fn open_live_steer(&mut self) {
        let key = if self.selected.contains(':') || self.selected == ORCHESTRATOR {
            self.selected.clone()
        } else if let Some(task) = self.active_graph_task() {
            format!("worker:{}", task.id)
        } else {
            ORCHESTRATOR.to_owned()
        };
        self.graph_expanded = None;
        self.open_details(key);
        if let Some(details) = &mut self.details {
            details.editing = true;
            let composer_key = details.composer_key();
            details.drafts.borrow_mut().entry(composer_key).or_default();
        }
    }

    pub fn send_flow_revision(
        &self,
        revision: crate::lean::store::PlanRevision,
        request_ai_review: bool,

        task_id: String,
        task_title: String,
    ) {
        let client = self.client.clone();
        let updates = self.updates.clone();
        let conversation = self.conversation.clone();
        let generation = self.generation;
        let rev_for_req = revision.clone();
        std::thread::spawn(move || {
            let reply = client
                .call(&Request::Conversation(
                    crate::lean::daemon::ConversationRequest::RevisePlan {
                        conversation: conversation.clone(),
                        operation_id: Some(crate::id()),
                        revision: rev_for_req,
                        request_ai_review,
                    },
                ))
                .map_err(|error| error.to_string());
            let _ = updates.send(Update::PlanRevisionResult {
                conversation,
                generation,
                revision,
                request_ai_review,
                task_id,
                task_title,
                reply,
            });
        });
    }

    fn open_interview_answer(&mut self) {
        self.sync_interview();
        if let Some(form) = self
            .interview_forms
            .get_mut(&self.conversation)
            .filter(|form| !form.sent)
        {
            form.visible = true;
            self.plan_view = None;
            self.diff = None;
            self.details = None;
        } else {
            self.notice = Some("No question is waiting for an answer.".into());
        }
    }

    fn open_plan_history(&mut self) {
        self.plan_history = Some(planhistory::Browser::new());
        self.load_plan_history(0);
    }

    fn open_current_plan(&mut self) {
        if self.view.plan.is_some() || !self.view.tasks.is_empty() || !self.view.plan_doc.is_empty()
        {
            let id = self.view.plan.clone().unwrap_or_else(|| "plan".into());
            self.plan_view = Some(PlanView::new(id));
        } else {
            self.notice = Some("✗ there is no plan to view".into());
        }
    }

    fn reject_plan(&mut self) {
        let Some(plan) = self.view.plan.clone().filter(|_| self.view.plan_proposed()) else {
            self.notice = Some("✗ there is no plan to reject".into());
            return;
        };
        self.plan_view = None;
        self.send_command(Some("plan rejected"), |conversation| {
            Request::Conversation(crate::lean::daemon::ConversationRequest::Reject {
                conversation,
                operation_id: Some(crate::id()),
                plan,
                reason: None,
                revision_token: None,
            })
        });
    }

    pub(super) fn accept_plan(&mut self) {
        let Some(plan) = self.view.plan.clone().filter(|_| self.view.plan_proposed()) else {
            self.notice = Some("✗ there is no plan to accept".into());
            return;
        };
        if !self.view.open_plan_comments().is_empty() {
            self.notice =
                Some("✗ send the draft comments (Review) and resolve the rest first".into());
            return;
        }
        self.plan_view = None;
        self.send_command(Some("plan accepted"), |conversation| {
            Request::Conversation(crate::lean::daemon::ConversationRequest::Resume {
                conversation,
                operation_id: Some(crate::id()),
                plan,
                revision_token: None,
            })
        });
    }

    fn resume_plan(&mut self) {
        let Some(plan) = self
            .view
            .plan
            .clone()
            .filter(|_| self.view.plan_resumable())
        else {
            self.notice = Some("✗ there is nothing to resume".into());
            return;
        };
        self.send_command(Some("plan resumed"), |conversation| {
            Request::Conversation(crate::lean::daemon::ConversationRequest::Resume {
                conversation,
                operation_id: Some(crate::id()),
                plan,
                revision_token: None,
            })
        });
    }

    fn take_control(&mut self) {
        self.observer = false;
        self.send_command(
            Some("this window steers the conversation"),
            |conversation| {
                Request::Conversation(crate::lean::daemon::ConversationRequest::TakeControl {
                    conversation,
                })
            },
        );
    }

    fn request_cancel(&mut self) {
        self.send_command(Some("cancelled"), |conversation| {
            Request::Conversation(crate::lean::daemon::ConversationRequest::Cancel {
                conversation,
                operation_id: Some(crate::id()),
                revision_token: None,
            })
        });
    }

    fn set_paused(&mut self, paused: bool) {
        self.send_command(None, |conversation| {
            Request::Conversation(crate::lean::daemon::ConversationRequest::Pause {
                conversation,
                paused,
            })
        });
    }

    pub(in crate::lean::tui) fn send_command(
        &mut self,
        notice: Option<&'static str>,
        request: impl FnOnce(String) -> Request,
    ) {
        let conversation = self.conversation.clone();
        self.request(request(conversation), notice);
    }
}
