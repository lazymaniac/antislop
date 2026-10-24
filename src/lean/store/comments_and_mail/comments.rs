use super::*;

#[path = "comments/insert.rs"]
mod insert;
#[path = "comments/updates.rs"]
mod updates;
pub(in crate::lean::store::comments_and_mail) use insert::{
    append_comment_event, insert_draft_comment, persist_comment,
};
pub(in crate::lean::store::comments_and_mail) use updates::{
    draft_comments, draft_message, ensure_comment_was_sent, insert_draft_message, load_comment,
    open_draft_comments, reopen_comment, update_comment_status,
};
