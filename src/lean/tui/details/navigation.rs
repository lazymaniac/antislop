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
        if next_tab == self.tab || !tabs_for(&self.key).contains(&next_tab) {
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
        let tabs = tabs_for(&self.key);
        if !tabs.contains(&self.tab) {
            self.tab = tabs[0];
        }
        self.fetch_in_flight = false;
        self.show_info = false;
        self.selected_round = None;
        self.resume_round = false;
        self.task_uuid = None;
        self.available_rounds.clear();
        self.documents.clear();
        self.selected_result_document = None;
        self.result_index.clear();
        self.result_index_membership.clear();
        self.result_metadata.clear();
        self.result_metadata_source = serde_json::Value::Null;
        self.result_selection_generation = self.result_selection_generation.wrapping_add(1);
        self.result_loading = None;
        self.result_error = None;
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

    pub fn switch_role(&mut self) {
        let Some((role @ ("worker" | "reviewer"), id)) = self.key.split_once(':') else {
            return;
        };
        let other = if role == "worker" {
            "reviewer"
        } else {
            "worker"
        };
        let key = format!("{other}:{id}");
        self.save_current_tab();
        self.key = key;
        self.tab = tabs_for(&self.key)[0];
        // Replies for the previous role are ignored; let this role refetch.
        self.fetch_in_flight = false;
        self.fetched_round = None;
        self.fetch_failures = 0;
        self.fetch_retry_after = None;
        self.fetch_error = None;
        self.result_loading = None;
        self.result_error = None;
        self.show_info = false;
        self.active_panel = 0;
        self.panel_scroll = [0; 2];
        self.panel_expanded = [false; 2];
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
            self.selected_result_document = None;
            self.result_index.clear();
            self.result_index_membership.clear();
            self.result_metadata.clear();
            self.result_metadata_source = serde_json::Value::Null;
            self.result_selection_generation = self.result_selection_generation.wrapping_add(1);
            self.result_loading = None;
            self.result_error = None;
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

    pub fn change_result_document(&mut self, direction: isize) {
        let keys = self.visible_result_keys();
        if keys.is_empty() {
            self.selected_result_document = None;
            return;
        }
        let current = self
            .selected_result_document
            .as_ref()
            .and_then(|key| keys.iter().position(|item| item == key))
            .unwrap_or(0);
        let next = (current as isize + direction).rem_euclid(keys.len() as isize) as usize;
        self.selected_result_document = Some(keys[next].clone());
        self.result_selection_generation = self.result_selection_generation.wrapping_add(1);
        self.result_loading = None;
        self.result_error = None;
        self.scroll = 0;
    }

    /// Result documents in the order shown in the Results tab.
    pub(in crate::lean::tui) fn visible_result_keys(&self) -> &[String] {
        &self.result_index
    }

    pub(in crate::lean::tui) fn has_result_key(&self, key: &str) -> bool {
        self.result_index_membership.contains(key)
    }

    /// Rebuild once per metadata update; rendering only borrows this cached order.
    pub(in crate::lean::tui) fn rebuild_result_index(&mut self) {
        let metadata = self
            .task
            .as_ref()
            .and_then(|task| task["_result_documents"].as_array())
            .cloned()
            .unwrap_or_default();
        let mut keys = Vec::new();
        let mut records = std::collections::HashMap::new();
        if self.documents.contains_key("result") {
            keys.push("result".to_owned());
        }
        let mut membership = keys
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let mut artifacts = Vec::new();
        for item in &metadata {
            if let Some(id) = item["id"].as_str() {
                let key = format!("result_document:{id}");
                if membership.insert(key.clone()) {
                    artifacts.push(key.clone());
                }
                records.insert(key, item.clone());
            }
        }
        artifacts.sort();
        keys.extend(artifacts);
        let mut cached = self
            .documents
            .keys()
            .filter(|key| key.starts_with("result_document:") && membership.insert((*key).clone()))
            .cloned()
            .collect::<Vec<_>>();
        cached.sort();
        keys.extend(cached);
        let source = serde_json::Value::Array(metadata);
        self.result_index = keys;
        self.result_index_membership = membership;
        self.result_metadata = records;
        self.result_metadata_source = source;
        if self
            .selected_result_document
            .as_ref()
            .is_some_and(|key| !self.result_index_membership.contains(key))
        {
            self.selected_result_document = None;
            self.result_selection_generation = self.result_selection_generation.wrapping_add(1);
            self.result_loading = None;
            self.result_error = None;
        }
        if self.selected_result_document.is_none() {
            self.selected_result_document = self
                .result_index
                .iter()
                .find(|key| key.starts_with("result_document:"))
                .cloned();
        }
    }
}
