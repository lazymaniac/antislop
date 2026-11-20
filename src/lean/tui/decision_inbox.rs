use super::{Theme, interview, view};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    Approval(String),
    Interview(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Focus {
    List,
    #[default]
    Details,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Inbox {
    pub conversation: String,
    pub selected: Option<Target>,
    pub selected_option: usize,
    focus: Focus,
    list_offset: usize,
    detail_scroll: u16,
}

pub struct InboxView<'a> {
    pub items: &'a [(Target, String, String)],
    pub approvals: &'a [view::PendingApproval],
    pub form: Option<&'a mut interview::Form>,
    pub statuses: &'a std::collections::HashMap<String, ApprovalStatus>,
}

impl Inbox {
    pub fn new(conversation: String, items: &[Target]) -> Self {
        Self {
            conversation,
            selected: items.first().cloned(),
            selected_option: 0,
            focus: Focus::Details,
            list_offset: 0,
            detail_scroll: 0,
        }
    }

    pub fn key(&mut self, key: KeyEvent, items: &[Target]) -> InboxAction {
        if key.code == KeyCode::Esc {
            return InboxAction::Close;
        }
        if key.code == KeyCode::Tab || key.code == KeyCode::BackTab {
            self.focus = match self.focus {
                Focus::List => Focus::Details,
                Focus::Details => Focus::List,
            };
            return InboxAction::Handled;
        }
        if items.is_empty() {
            return InboxAction::Handled;
        }
        let current = self
            .selected
            .as_ref()
            .and_then(|selected| items.iter().position(|item| item == selected))
            .unwrap_or(0);
        match (self.focus, key.code) {
            (Focus::List, KeyCode::Up) => self.select(items, current.saturating_sub(1)),
            (Focus::List, KeyCode::Down) => self.select(items, (current + 1).min(items.len() - 1)),
            (Focus::List, KeyCode::Enter | KeyCode::Right) => self.focus = Focus::Details,
            (Focus::Details, KeyCode::Left) => self.focus = Focus::List,
            (Focus::List, KeyCode::PageUp) => self.list_offset = self.list_offset.saturating_sub(5),
            (Focus::List, KeyCode::PageDown) => {
                self.list_offset = self.list_offset.saturating_add(5)
            }
            _ => return InboxAction::Details(key),
        }
        InboxAction::Handled
    }

    pub fn reconcile(&mut self, items: &[Target]) {
        if self
            .selected
            .as_ref()
            .is_some_and(|selected| items.contains(selected))
        {
            return;
        }
        self.selected = items.first().cloned();
        self.selected_option = 0;
        self.list_offset = 0;
    }

    fn select(&mut self, items: &[Target], index: usize) {
        let target = items[index].clone();
        if self.selected.as_ref() != Some(&target) {
            self.selected = Some(target);
            self.selected_option = 0;
            self.detail_scroll = 0;
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, screen: Rect, data: InboxView<'_>, theme: &Theme) {
        let InboxView {
            items,
            approvals,
            form,
            statuses,
        } = data;
        if screen.width == 0 || screen.height == 0 {
            return;
        }
        let width = screen.width.min(88);
        let height = screen.height.min(30);
        let area = Rect::new(
            screen.x + screen.width.saturating_sub(width) / 2,
            screen.y + screen.height.saturating_sub(height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, area);
        let outer = Block::default()
            .borders(Borders::ALL)
            .title(" Decisions ")
            .style(theme.card());
        let body = outer.inner(area);
        frame.render_widget(outer, area);
        if body.width < 2 || body.height < 2 {
            return;
        }
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(31), Constraint::Percentage(69)])
            .split(body);
        let selected_index = self
            .selected
            .as_ref()
            .and_then(|target| items.iter().position(|item| &item.0 == target));
        let list_block = Block::default().borders(Borders::RIGHT).title(" Inbox ");
        let list_inner = list_block.inner(columns[0]);
        frame.render_widget(list_block, columns[0]);
        let list_height = usize::from(list_inner.height);
        if let Some(index) = selected_index {
            if index < self.list_offset {
                self.list_offset = index;
            }
            if index >= self.list_offset.saturating_add(list_height) {
                self.list_offset = index.saturating_add(1).saturating_sub(list_height);
            }
        }
        self.list_offset = self
            .list_offset
            .min(items.len().saturating_sub(list_height));
        let list_lines = items
            .iter()
            .enumerate()
            .skip(self.list_offset)
            .take(list_height)
            .map(|(index, (_, title, status))| {
                let selected = Some(index) == selected_index;
                let line = format!("{} {}", if selected { ">" } else { " " }, title);
                let style = if selected {
                    theme.selected()
                } else {
                    theme.card()
                };
                let status = Span::styled(status.clone(), theme.dim());
                Line::from(vec![
                    Span::styled(theme.fit(&line, usize::from(list_inner.width)), style),
                    status,
                ])
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(list_lines).style(theme.card()), list_inner);
        let Some(target) = self
            .selected
            .as_ref()
            .filter(|target| items.iter().any(|item| &item.0 == *target))
        else {
            frame.render_widget(
                Paragraph::new("No open decisions.").style(theme.dim()),
                columns[1],
            );
            return;
        };
        match target {
            Target::Interview(_) => {
                if let Some(form) = form {
                    form.draw(frame, columns[1], theme);
                }
            }
            Target::Approval(request) => {
                if let Some(approval) = approvals
                    .iter()
                    .find(|approval| &approval.request == request)
                {
                    self.draw_approval(
                        frame,
                        columns[1],
                        approval,
                        statuses.get(request).cloned(),
                        theme,
                    );
                }
            }
        }
    }

    fn draw_approval(
        &self,
        frame: &mut Frame,
        area: Rect,
        approval: &view::PendingApproval,
        status: Option<ApprovalStatus>,
        theme: &Theme,
    ) {
        let inner = Block::default()
            .borders(Borders::ALL)
            .title(" CLI approval ")
            .inner(area);
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .title(" CLI approval ")
                .style(theme.card()),
            area,
        );
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let width = usize::from(inner.width).saturating_sub(1).max(1);
        let mut lines = vec![
            Line::from(Span::styled(
                format!("{} · {}", approval.role, approval.provider),
                theme.title(),
            )),
            Line::raw(String::new()),
        ];
        lines.extend(
            view::wrap(&approval.summary, width)
                .into_iter()
                .map(Line::raw),
        );
        lines.push(Line::raw(String::new()));
        let chosen = self
            .selected_option
            .min(approval.options.len().saturating_sub(1));
        for (index, (id, label, decision)) in approval.options.iter().enumerate() {
            let marker = if index == chosen { ">" } else { " " };
            let line = format!("{marker} {}. {} [{}] · {}", index + 1, label, id, decision);
            let style = if index == chosen {
                theme.selected()
            } else {
                theme.card()
            };
            lines.extend(
                view::wrap(&line, width)
                    .into_iter()
                    .map(|line| Line::from(Span::styled(line, style))),
            );
        }
        lines.push(Line::raw(String::new()));
        lines.push(match status {
            Some(ApprovalStatus::Sending { .. }) => {
                Line::from(Span::styled("Sending…", theme.dim()))
            }
            Some(ApprovalStatus::Accepted { .. }) => {
                Line::from(Span::styled("Sent · waiting for resolution", theme.dim()))
            }
            Some(ApprovalStatus::Failed { error, .. }) => Line::from(Span::styled(
                format!("Error: {error} · retry with Enter"),
                theme.warning(),
            )),
            None if approval.orchestrator => Line::from(Span::styled(
                "Waiting for orchestrator; you may still decide.",
                theme.dim(),
            )),
            None => Line::from(Span::styled(
                "Enter: send · 0: decide later · Tab: inbox",
                theme.dim(),
            )),
        });
        frame.render_widget(
            Paragraph::new(lines)
                .style(theme.card())
                .scroll((self.detail_scroll, 0)),
            inner,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::{Terminal, backend::TestBackend};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn approval(request: &str) -> view::PendingApproval {
        view::PendingApproval {
            request: request.into(),
            role: "worker:t1".into(),
            provider: "Codex".into(),
            summary: "Run the requested validation command in the assigned checkout.".into(),
            orchestrator: false,
            options: vec![
                (
                    "allow-once".into(),
                    "Allow once".into(),
                    "allow_once".into(),
                ),
                ("deny".into(), "Deny".into(), "deny".into()),
            ],
        }
    }

    #[test]
    fn inbox_navigation_uses_stable_request_ids_and_preserves_option_selection() {
        let items = vec![Target::Approval("a".into()), Target::Approval("b".into())];
        let mut inbox = Inbox::new("session-a".into(), &items);
        assert_eq!(inbox.selected, Some(items[0].clone()));
        inbox.key(key(KeyCode::Tab), &items);
        assert_eq!(inbox.key(key(KeyCode::Down), &items), InboxAction::Handled);
        assert_eq!(inbox.selected, Some(items[1].clone()));
        inbox.selected_option = 1;
        inbox.key(key(KeyCode::Up), &items);
        assert_eq!(inbox.selected, Some(items[0].clone()));
        assert_eq!(inbox.selected_option, 0);
        assert_eq!(inbox.key(key(KeyCode::Esc), &items), InboxAction::Close);
    }

    #[test]
    fn f2_opens_modal_over_interview_and_preserves_question_draft() {
        let mut app = crate::lean::tui::App::test_app();
        app.view.workflow_phase = Some("interview".into());
        app.view.interview_question = Some((
            7,
            serde_json::from_value(serde_json::json!({
                "id":"scope", "context":"Context to review.", "question":"Choose scope?",
                "options":[{"label":"Small"},{"label":"Full"}]
            }))
            .unwrap(),
        ));
        app.sync_interview();
        app.input.insert_str("chat draft");
        app.interview_forms
            .get_mut(&app.conversation)
            .unwrap()
            .input
            .insert_str("draft");
        app.interview_forms
            .get_mut(&app.conversation)
            .unwrap()
            .visible = false;
        assert!(app.key_inner(key(KeyCode::F(2))));
        assert!(app.decision_inbox.is_some());
        assert_eq!(app.interview_forms[&app.conversation].input.text, "draft");
        assert!(app.key_inner(key(KeyCode::PageDown)));
        assert!(app.decision_inbox.is_some());
        assert!(app.key_inner(key(KeyCode::Char('x'))));
        assert_eq!(app.interview_forms[&app.conversation].input.text, "draftx");
        assert_eq!(app.input.text, "chat draft");
        assert!(app.key_inner(key(KeyCode::Esc)));
        assert!(app.decision_inbox.is_none());
        assert!(!app.interview_forms[&app.conversation].visible);
    }

    #[test]
    fn approval_details_render_original_provider_text_and_every_choice() {
        let approval = approval("request");
        let items = vec![(
            Target::Approval("request".into()),
            "worker:t1 · Codex".into(),
            "Needs decision".into(),
        )];
        let mut inbox = Inbox::new(
            "session".into(),
            &items.iter().map(|item| item.0.clone()).collect::<Vec<_>>(),
        );
        let statuses = std::collections::HashMap::new();
        let mut terminal = Terminal::new(TestBackend::new(84, 24)).unwrap();
        terminal
            .draw(|frame| {
                inbox.draw(
                    frame,
                    frame.area(),
                    InboxView {
                        items: &items,
                        approvals: &[approval],
                        form: None,
                        statuses: &statuses,
                    },
                    &Theme::new(true, false),
                )
            })
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        for text in [
            "Codex",
            "Run the requested validation",
            "Allow once",
            "Deny",
            "decide later",
        ] {
            assert!(rendered.contains(text), "missing {text}");
        }
    }

    #[test]
    fn approval_option_fields_wrap_without_truncating_their_ends() {
        let mut approval = approval("request");
        approval.options = vec![(
            format!("{}ID-END", "i".repeat(64)),
            format!("{}LABEL-END", "l".repeat(64)),
            format!("{}DECISION-END", "d".repeat(64)),
        )];
        let items = vec![(
            Target::Approval("request".into()),
            "worker:t1 · Codex".into(),
            "Needs decision".into(),
        )];
        let mut inbox = Inbox::new(
            "session".into(),
            &items.iter().map(|item| item.0.clone()).collect::<Vec<_>>(),
        );
        let statuses = std::collections::HashMap::new();
        let mut terminal = Terminal::new(TestBackend::new(84, 30)).unwrap();
        terminal
            .draw(|frame| {
                inbox.draw(
                    frame,
                    frame.area(),
                    InboxView {
                        items: &items,
                        approvals: &[approval],
                        form: None,
                        statuses: &statuses,
                    },
                    &Theme::new(true, false),
                )
            })
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        for suffix in ["ID-END", "LABEL-END", "DECISION-END"] {
            assert!(rendered.contains(suffix), "missing wrapped suffix {suffix}");
        }
    }

    #[test]
    fn approval_result_requires_the_matching_session_request_and_operation() {
        let mut app = crate::lean::tui::App::test_app();
        app.approval_submissions.insert(
            ("session-a".into(), "r1".into()),
            ApprovalStatus::Sending {
                operation_id: "op-new".into(),
            },
        );
        app.apply_approval_result(crate::lean::tui::Update::ApprovalResult {
            conversation: "session-a".into(),
            request: "r1".into(),
            operation_id: "op-old".into(),
            result: Err("late failure".into()),
        });
        assert!(app.approval_submissions[&("session-a".into(), "r1".into())].pending());
        app.apply_approval_result(crate::lean::tui::Update::ApprovalResult {
            conversation: "session-b".into(),
            request: "r1".into(),
            operation_id: "op-new".into(),
            result: Err("other session".into()),
        });
        app.apply_approval_result(crate::lean::tui::Update::ApprovalResult {
            conversation: "session-a".into(),
            request: "r1".into(),
            operation_id: "op-new".into(),
            result: Err("raw daemon error".into()),
        });
        assert_eq!(
            app.approval_submissions[&("session-a".into(), "r1".into())].error(),
            Some("raw daemon error")
        );
        assert!(
            !app.approval_submissions
                .contains_key(&("session-b".into(), "r1".into()))
        );
    }

    #[test]
    fn accepted_decision_waits_without_resubmitting_and_defer_keeps_request_open() {
        let mut app = crate::lean::tui::App::test_app();
        app.view.approvals.push(approval("r1"));
        app.approval_submissions.insert(
            (app.conversation.clone(), "r1".into()),
            ApprovalStatus::Accepted {
                operation_id: "op1".into(),
            },
        );
        app.open_decision_inbox();
        assert!(app.key_inner(key(KeyCode::PageDown)));
        assert_eq!(app.decision_inbox.as_ref().unwrap().detail_scroll, 5);
        assert!(app.key_inner(key(KeyCode::Enter)));
        assert!(matches!(
            app.approval_submissions[&(app.conversation.clone(), "r1".into())],
            ApprovalStatus::Accepted { .. }
        ));
        assert!(app.key_inner(key(KeyCode::Char('0'))));
        assert!(app.decision_inbox.is_none());
        assert!(
            app.view
                .approvals
                .iter()
                .any(|pending| pending.request == "r1")
        );
        app.open_decision_inbox();
        assert_eq!(
            app.decision_inbox.as_ref().unwrap().selected,
            Some(Target::Approval("r1".into()))
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboxAction {
    Handled,
    Close,
    Details(KeyEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalStatus {
    Sending { operation_id: String },
    Accepted { operation_id: String },
    Failed { operation_id: String, error: String },
}

impl ApprovalStatus {
    pub fn operation(&self) -> &str {
        match self {
            Self::Sending { operation_id }
            | Self::Accepted { operation_id }
            | Self::Failed { operation_id, .. } => operation_id,
        }
    }
    pub fn pending(&self) -> bool {
        matches!(self, Self::Sending { .. } | Self::Accepted { .. })
    }
    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Failed { error, .. } => Some(error),
            _ => None,
        }
    }
}

impl super::App {
    pub(super) fn draw_decision_inbox(&mut self, frame: &mut Frame) {
        if self.help.is_some()
            || self.picker.is_some()
            || self.editor.is_some()
            || self.decision_inbox.is_none()
        {
            return;
        }
        let items = self.decision_items();
        let approvals = self.view.approvals.clone();
        let statuses = self
            .approval_submissions
            .iter()
            .filter(|((conversation, _), _)| conversation == &self.conversation)
            .map(|((_, request), status)| (request.clone(), (*status).clone()))
            .collect::<std::collections::HashMap<_, _>>();
        let theme = Theme::from_env();
        let Some(inbox) = self.decision_inbox.as_mut() else {
            return;
        };
        inbox.reconcile(&items.iter().map(|item| item.0.clone()).collect::<Vec<_>>());
        let form = self.interview_forms.get_mut(&self.conversation);
        inbox.draw(
            frame,
            frame.area(),
            InboxView {
                items: &items,
                approvals: &approvals,
                form,
                statuses: &statuses,
            },
            &theme,
        );
    }

    pub(super) fn decision_items(&self) -> Vec<(Target, String, String)> {
        let mut items = self
            .view
            .approvals
            .iter()
            .map(|approval| {
                let status = self
                    .approval_submissions
                    .get(&(self.conversation.clone(), approval.request.clone()));
                let label = if status.is_some_and(ApprovalStatus::pending) {
                    if matches!(status, Some(ApprovalStatus::Accepted { .. })) {
                        "Sent · waiting"
                    } else {
                        "Sending"
                    }
                } else if status.and_then(ApprovalStatus::error).is_some() {
                    "Error"
                } else if approval.orchestrator {
                    "Waiting"
                } else {
                    "Needs decision"
                };
                (
                    Target::Approval(approval.request.clone()),
                    format!("{} · {}", approval.role, approval.provider),
                    label.to_owned(),
                )
            })
            .collect::<Vec<_>>();
        if let Some((event, question)) = self.view.interview_question.as_ref()
            && self
                .interview_forms
                .get(&self.conversation)
                .is_some_and(|form| form.event == *event && !form.sent)
        {
            let form = &self.interview_forms[&self.conversation];
            let status = if form.sending {
                "Sending"
            } else if form.error.is_some() {
                "Error"
            } else {
                "Needs answer"
            };
            items.push((
                Target::Interview(*event),
                question.question.clone(),
                status.to_owned(),
            ));
        }
        items
    }

    pub(super) fn open_decision_inbox(&mut self) {
        self.sync_interview();
        let items = self.decision_items();
        if items.is_empty() {
            self.notice = Some("No decisions are waiting.".into());
            return;
        }
        let conversation = self.conversation.clone();
        self.decision_inbox = Some(super::decision_inbox::Inbox::new(
            conversation,
            &items.iter().map(|item| item.0.clone()).collect::<Vec<_>>(),
        ));
    }

    pub(super) fn route_decision_inbox(&mut self, key: KeyEvent) -> Option<bool> {
        let mut inbox = self.decision_inbox.take()?;
        if inbox.conversation != self.conversation {
            return Some(true);
        }
        let items = self.decision_items();
        let targets = items.iter().map(|item| item.0.clone()).collect::<Vec<_>>();
        inbox.reconcile(&targets);
        match inbox.key(key, &targets) {
            InboxAction::Close => return Some(true),
            InboxAction::Handled => {
                self.decision_inbox = Some(inbox);
                return Some(true);
            }
            InboxAction::Details(key) => {
                if let Some(Target::Interview(event)) = inbox.selected.as_ref() {
                    let answer = self
                        .interview_forms
                        .get_mut(&self.conversation)
                        .filter(|form| form.event == *event)
                        .and_then(|form| form.key(key));
                    if let Some(answer) = answer {
                        self.send_interview_answer(answer);
                    }
                } else if let Some(Target::Approval(request)) = inbox.selected.as_ref()
                    && let Some(approval) = self
                        .view
                        .approvals
                        .iter()
                        .find(|approval| &approval.request == request)
                {
                    if key.code == KeyCode::Char('0') || key.code == KeyCode::Char('d') {
                        self.approval_hidden.insert(request.clone());
                        self.decision_inbox = None;
                        return Some(true);
                    }
                    let status_key = (self.conversation.clone(), request.clone());
                    if key.code == KeyCode::PageUp {
                        inbox.detail_scroll = inbox.detail_scroll.saturating_sub(5);
                    } else if key.code == KeyCode::PageDown {
                        inbox.detail_scroll = inbox.detail_scroll.saturating_add(5);
                    } else if !self
                        .approval_submissions
                        .get(&status_key)
                        .is_some_and(ApprovalStatus::pending)
                    {
                        match key.code {
                            KeyCode::Up => {
                                inbox.selected_option = inbox.selected_option.saturating_sub(1)
                            }
                            KeyCode::Down => {
                                inbox.selected_option = (inbox.selected_option + 1)
                                    .min(approval.options.len().saturating_sub(1))
                            }
                            KeyCode::Enter if !approval.options.is_empty() => {
                                let option = &approval.options
                                    [inbox.selected_option.min(approval.options.len() - 1)];
                                self.send_approval_decision(request.clone(), option.0.clone());
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        self.decision_inbox = Some(inbox);
        Some(true)
    }

    fn send_approval_decision(&mut self, request: String, choice: String) {
        let conversation = self.conversation.clone();
        let operation_id = crate::id();
        self.approval_submissions.insert(
            (conversation.clone(), request.clone()),
            ApprovalStatus::Sending {
                operation_id: operation_id.clone(),
            },
        );
        let client = self.client.clone();
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let result = client
                .call(&crate::lean::daemon::Request::Conversation(
                    crate::lean::daemon::ConversationRequest::AnswerApproval {
                        conversation: conversation.clone(),
                        operation_id: Some(operation_id.clone()),
                        request: request.clone(),
                        choice,
                    },
                ))
                .map(|_| ())
                .map_err(|error| error.to_string());
            let _ = updates.send(crate::lean::tui::Update::ApprovalResult {
                conversation,
                request,
                operation_id,
                result,
            });
        });
    }

    pub(super) fn apply_approval_result(&mut self, update: crate::lean::tui::Update) {
        let crate::lean::tui::Update::ApprovalResult {
            conversation,
            request,
            operation_id,
            result,
        } = update
        else {
            unreachable!()
        };
        let key = (conversation, request);
        let Some(status) = self.approval_submissions.get(&key) else {
            return;
        };
        if status.operation() != operation_id {
            return;
        }
        match result {
            Ok(()) => {
                self.approval_submissions
                    .insert(key, ApprovalStatus::Accepted { operation_id });
            }
            Err(error) => {
                self.approval_submissions.insert(
                    key,
                    ApprovalStatus::Failed {
                        operation_id,
                        error,
                    },
                );
            }
        }
    }
}
