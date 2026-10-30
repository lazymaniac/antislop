use super::super::*;

mod fetch;

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
        details.tab == CHECKS
            && details.fetched_checks
                != details
                    .task_id()
                    .and_then(|id| self.view.task_check_events.get(id))
                    .copied()
                    .unwrap_or(0)
    }

    fn session_is_stale(&self, details: &Details) -> bool {
        matches!(details.tab, 0 | 1 | details::REVIEW | details::VERIFIER)
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
