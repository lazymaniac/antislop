use super::super::*;

fn selected_session_action_request(
    view: &mut View,
    conversation: &str,
    index: usize,
) -> Option<(
    crate::lean::daemon::SessionActionKind,
    crate::lean::daemon::Request,
)> {
    let kind = view.cards.iter_mut().rev().find_map(|card| {
        if let crate::lean::tui::TranscriptCard::Completion {
            actions, selected, ..
        } = card
        {
            *selected = index;
            actions.get(index).map(|action| action.kind)
        } else {
            None
        }
    })?;
    Some((
        kind,
        crate::lean::daemon::Request::Session(crate::lean::daemon::SessionRequest::SessionAction {
            conversation: conversation.to_owned(),
            action: kind,
        }),
    ))
}

impl App {
    pub(in crate::lean::tui) fn select_session_action(&mut self, index: usize) -> bool {
        if let Some(crate::lean::tui::TranscriptCard::Completion { actions, .. }) = self
            .view
            .cards
            .iter()
            .rev()
            .find(|card| matches!(card, crate::lean::tui::TranscriptCard::Completion { .. }))
            && actions.iter().any(|action| {
                matches!(
                    action.result,
                    Some(crate::lean::daemon::SessionActionResult::Queued { .. })
                )
            })
        {
            self.notice = Some(
                "A session action is already queued; wait for its result before choosing another."
                    .into(),
            );
            return true;
        }
        let Some((kind, request)) =
            selected_session_action_request(&mut self.view, &self.conversation, index)
        else {
            return false;
        };
        self.view.set_session_action_result(
            kind,
            crate::lean::daemon::SessionActionResult::Queued { message: 0 },
        );
        let conversation = self.conversation.clone();
        let client = self.client.clone();
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let result = client.call(&request).and_then(|value| {
                serde_json::from_value::<crate::lean::daemon::SessionActionResult>(value)
                    .map_err(crate::Error::from)
            });
            let result = match result {
                Ok(result) => result,
                Err(error) => crate::lean::daemon::SessionActionResult::Failed {
                    details: error.to_string(),
                },
            };
            let _ = updates.send(Update::SessionAction {
                conversation,
                action: kind,
                result,
            });
        });
        true
    }

    pub fn handle_plan_action(&mut self, action: PlanAction) {
        match action {
            PlanAction::Close => self.plan_view = None,
            PlanAction::Accept => self.accept_plan(),
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

    fn open_plan_rejection(&mut self) {
        if let Some(plan) = self.view.plan.clone().filter(|_| self.view.plan_proposed()) {
            self.open_comment_editor(format!("reject:{plan}"));
        } else {
            self.notice = Some("✗ there is no plan to reject".into());
        }
    }

    fn open_plan_comment(&mut self, line: Option<usize>) {
        let Some(plan) = self
            .view
            .plan
            .clone()
            .or_else(|| self.plan_view.as_ref().map(|p| p.plan_id.clone()))
        else {
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
        if let Some(plan) = self
            .view
            .plan
            .clone()
            .or_else(|| self.plan_view.as_ref().map(|p| p.plan_id.clone()))
        {
            self.request(
                Request::Conversation(crate::lean::daemon::ConversationRequest::ReviewComments {
                    conversation: self.conversation.clone(),
                    operation_id: Some(crate::id()),
                    plan,
                }),
                Some("comments sent to the agent for review"),
            );
        }
    }

    /// The header as drawn, which is also what a click on it is read from.
    pub fn header_line(&self, width: u16) -> Line<'static> {
        self.header_line_themed(width, &Theme::from_env())
    }

    pub(in crate::lean::tui) fn header_line_themed(
        &self,
        width: u16,
        theme: &Theme,
    ) -> Line<'static> {
        let panel_focus = match self.navigation.background_focus {
            Focus::Chat => PanelFocus::Chat,
            Focus::Graph => PanelFocus::Graph,
        };
        let label = self.header_project_label(theme);
        let mut line = view::render_header_themed(
            &label,
            &self.conversation,
            &theme.chrome(self.header_status().trim_end_matches(" · ")),
            panel_focus,
            width,
            self.view.controls(),
            theme,
        );
        self.prepend_header_notice(&mut line, theme);
        line
    }

    fn header_project_label(&self, theme: &Theme) -> String {
        let project = workspace::others_summary(&self.projects, self.active_project).map_or_else(
            || self.project.clone(),
            |others| format!("{} ({others})", self.project),
        );
        let branch = self.view.session_branch.as_ref();
        branch.map_or(project.clone(), |branch| {
            format!("{project} {} branch: {branch}", theme.glyphs().dot)
        })
    }

    fn prepend_header_notice(&self, line: &mut Line<'static>, theme: &Theme) {
        if let Some(notice) = &self.notice {
            line.spans.insert(
                0,
                Span::styled(
                    format!(" {} {} ", sanitize(notice), theme.glyphs().dot),
                    theme.title(),
                ),
            );
        }
    }

    /// The header's status text: the stage, then what is going on.
    pub(in crate::lean::tui) fn header_status(&self) -> String {
        let waiting = self.view.approvals.len();
        let status = [
            self.observer.then(|| "observer".to_owned()),
            self.view.paused.then(|| "paused".to_owned()),
            self.view.autonomous.then(|| "approvals: auto".to_owned()),
            (waiting > 0).then(|| format!("{waiting} approval(s) waiting")),
            view::Usage::total(self.view.agents.values())
                .short()
                .map(|usage| format!("tokens {usage}")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let status = status.as_str();
        let stage = self.view.stage();
        let strip = stage.strip(Theme::from_env().glyphs());
        match stage {
            view::Stage::Executing => format!("{strip} · {status} [Task actions]"),
            // The way out is one click away whenever something is stuck.
            view::Stage::Blocked => format!("{strip} · {status} [Unblock]"),
            _ => format!("{strip} · {status}"),
        }
    }

    pub fn key(&mut self, key: event::KeyEvent) -> bool {
        let before = self.navigation.clone();
        let overlay_before = (
            self.help,
            self.picker.is_some(),
            self.plan_view.is_some(),
            self.plan_history.is_some(),
            self.decision_inbox.is_some(),
            self.editor.is_some(),
        );
        let keep_running = self.key_inner(key);
        if self.navigation != before
            || overlay_before
                != (
                    self.help,
                    self.picker.is_some(),
                    self.plan_view.is_some(),
                    self.plan_history.is_some(),
                    self.decision_inbox.is_some(),
                    self.editor.is_some(),
                )
        {
            self.navigation_revision = self.navigation_revision.wrapping_add(1);
        }
        self.text_selection
            .invalidate_context(&self.selection_context());
        if self.selection_blocked() {
            self.text_selection = Default::default();
        }
        keep_running
    }
}
