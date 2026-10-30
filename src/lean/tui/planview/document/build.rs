use super::super::*;

impl PlanView {
    pub fn build_lines(&self, view: &View) -> Vec<DocLine> {
        let raw_doc = raw_document(view);
        DocumentBuilder::new(view).build(&raw_doc)
    }
    pub fn line_comments<'a>(&self, view: &'a View, line_no: usize) -> Vec<&'a Comment> {
        let target_scoped = format!("plan:{}:line:{line_no}", self.plan_id);
        let target_plain = format!("plan:line:{line_no}");
        view.comments
            .values()
            .filter(|c| c.status != "deleted")
            .filter(|c| c.target == target_scoped || c.target == target_plain)
            .collect()
    }
}

fn raw_document(view: &View) -> String {
    if view.plan_doc.trim().is_empty() {
        document::synthesize_document(&view.tasks)
    } else {
        view.plan_doc.clone()
    }
}

struct DocumentBuilder<'a> {
    view: &'a View,
    lines: Vec<DocLine>,
    section: Option<String>,
    level: usize,
    tasks: Vec<String>,
}

impl<'a> DocumentBuilder<'a> {
    fn new(view: &'a View) -> Self {
        Self {
            view,
            lines: Vec::new(),
            section: None,
            level: 0,
            tasks: Vec::new(),
        }
    }

    fn build(mut self, raw: &str) -> Vec<DocLine> {
        for (index, line) in raw.lines().enumerate() {
            self.add_line(index + 1, line);
        }
        if self.lines.is_empty() {
            self.add_empty_document();
        }
        self.lines
    }

    fn add_line(&mut self, number: usize, raw: &str) {
        let is_heading = raw.trim().starts_with('#');
        if is_heading {
            self.update_section(raw.trim());
        }
        self.lines.push(DocLine {
            number,
            text: clean(raw),
            section: self.section.clone(),
            task_id: self.tasks.first().cloned(),
            is_heading,
            state: document::section_state(&self.tasks, &self.view.tasks),
        });
    }

    fn update_section(&mut self, heading: &str) {
        let level = heading.chars().take_while(|ch| *ch == '#').count();
        let text = heading.trim_start_matches('#').trim();
        let matched = document::section_tasks(text, &self.view.tasks);
        if !matched.is_empty() || level <= self.level || self.section.is_none() {
            self.tasks = matched;
            self.section = Some(text.to_owned());
            self.level = level;
        }
    }

    fn add_empty_document(&mut self) {
        self.lines.push(DocLine {
            number: 1,
            text: "# Plan Document (empty)".into(),
            section: None,
            task_id: None,
            is_heading: true,
            state: SectionState::Open,
        });
    }
}
