use super::super::model::TABS;
use super::*;

impl LedgerView {
    pub(super) fn browse_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => self.visible = false,
            KeyCode::Char('/') => self.searching = true,
            KeyCode::Char('f') => self.status_edit = true,
            KeyCode::Char('t') => {
                self.task_filter = InputBuffer::with_text(self.task.clone().unwrap_or_default());
                self.task_filter_edit = true;
            }
            KeyCode::Char('m') => return self.toggle_missing(),
            KeyCode::Down | KeyCode::Up => return self.move_record(key.code == KeyCode::Down),
            KeyCode::Right | KeyCode::Left => return self.change_tab(key.code == KeyCode::Right),
            KeyCode::Char(']' | '[') => return self.change_tab(key.code == KeyCode::Char(']')),
            KeyCode::Char(ch @ '1'..='9') => {
                let tab = (ch as usize) - ('1' as usize);
                if tab < TABS.len() {
                    return self.select_tab(tab);
                }
            }
            KeyCode::Enter => return Action::Detail,
            KeyCode::Char('s') => return Action::Source,
            KeyCode::Char('e') => self.edit(),
            KeyCode::Char('r') => return Action::Fetch,
            KeyCode::Char('g') => return Action::Runs,
            KeyCode::Char('p' | 'P') => return Action::Proposal,
            KeyCode::Char('j') => self.scroll = self.scroll.saturating_add(1),
            KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::PageDown | KeyCode::PageUp => {
                return self.change_page(key.code == KeyCode::PageDown);
            }
            KeyCode::Char('N') => self.confirm_new_run = true,
            KeyCode::Char('n') => return self.new_entry(),
            _ => {}
        }
        Action::None
    }

    fn toggle_missing(&mut self) -> Action {
        self.missing_only = !self.missing_only;
        Action::Fetch
    }

    fn move_record(&mut self, down: bool) -> Action {
        self.invalidate_read();
        let count = self.filtered().len().saturating_sub(1);
        let previous = self.selected;
        self.selected = if down {
            (self.selected + 1).min(count)
        } else {
            self.selected.saturating_sub(1)
        };
        if self.selected == previous && self.detail.is_some() {
            return Action::None;
        }
        self.detail = None;
        self.source = None;
        self.scroll = 0;
        Action::Detail
    }

    fn change_tab(&mut self, right: bool) -> Action {
        let count = TABS.len();
        let next = if right {
            (self.tab + 1) % count
        } else {
            (self.tab + count - 1) % count
        };
        self.select_tab(next)
    }

    fn select_tab(&mut self, tab: usize) -> Action {
        self.invalidate_read();
        self.tab = tab.min(TABS.len() - 1);
        self.source = None;
        self.scroll = 0;
        if self.detail.is_some() {
            Action::None
        } else {
            Action::Detail
        }
    }

    fn change_page(&mut self, down: bool) -> Action {
        let Some(page) = &mut self.page else {
            return Action::None;
        };
        if down && !page.has_more {
            return Action::None;
        }
        page.offset = if down {
            page.offset.saturating_add(100)
        } else {
            page.offset.saturating_sub(100)
        };
        self.selected = 0;
        Action::Fetch
    }

    fn new_entry(&mut self) -> Action {
        let Some(page) = &self.page else {
            return Action::Create;
        };
        let spec: EntrySpec = serde_json::from_value(serde_json::json!({
            "id": format!("req-{}", crate::id()), "source":{"source_id":"new","start":0,"end":1},
            "interpretation":"", "criteria":[], "mandatory":true
        }))
        .unwrap();
        let key = (page.run.id.clone(), spec.id.clone());
        let mut form = Form::new(spec, page.run.spec_revision);
        form.field = FormField::Interpretation;
        self.drafts.insert(key.clone(), form);
        self.editing = Some(key);
        Action::None
    }
}
