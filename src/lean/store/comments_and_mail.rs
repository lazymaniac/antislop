use super::*;
#[path = "comments_and_mail/comments.rs"]
mod comments;
#[path = "comments_and_mail/mail.rs"]
mod mail;
use comments::*;
use mail::*;

impl Store {
    /// Approved writers for this project, most recently approved first.
    /// Task events give completion order, including writers from older plans
    /// and conversations; creation order need not match execution order.
    pub fn approved_writer_results(&self, project: &str) -> Result<Vec<(String, String)>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT t.base,t.result FROM events e
             JOIN tasks t ON t.id=json_extract(e.payload,'$.task')
             JOIN conversations c ON c.id=t.conversation_id
             WHERE c.project=?1 AND e.conversation_id=t.conversation_id
               AND e.kind='task' AND json_extract(e.payload,'$.status')='approved'
               AND t.status='approved' AND t.base IS NOT NULL AND t.result IS NOT NULL
             GROUP BY t.id ORDER BY max(e.seq) DESC",
        )?;
        let rows = statement.query_map([project], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Records a message for `role` and announces it.
    pub fn add_mail(&self, conversation: &str, role: &str, mail: &Mail) -> Result<Mail> {
        self.add_mail_from(conversation, role, mail, None)
    }

    pub fn add_mail_from(
        &self,
        conversation: &str,
        role: &str,
        mail: &Mail,
        source_turn: Option<&str>,
    ) -> Result<Mail> {
        let stored_text = self.content.encode(&mail.text)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let mail = insert_mail_row(&tx, conversation, role, mail, source_turn, &stored_text)?;
        append_mail_event(&tx, conversation, &mail, &stored_text)?;
        tx.commit()?;
        Ok(mail)
    }

    /// Stores a comment with the message that tells the orchestrator about
    /// it (`text` gets the comment's id), in one transaction.
    pub fn add_comment(
        &self,
        conversation: &str,
        comment: &Comment,
        text: impl Fn(u64) -> String,
    ) -> Result<Comment> {
        self.add_comment_at(conversation, comment, None, None, text)
    }

    pub fn add_comment_at(
        &self,
        conversation: &str,
        comment: &Comment,
        round: Option<u32>,
        run: Option<&str>,
        text: impl Fn(u64) -> String,
    ) -> Result<Comment> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let saved = persist_comment(self, &tx, conversation, comment, round, run, text)?;
        tx.commit()?;
        Ok(saved)
    }

    /// Stores a comment on the plan as a draft: the user is still writing,
    /// and the orchestrator hears of it only when the user sends the review.
    pub fn add_draft_comment(&self, conversation: &str, comment: &Comment) -> Result<Comment> {
        let stored_text = self.content.encode(&comment.text)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let comment = insert_draft_comment(self, &tx, conversation, comment, &stored_text)?;
        tx.commit()?;
        Ok(comment)
    }

    /// Hands every draft comment on `plan_id` to the orchestrator as one
    /// message, so that they are answered together in one turn.
    pub fn send_draft_comments(&self, conversation: &str, plan_id: &str) -> Result<Vec<Comment>> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let drafts = draft_comments(self, &tx, conversation, plan_id)?;
        let text = self.content.encode(&draft_message(&drafts))?;
        let now = now_ms();
        let message = insert_draft_message(&tx, conversation, plan_id, &text, now)?;
        let sent = open_draft_comments(self, &tx, conversation, drafts, message, now)?;
        tx.commit()?;
        Ok(sent)
    }

    pub fn set_comment_status(&self, conversation: &str, id: u64, status: &str) -> Result<Comment> {
        let mut db = self.db();
        let tx = db.transaction()?;
        update_comment_status(&tx, conversation, id, status)?;
        let mut comment = load_comment(self, &tx, id)?;
        ensure_comment_was_sent(&comment, status)?;
        if status == "open" {
            reopen_comment(self, &tx, conversation, id, &mut comment)?;
        }
        append_comment_event(self, &tx, conversation, &comment)?;
        tx.commit()?;
        Ok(comment)
    }
}
