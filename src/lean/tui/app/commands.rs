use super::super::*;

mod recovery;

impl App {
    /// Runs a workflow action of the UI; every one has a shortcut, and the
    /// command palette (Ctrl+P) lists them all. Typed text is always chat.
    pub fn run_workflow(&mut self, command: Command) {
        match command {
            Command::Requirements => self.open_ledger(),
            Command::InterviewQuestion => self.open_interview_answer(),
            Command::Plans => self.open_plan_history(),
            Command::Plan => self.open_current_plan(),
            Command::Accept if self.view.workflow_phase.as_deref() == Some("interview") => {
                self.start_planning();
            }
            Command::ApproveRequirements => self.start_planning(),
            Command::Accept => self.accept_plan(),
            Command::Reject => self.reject_plan(),
            Command::Resume => self.resume_plan(),
            Command::Cancel => self.request_cancel(),
            Command::Pause => self.set_paused(true),
            Command::Continue => self.set_paused(false),
            Command::PauseOrContinue => self.set_paused(!self.view.paused),
            Command::ToggleMode => self.toggle_mode(),
            Command::Autonomous => self.set_mode(true),
            Command::Supervised => self.set_mode(false),
            Command::Config => self.open_config(),
            Command::TaskActions => self.open_unblock(None),
            Command::NewConversation => self.new_conversation(),
            Command::SwitchConversation => self.open_conversation_picker(),
            Command::Projects => self.open_projects_picker(),
            _ => {}
        }
    }

    fn open_interview_answer(&mut self) {
        self.sync_interview();
        if let Some(form) = self
            .interview_forms
            .get_mut(&self.conversation)
            .filter(|form| !form.sent)
        {
            form.visible = true;
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

    fn start_planning(&mut self) {
        if self.view.workflow_phase.as_deref() != Some("interview") {
            self.notice = Some("✗ this session is not in interview".into());
        } else if self.view.requirements_summary.is_empty()
            || self.sending
            || !self.input.text.is_empty()
        {
            self.notice =
                Some("✗ send your answers and wait for the requirements summary first".into());
        } else {
            self.send_command(
                Some("requirements approved; planning started"),
                |conversation| {
                    Request::Conversation(crate::lean::daemon::ConversationRequest::Send {
                        conversation,
                        text: "/planning".into(),
                    })
                },
            );
        }
    }

    fn accept_plan(&mut self) {
        let Some(plan) = self.view.plan.clone().filter(|_| self.view.plan_proposed()) else {
            self.notice = Some("✗ there is no plan to accept".into());
            return;
        };
        if !self.view.open_plan_comments().is_empty() {
            self.notice = Some(
                "✗ send the draft comments (Review) and resolve the rest before accepting".into(),
            );
            return;
        }
        self.plan_view = None;
        self.send_command(Some("plan accepted"), |conversation| {
            Request::Conversation(crate::lean::daemon::ConversationRequest::Accept {
                conversation,
                plan,
            })
        });
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
                plan,
                reason: None,
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
                plan,
            })
        });
    }

    fn request_cancel(&mut self) {
        self.send_command(Some("cancelled"), |conversation| {
            Request::Conversation(crate::lean::daemon::ConversationRequest::Cancel { conversation })
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

    fn toggle_mode(&mut self) {
        let autonomous = self.view.autonomous != Some(true);
        self.set_mode(autonomous);
    }

    fn set_mode(&mut self, autonomous: bool) {
        self.send_command(None, |conversation| {
            Request::Conversation(crate::lean::daemon::ConversationRequest::Mode {
                conversation,
                autonomous,
            })
        });
    }

    fn send_command(
        &mut self,
        notice: Option<&'static str>,
        request: impl FnOnce(String) -> Request,
    ) {
        let conversation = self.conversation.clone();
        self.request(request(conversation), notice);
    }
}
