use super::super::*;

impl App {
    pub(in crate::lean::tui) fn sync_interview(&mut self) {
        if let Some((event, question)) = &self.view.interview_question {
            let same = self
                .interview_forms
                .get(&self.conversation)
                .is_some_and(|form| form.event == *event);
            if !same {
                self.interview_forms.insert(
                    self.conversation.clone(),
                    interview::Form::new(*event, question.clone()),
                );
            }
        }
    }

    pub(in crate::lean::tui) fn interview_visible(&self) -> bool {
        (self.view.workflow_phase.as_deref() == Some("interview") || self.view.user_decision)
            && self
                .view
                .interview_question
                .as_ref()
                .is_some_and(|(event, _)| {
                    self.interview_forms
                        .get(&self.conversation)
                        .is_some_and(|form| form.event == *event && form.visible && !form.sent)
                })
    }

    pub(in crate::lean::tui) fn send_interview_answer(&mut self, text: String) {
        let Some(form) = self.interview_forms.get_mut(&self.conversation) else {
            return;
        };
        if form.sending || form.sent {
            return;
        }
        form.sending = true;
        form.error = None;
        let question = form.event;
        let client = self.client.clone();
        let updates = self.updates.clone();
        let conversation = self.conversation.clone();
        std::thread::spawn(move || {
            let result = client
                .call(&Request::Send {
                    conversation: conversation.clone(),
                    text,
                })
                .map(|_| ())
                .map_err(|error| error.to_string());
            let _ = updates.send(Update::InterviewResult {
                conversation,
                question,
                result,
            });
        });
    }
}
