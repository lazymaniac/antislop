use super::super::*;

mod recovery;

impl App {
    /// Runs a workflow action routed from the keyboard or command palette.
    /// Typed text is always chat.
    pub fn run_workflow(&mut self, command: Command) {
        match command {
            Command::Requirements => self.open_ledger(),
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
                plan,
                revision_token: None,
            })
        });
    }

    fn request_cancel(&mut self) {
        self.send_command(Some("cancelled"), |conversation| {
            Request::Conversation(crate::lean::daemon::ConversationRequest::Cancel {
                conversation,
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
