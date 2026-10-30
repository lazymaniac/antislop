use super::*;
use crate::lean::ledger::*;
use serde_json::Value;

impl LedgerView {
    pub fn reply(&mut self, kind: &str, reply: std::result::Result<Value, String>) {
        self.pending = false;
        let value = match reply {
            Ok(value) => value,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        self.error = None;
        match kind {
            "page" => self.page_reply(value),
            "detail" => self.detail = Some(value),
            "source" => self.source = value["text"].as_str().map(str::to_owned),
            "preview" => self.preview_reply(value),
            "rejected" => self.rejected_reply(),
            "applied" => self.applied_reply(value),
            "runs" => self.runs_reply(value),
            "created" => self.created_reply(value),
            _ => {}
        }
    }

    fn page_reply(&mut self, value: Value) {
        match serde_json::from_value::<LedgerPage>(value) {
            Ok(page) => {
                if let Some(form) = self.form_mut() {
                    form.revision = page.run.spec_revision;
                    form.operation_id = crate::id();
                }
                self.page = Some(page);
                self.selected = self.selected.min(self.filtered().len().saturating_sub(1));
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn preview_reply(&mut self, value: Value) {
        if self.preview_is_for_other_run(&value) {
            self.error = Some("Proposal response belongs to another run".into());
            return;
        }
        self.save_rejection_draft();
        self.restore_rejection_draft(&value);
        self.rejecting = false;
        self.stale = self.scope_epoch != self.pending_scope_epoch;
        if self.stale {
            self.error = Some("Scope changed during review. Ctrl+R reloads the proposal.".into());
        }
        self.proposed_change = serde_json::from_value(value["change"].clone()).ok();
        self.preview = Some(value["proposal"].clone());
        self.scroll = 0;
    }

    fn preview_is_for_other_run(&self, value: &Value) -> bool {
        self.pending_run
            .as_deref()
            .is_some_and(|run| value["change"]["run_id"].as_str() != Some(run))
    }

    fn restore_rejection_draft(&mut self, value: &Value) {
        let id = value["proposal"]["proposal_id"]
            .as_str()
            .unwrap_or_default();
        self.rejection = self.rejection_drafts.get(id).cloned().unwrap_or_default();
    }

    fn rejected_reply(&mut self) {
        self.visible = false;
        self.preview = None;
        self.proposed_change = None;
        self.rejecting = false;
    }

    fn applied_reply(&mut self, value: Value) {
        self.visible = false;
        self.scroll = 0;
        self.detail = None;
        self.source = None;
        self.notice = Some("Changes saved. Approved analyst proposals resume verification and planning automatically.".into());
        self.preview = None;
        self.update_saved_draft(&value);
        self.proposed_change = None;
    }

    fn update_saved_draft(&mut self, value: &Value) {
        let Some((key, _)) = self.prepared_draft.take() else {
            return;
        };
        let Some(form) = self.drafts.get_mut(&key) else {
            return;
        };
        form.revision = value["spec_revision"].as_i64().unwrap_or(form.revision);
        form.operation_id = crate::id();
        if let Some(saved) = self
            .proposed_change
            .as_ref()
            .and_then(|change| change.entries.first())
        {
            form.base = saved.clone();
        }
    }

    fn runs_reply(&mut self, value: Value) {
        self.preserve_filter();
        self.runs = serde_json::from_value(value).unwrap_or_default();
        if self.runs.is_empty() {
            return;
        }
        let current = self.page.as_ref().map(|page| page.run.id.as_str());
        let index = self.next_run_index(current);
        self.select_run(index);
    }

    fn next_run_index(&self, current: Option<&str>) -> usize {
        self.runs
            .iter()
            .position(|run| Some(run.id.as_str()) == current)
            .map_or(0, |index| (index + 1) % self.runs.len())
    }

    fn select_run(&mut self, index: usize) {
        if let Some(page) = &mut self.page {
            page.run = self.runs[index].clone();
            page.offset = 0;
            page.entries.clear();
        }
        self.restore_filter();
    }

    fn created_reply(&mut self, value: Value) {
        self.preserve_filter();
        if let Ok(run) = serde_json::from_value::<Run>(value) {
            self.install_new_run(run);
        }
        self.restore_filter();
    }

    fn install_new_run(&mut self, run: Run) {
        self.page = Some(LedgerPage {
            run,
            entries: vec![],
            total: 0,
            offset: 0,
            has_more: false,
        });
        self.notice =
            Some("Draft set created. Current planning keeps its existing requirements.".into());
    }
}
