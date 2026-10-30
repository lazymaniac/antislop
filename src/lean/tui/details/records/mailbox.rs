use super::*;

pub(super) fn mailbox(details: &mut Details, view: &View, records: &mut Vec<Rc<DetailRecord>>) {
    let plain = |text: String| (Style::default(), text);
    let mailbox_items = view.mailbox.get(&details.key);
    match mailbox_items {
        Some(mail) if !mail.is_empty() => {
            for item in mail {
                records.push(Rc::new(DetailRecord {
                    lines: vec![plain(item.clone())],
                    search_text: item.clone(),
                }));
            }
        }
        _ => {
            let id = details.task_id().unwrap_or(&details.key);
            records.push(Rc::new(DetailRecord {
                lines: vec![(
                    dim(),
                    format!("No messages. [no recorded messages in history for this agent]. Use '@{id} message' in chat to write to this agent."),
                )],
                search_text: "no messages".into(),
            }));
        }
    }
}
