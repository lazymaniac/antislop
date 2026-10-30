use super::super::*;

mod recovery;

impl App {
    /// Routes commands to the operation that owns their state changes.
    pub(in crate::lean::tui) fn dispatch(&mut self, text: &str) {
        if self.dispatch_project_command(text) {
            return;
        }
        match text {
            "" => {}
            "/requirements" => self.open_ledger(),
            "/interview" | "/question" => self.open_interview_answer(),
            "/plans" => self.open_plan_history(),
            "/plan" => self.open_current_plan(),
            "/accept" if self.view.workflow_phase.as_deref() == Some("interview") => {
                self.dispatch("/planning");
            }
            "/planning" => self.start_planning(),
            "/accept" => self.accept_plan(),
            "/reject" => self.reject_plan(),
            "/resume" => self.resume_plan(),
            "/cancel" => self.request_cancel(),
            "/pause" => self.set_paused(true),
            "/continue" => self.set_paused(false),
            "/mode" => self.toggle_mode(),
            "/mode autonomous" | "/mode supervised" => {
                self.set_mode(text.ends_with("autonomous"));
            }
            "/config" => self.open_config(),
            _ => self.send_chat(text),
        }
    }

    fn dispatch_project_command(&mut self, text: &str) -> bool {
        if text == "/unblock" {
            self.open_unblock(None);
            return true;
        }
        if let Some(task) = text.strip_prefix("/unblock ") {
            self.open_unblock(Some(task.trim().to_owned()));
            return true;
        }
        match text {
            "/project" => {
                self.open_projects_picker();
                true
            }
            _ if text.starts_with("/project ") => {
                self.open_project_command(text);
                true
            }
            _ => false,
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

    fn open_project_command(&mut self, text: &str) {
        let project = text.trim_start_matches("/project ").trim();
        if !project.is_empty() {
            self.open_project(project);
        }
    }

    fn send_chat(&mut self, text: &str) {
        self.scroll = 0;
        let message = text.to_owned();
        self.send_command(None, |conversation| {
            Request::Conversation(crate::lean::daemon::ConversationRequest::Send {
                conversation,
                text: message,
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
