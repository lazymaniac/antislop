use super::super::*;

impl App {
    pub fn handle_plan_action(&mut self, action: PlanAction) {
        match action {
            PlanAction::Close => self.plan_view = None,
            PlanAction::Accept => self.accept_plan_from_view(),
            PlanAction::Reject => self.open_plan_rejection(),
            PlanAction::None => {}
            PlanAction::CommentLine { line, .. } => self.open_plan_comment(Some(line)),
            PlanAction::CommentWholePlan => self.open_plan_comment(None),
            PlanAction::SubmitReview => self.submit_plan_review(),
            PlanAction::ToggleResolveComment { id, next_status } => {
                self.set_comment(id, &next_status)
            }
        }
    }

    fn accept_plan_from_view(&mut self) {
        if self.view.open_plan_comments().is_empty() {
            self.plan_view = None;
            self.run_workflow(Command::Accept);
        } else {
            self.notice = Some("✗ resolve all plan comments before accepting the plan".into());
        }
    }

    fn open_plan_rejection(&mut self) {
        if let Some(plan) = self.view.plan.clone().filter(|_| self.view.plan_proposed()) {
            self.open_comment_editor(format!("reject:{plan}"));
        } else {
            self.notice = Some("✗ there is no plan to reject".into());
        }
    }

    fn open_plan_comment(&mut self, line: Option<usize>) {
        let Some(plan) = self.view.plan.clone() else {
            return;
        };
        let target = line.map_or_else(
            || format!("plan:{plan}"),
            |line| format!("plan:{plan}:line:{line}"),
        );
        self.open_comment_editor(target);
    }

    fn open_comment_editor(&mut self, target: String) {
        let input = self.get_comment_draft(&target);
        self.editor = Some(Editor { target, input });
    }

    fn submit_plan_review(&self) {
        if let Some(plan) = self.view.plan.clone() {
            self.request(
                Request::Conversation(crate::lean::daemon::ConversationRequest::ReviewComments {
                    conversation: self.conversation.clone(),
                    plan,
                }),
                Some("comments sent to the agent for review"),
            );
        }
    }

    /// The header as drawn, which is also what a click on it is read from.
    pub fn header_line(&self, width: u16) -> Line<'static> {
        let panel_focus = match self.focus {
            Focus::Chat => PanelFocus::Chat,
            Focus::Graph => PanelFocus::Graph,
        };
        let label = self.header_project_label();
        let mut line = view::render_header_with(
            &label,
            &self.conversation,
            self.view.autonomous,
            self.header_status().trim_end_matches(" · "),
            panel_focus,
            width,
            self.view.controls(),
        );
        self.prepend_header_notice(&mut line);
        line
    }

    fn header_project_label(&self) -> String {
        let project = workspace::others_summary(&self.projects, self.active_project).map_or_else(
            || self.project.clone(),
            |others| format!("{} ({others})", self.project),
        );
        let branch = self.view.session_branch.as_ref().or_else(|| {
            self.projects
                .get(self.active_project)
                .and_then(|project| project.branch.as_ref())
        });
        branch.map_or(project.clone(), |branch| {
            format!("{project} · branch: {branch}")
        })
    }

    fn prepend_header_notice(&self, line: &mut Line<'static>) {
        if let Some(notice) = &self.notice {
            line.spans.insert(
                0,
                Span::styled(
                    format!(" {} · ", sanitize(notice)),
                    Theme::from_env().title(),
                ),
            );
        }
    }

    /// The header's status text: the stage, then what is going on.
    pub(in crate::lean::tui) fn header_status(&self) -> String {
        let status = if self.view.paused { "paused" } else { "" };
        let stage = self.view.stage();
        match stage {
            view::Stage::Fresh => format!("1/3 interview · {status}"),
            view::Stage::Interview => format!("1/3 interview · {status}"),
            view::Stage::Planning => format!("2/3 planning · {status}"),
            view::Stage::Review => format!("2/3 plan review · {status}"),
            view::Stage::Executing => format!("3/3 implementation · {status} [Task actions]"),
            // The way out is one click away whenever something is stuck.
            view::Stage::Blocked => format!("{} · {status} [Unblock]", stage.label()),
            _ => format!("{} · {status}", stage.label()),
        }
    }

    pub fn key(&mut self, key: event::KeyEvent) -> bool {
        let keep_running = self.key_inner(key);
        self.text_selection
            .invalidate_context(&self.selection_context());
        if self.selection_blocked() {
            self.text_selection = Default::default();
        }
        keep_running
    }
}
