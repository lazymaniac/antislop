use super::*;

impl Details {
    pub fn neighbour_tab(&self, steps: isize) -> usize {
        let tabs = tabs_for(&self.key);
        let at = tabs.iter().position(|tab| *tab == self.tab).unwrap_or(0) as isize;
        tabs[(at + steps).rem_euclid(tabs.len() as isize) as usize]
    }

    /// The tab a number key picks: the n-th (from 0) of this agent's tabs.
    pub fn nth_tab(&self, n: usize) -> Option<usize> {
        tabs_for(&self.key).get(n).copied()
    }

    pub fn switch_tab(&mut self, next: usize) {
        let next_tab = next % TABS.len();
        if next_tab == self.tab {
            return;
        }
        self.save_current_tab();
        self.tab = next_tab;
        self.restore_current_tab();
        self.searching = false;
    }

    pub fn switch_key(&mut self, new_key: String) {
        self.save_current_tab();
        self.key = new_key;
        self.fetch_in_flight = false;
        self.show_info = false;
        self.selected_round = None;
        self.resume_round = false;
        self.task_uuid = None;
        self.available_rounds.clear();
        self.documents.clear();
        self.resume_round = false;
        self.panel_scroll = [0; 2];
        self.panel_expanded = [false; 2];
        self.active_panel = 0;
        // A different element is a different task: its old record is gone.
        self.task = None;
        self.fetch_in_flight = false;
        self.fetched_round = None;
        self.fetched_generation = None;
        self.fetched_plan = None;
        self.fetch_failures = 0;
        self.fetch_retry_after = None;
        self.fetch_error = None;
        self.restore_current_tab();
        self.searching = false;
    }

    pub(in crate::lean::tui) fn composer_recipient(&self, key: &str) -> String {
        match (key.split_once(':'), &self.task_uuid) {
            (Some((role, _)), Some(uuid)) => format!("{role}:{uuid}"),
            _ => key.to_owned(),
        }
    }

    pub fn composer_key(&self) -> (String, String) {
        let keys = self.session_keys();
        let key = keys.get(self.active_panel).unwrap_or(&self.key);
        (self.conversation.clone(), self.composer_recipient(key))
    }

    pub fn change_round(&mut self, view: &View, direction: isize) {
        let latest = self
            .task_id()
            .and_then(|id| view.tasks.iter().find(|task| task.id == id))
            .map_or_else(
                || self.available_rounds.last().copied().unwrap_or(0),
                |task| task.round,
            );
        let mut rounds = self.available_rounds.clone();
        if !rounds.contains(&latest) {
            rounds.push(latest);
            rounds.sort_unstable();
        }
        let current = self.selected_round.unwrap_or(latest);
        if let Some(index) = rounds.iter().position(|round| *round == current) {
            let next = (index as isize + direction)
                .clamp(0, rounds.len().saturating_sub(1) as isize) as usize;
            let round = rounds[next];
            self.selected_round = (round != latest).then_some(round);
            self.editing = false;
            self.task = None;
            self.documents.clear();
            self.resume_round = false;
            self.scroll = 0;
            self.panel_scroll = [0; 2];
            self.follow = false;
        }
    }

    pub fn save_current_tab(&mut self) {
        self.scroll_memory
            .insert((self.key.clone(), self.tab), self.scroll);
        self.query_memory
            .insert((self.key.clone(), self.tab), self.query.clone());
    }

    pub fn restore_current_tab(&mut self) {
        self.scroll = *self
            .scroll_memory
            .get(&(self.key.clone(), self.tab))
            .unwrap_or(&0);
        self.follow = self.overview() && !self.show_info;
        self.pending_g = false;
        self.query = self
            .query_memory
            .get(&(self.key.clone(), self.tab))
            .cloned()
            .unwrap_or_default();
    }
}
