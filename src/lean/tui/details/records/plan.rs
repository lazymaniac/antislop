use super::*;

pub(super) fn plan(_details: &mut Details, view: &View, records: &mut Vec<Rc<DetailRecord>>) {
    let plain = |text: String| (Style::default(), text);
    let label = |text: &str| {
        (
            Style::default().add_modifier(Modifier::BOLD),
            text.to_owned(),
        )
    };
    let document = view.plan_preview.as_deref().unwrap_or(&view.plan_doc);
    let lines = if document.trim().is_empty() {
        vec![(
            dim(),
            "No plan submitted yet. The preview appears when the planner submits a version.".into(),
        )]
    } else {
        let mut lines = vec![label(if view.plan_preview.is_some() {
            "Latest submitted plan"
        } else {
            "Current plan"
        })];
        lines.push(plain(String::new()));
        lines.extend(
            super::super::super::view::clean(document)
                .lines()
                .map(|line| plain(line.to_owned())),
        );
        lines
    };
    records.push(record(lines, "\n"));
}
