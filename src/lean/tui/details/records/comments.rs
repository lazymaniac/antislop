use super::super::super::view::Comment;
use super::*;

pub(super) fn comments(details: &mut Details, view: &View, records: &mut Vec<Rc<DetailRecord>>) {
    let comments = details.comments(view);
    if comments.is_empty() {
        records.push(empty_comments());
        return;
    }
    details.comment = details.comment.min(comments.len().saturating_sub(1));
    records.extend(
        comments
            .iter()
            .enumerate()
            .map(|(index, comment)| comment_record(view, index == details.comment, comment)),
    );
}

fn empty_comments() -> Rc<DetailRecord> {
    Rc::new(DetailRecord {
        lines: vec![(dim(), "No comments. [no comments in history for this element]. Press 'c' to add a comment to this element.".into())],
        search_text: "no comments".into(),
    })
}

fn comment_record(view: &View, selected: bool, comment: &Comment) -> Rc<DetailRecord> {
    let style = comment_style(selected, comment);
    let header = comment_header(view, comment);
    record(
        vec![
            (style, header),
            (Style::default(), format!("  {}", comment.text)),
        ],
        " ",
    )
}

fn comment_style(selected: bool, comment: &Comment) -> Style {
    let style = if comment.status == "open" {
        Style::default()
    } else {
        dim()
    };
    if selected {
        style.add_modifier(Modifier::REVERSED)
    } else {
        style
    }
}

fn comment_header(view: &View, comment: &Comment) -> String {
    let state = view.comment_state(comment.id);
    let stale = if view.stale(comment) {
        " · from previous plan"
    } else {
        ""
    };
    let scope = view
        .comment_scopes
        .get(&comment.id)
        .map_or(" · shared", |(round, run)| {
            if round.is_none() && run.is_none() {
                " · shared"
            } else {
                ""
            }
        });
    format!(
        "#{} {} · {state}{stale}{scope}",
        comment.id,
        super::super::super::view::target_name(&comment.target)
    )
}
