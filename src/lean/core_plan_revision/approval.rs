//! A revision that needs paths outside the accepted plan's scope is put to
//! the user as a decision. Approving it adds the paths to the plan's
//! authorization and resubmits the revision; nothing else widens a scope.
use super::scope::{self, Expansion};
use super::*;
use crate::lean::interview::{Choice, Question};
use rusqlite::OptionalExtension;

const REQUESTED: &str = "scope_approval_requested";
const SETTLED: &str = "scope_approval_settled";
const APPROVE: &str = "Approve";

impl Core {
    /// Asks the user about `expansion` and returns the refusal for the
    /// orchestrator, which says the runtime resubmits on approval.
    pub(super) fn ask_scope_approval(
        &self,
        conversation: &str,
        revision: &PlanRevision,
        expansion: &Expansion,
        write_allowed: &[String],
    ) -> Result<Error> {
        let id = format!("scope-{}", &crate::id()[..8]);
        let mut paths = Vec::new();
        if !expansion.write.is_empty() {
            paths.push(format!("Write: {}", expansion.write.join(", ")));
        }
        if !expansion.read.is_empty() {
            paths.push(format!("Read: {}", expansion.read.join(", ")));
        }
        let question = Question {
            id: id.clone(),
            context: tasks::bound(
                &format!(
                    "The plan revision \"{}\" needs paths the accepted plan does not cover.\n{}",
                    revision.reason,
                    paths.join("\n")
                ),
                4000,
            ),
            question: "Add these paths to the plan's approved scope?".into(),
            options: vec![
                Choice {
                    label: APPROVE.into(),
                    description:
                        "The paths join the approved scope and the revision goes to its review."
                            .into(),
                },
                Choice {
                    label: "Reject".into(),
                    description: "The revision is not applied; work keeps to the approved paths."
                        .into(),
                },
            ],
        };
        self.store.ask_user_decision(conversation, &question)?;
        self.store.append_event(
            conversation,
            None,
            None,
            REQUESTED,
            &json!({"decision": id, "plan": revision.plan, "revision": revision,
                    "read": expansion.read, "write": expansion.write}),
        )?;
        Ok(Error::Invalid(format!(
            "{}. The runtime asked the user (decision {id}); when they approve, it adds the paths and resubmits this revision itself, so do not ask them again or resubmit it",
            scope::expansion_text(expansion, write_allowed)
        )))
    }

    /// Settles a scope decision the user's message answers, if it is one.
    pub(in crate::lean) fn settle_scope_approval(
        self: &Arc<Self>,
        conversation: &str,
        text: &str,
    ) -> Result<()> {
        let Some((decision, answer)) = scope_answer(text) else {
            return Ok(());
        };
        let Some(request) = self.pending_scope_approval(conversation, &decision)? else {
            return Ok(());
        };
        self.store.append_event(
            conversation,
            None,
            None,
            SETTLED,
            &json!({"decision": decision, "approved": answer == APPROVE}),
        )?;
        let revision: PlanRevision = serde_json::from_value(request["revision"].clone())?;
        let text = if answer == APPROVE {
            self.approve_scope(conversation, &decision, &request, revision)
        } else {
            format!(
                "The user answered scope decision {decision} with \"{answer}\" instead of {APPROVE}; plan revision \"{}\" was not applied. Act on their answer.",
                revision.reason
            )
        };
        self.store.add_message(conversation, RUNTIME, &text)?;
        Ok(())
    }

    fn approve_scope(
        self: &Arc<Self>,
        conversation: &str,
        decision: &str,
        request: &serde_json::Value,
        revision: PlanRevision,
    ) -> String {
        let outcome = self
            .extend_plan_authorization(conversation, &revision.plan, request)
            .and_then(|()| self.submit_revision(conversation, decision, revision));
        match outcome {
            Ok(receipt) => format!(
                "The user approved the paths of scope decision {decision}; they joined the plan's approved scope and the revision was resubmitted (request {}, {}).",
                receipt["request"].as_str().unwrap_or_default(),
                receipt["state"].as_str().unwrap_or_default()
            ),
            Err(error) => format!(
                "The user approved the paths of scope decision {decision}, but the revision was not resubmitted: {error}. Submit it again with revise_plan."
            ),
        }
    }

    fn pending_scope_approval(
        &self,
        conversation: &str,
        decision: &str,
    ) -> Result<Option<serde_json::Value>> {
        let payload: Option<String> = self
            .store
            .db()
            .query_row(
                "SELECT payload FROM events WHERE conversation_id=?1 AND kind=?2
                   AND json_extract(payload,'$.decision')=?3
                   AND NOT EXISTS(SELECT 1 FROM events WHERE conversation_id=?1 AND kind=?4
                                  AND json_extract(payload,'$.decision')=?3)
                 ORDER BY seq DESC LIMIT 1",
                rusqlite::params![conversation, REQUESTED, decision, SETTLED],
                |row| row.get(0),
            )
            .optional()?;
        payload
            .map(|payload| serde_json::from_str(&payload).map_err(Error::from))
            .transpose()
    }

    fn extend_plan_authorization(
        &self,
        conversation: &str,
        plan: &str,
        request: &serde_json::Value,
    ) -> Result<()> {
        let mut authorization = scope::plan_authorization(self, conversation, plan)?;
        let paths = |key: &str| -> Vec<String> {
            serde_json::from_value(request[key].clone()).unwrap_or_default()
        };
        extend(&mut authorization.read, paths("read"));
        extend(&mut authorization.write, paths("write"));
        self.store.db().execute(
            "UPDATE plan_authorizations SET scope=?3,write_scope=?4 WHERE plan_id=?1 AND conversation_id=?2",
            rusqlite::params![
                plan,
                conversation,
                serde_json::to_string(&authorization.read)?,
                serde_json::to_string(&authorization.write)?
            ],
        )?;
        Ok(())
    }
}

fn extend(approved: &mut Vec<String>, paths: Vec<String>) {
    for path in paths {
        if !approved.contains(&path) {
            approved.push(path);
        }
    }
}

/// The decision id and answer of an interview answer to a scope decision.
fn scope_answer(text: &str) -> Option<(String, String)> {
    let rest = text.strip_prefix("Interview answer (scope-")?;
    let (suffix, body) = rest.split_once(")\n")?;
    let answer = body
        .lines()
        .find_map(|line| line.strip_prefix("Answer: "))
        .unwrap_or(body)
        .trim();
    Some((format!("scope-{suffix}"), answer.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_answer_to_a_scope_decision_settles_one() {
        assert_eq!(
            scope_answer("Interview answer (scope-1a2b)\nQuestion: Add?\nAnswer: Approve"),
            Some(("scope-1a2b".into(), "Approve".into()))
        );
        assert_eq!(
            scope_answer("Interview answer (scope-1a2b)\nonly the src part"),
            Some(("scope-1a2b".into(), "only the src part".into()))
        );
        assert_eq!(
            scope_answer("Interview answer (remediate)\nAnswer: Approve"),
            None
        );
        assert_eq!(scope_answer("Approve"), None);
    }
}
