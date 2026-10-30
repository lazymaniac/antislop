use super::*;

pub(super) fn review(details: &mut Details, view: &View, records: &mut Vec<Rc<DetailRecord>>) {
    match view.reviews.get(&details.key) {
        Some(lines) if !lines.is_empty() => {
            let lines = lines.iter().cloned().map(plain).collect();
            records.push(record(lines, " "));
        }
        _ => records.push(record(
            vec![(
                dim(),
                "No review yet. The latest verdict appears here once this agent has reviewed something."
                    .into(),
            )],
            " ",
        )),
    }
}
