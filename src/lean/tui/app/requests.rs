use super::super::*;

pub(super) mod fetch;
pub(in crate::lean::tui) use fetch::ResultFetchCoordinator;

impl App {
    /// Runs a daemon request off the draw loop; its error becomes a notice.
    pub(in crate::lean::tui) fn request(&self, request: Request, done: Option<&'static str>) {
        let client = self.client.clone();
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let notice = match client.call(&request) {
                Ok(_) => done.map(str::to_owned),
                Err(error) => Some(format!("✗ {error}")),
            };
            if let Some(notice) = notice {
                let _ = updates.send(Update::Notice(notice));
            }
        });
    }

    /// The round of the task row behind a details key, if the row is shown.
    pub(in crate::lean::tui) fn task_row_round(&self, key: &str) -> Option<u64> {
        let id = key.split_once(':')?.1;
        self.view
            .tasks
            .iter()
            .find(|task| task.id == id)
            .map(|task| task.round)
    }

    /// Refetches an open details panel once its task has moved on to a new
    /// round, so the stored report it shows never belongs to a past round.
    pub(in crate::lean::tui) fn refetch_stale_details(&mut self) {
        self.schedule_output_document_fetch();
        let stale = self
            .details
            .as_ref()
            .is_some_and(|details| self.details_are_stale(details));
        if stale {
            self.fetch_task();
        }
    }

    fn details_are_stale(&self, details: &Details) -> bool {
        if details.fetch_in_flight
            || details
                .fetch_retry_after
                .is_some_and(|deadline| std::time::Instant::now() < deadline)
        {
            return false;
        }
        let now = details
            .selected_round
            .or_else(|| self.task_row_round(&details.key))
            .or(details.fetched_round)
            .unwrap_or(0);
        details.fetched_round != Some(now)
            || self.checks_are_stale(details)
            || self.session_is_stale(details)
            || self.latest_round(details) != details.latest_observed
    }

    fn checks_are_stale(&self, details: &Details) -> bool {
        details.shows_checks()
            && details.fetched_checks
                != details
                    .task_id()
                    .and_then(|id| self.view.task_check_events.get(id))
                    .copied()
                    .unwrap_or(0)
    }

    fn session_is_stale(&self, details: &Details) -> bool {
        !details.shows_checks()
            && (matches!(
                details.content_tab(),
                Some(
                    WorkbenchTab::Summary
                        | WorkbenchTab::Conversation
                        | WorkbenchTab::Diff
                        | WorkbenchTab::Results
                )
            ) || (details.workbench_tab.is_none() && details.tab == details::REVIEW))
            && details.selected_round.is_none()
            && details.fetched_events != self.view.after
    }

    fn latest_round(&self, details: &Details) -> u64 {
        self.task_row_round(&details.key)
            .or_else(|| {
                self.view.agents.get(&details.key).map(|agent| {
                    if agent.running {
                        u64::from(agent.turns)
                    } else {
                        u64::from(agent.turns.saturating_sub(1))
                    }
                })
            })
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reviewer_summary_refreshes_when_the_final_verdict_arrives_without_another_check() {
        let mut app = App::test_app();
        app.view.tasks.push(view::TaskRow {
            id: "t1".into(),
            round: 1,
            status: "reviewing".into(),
            ..Default::default()
        });
        app.view.after = 10;
        app.view.check_events = 9;
        app.view.task_check_events.insert("t1".into(), 9);
        let mut details = Details::new("reviewer:t1".into());
        details.prepare_workbench_details(None, WorkbenchTab::Summary);
        details.fetched_round = Some(1);
        details.latest_observed = 1;
        details.fetched_checks = 9;
        details.fetched_events = 10;
        assert!(!app.details_are_stale(&details));
        app.view
            .apply(&serde_json::json!({"seq":11,"kind":"task","payload":{
                "id":"t1","status":"approved","round":1
            }}));
        assert_eq!(app.view.task_check_events["t1"], 10);
        assert!(app.details_are_stale(&details));
        details.fetched_events = app.view.after;
        details.fetched_checks = app.view.task_check_events["t1"];
        assert!(!app.details_are_stale(&details));
    }
}
