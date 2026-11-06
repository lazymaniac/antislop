//! Diagnostics enumerate only references already visible to the current lease.
use super::*;

pub(super) fn reference_error(
    field: &str,
    requested: &str,
    reason: &str,
    available: &[(String, String)],
) -> Error {
    let format = if uuid::Uuid::parse_str(requested).is_err() {
        "invalid UUID; "
    } else {
        ""
    };
    // Models mis-copy ids by dropping, adding or swapping characters, so compare by
    // edit distance. Only a single closest candidate is suggested, never substituted.
    let mut near = available
        .iter()
        .map(|(id, note)| (edit_distance(id, requested), id, note))
        .filter(|(distance, _, _)| *distance > 0 && *distance <= 4)
        .collect::<Vec<_>>();
    near.sort_by_key(|(distance, _, _)| *distance);
    let by_distance = match near.as_slice() {
        [(_, id, note)] => Some((*id, *note)),
        [(first, id, note), (second, _, _), ..] if first < second => Some((*id, *note)),
        _ => None,
    };
    // Models also splice segments of different ids; a first segment shared with
    // exactly one candidate is still a strong signal.
    let by_prefix = || {
        let head = requested.split('-').next().filter(|head| head.len() >= 8)?;
        let mut shared = available
            .iter()
            .filter(|(id, _)| id.split('-').next() == Some(head));
        match (shared.next(), shared.next()) {
            (Some((id, note)), None) => Some((id, note)),
            _ => None,
        }
    };
    let suggestion = by_distance.or_else(by_prefix);
    let hint = suggestion
        .map(|(id, note)| {
            format!(
                "did you mean {id} ({note})? possible typo: copy it exactly; nothing was substituted. "
            )
        })
        .unwrap_or_default();
    let choices = available
        .iter()
        .take(12)
        .map(|(id, note)| format!("{id} ({note})"))
        .collect::<Vec<_>>()
        .join(", ");
    let supplied = super::super::agent::clip(requested, 80);
    Error::Invalid(format!(
        "{field}: {hint}{format}{reason}; requested {supplied:?}; available in this scope: [{choices}]"
    ))
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut row: Vec<usize> = (0..=right.len()).collect();
    for (i, a) in left.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, b) in right.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (diagonal + usize::from(a != *b))
                .min(above + 1)
                .min(row[j] + 1);
            diagonal = above;
        }
    }
    row[right.len()]
}

impl Core {
    pub(super) fn report_reference_error(
        &self,
        conversation: &str,
        requested: &str,
        history: &store::dependency_results::DependencyReports,
    ) -> Error {
        let available = history
            .reports
            .iter()
            .map(|r| {
                (
                    r.result_ref.clone(),
                    format!(
                        "round {}, generation {}, {}",
                        r.round,
                        r.input_generation,
                        if r.current_inputs {
                            "current inputs"
                        } else {
                            "historical"
                        }
                    ),
                )
            })
            .collect::<Vec<_>>();
        let detail:Option<(String,String,i64,String)>=self.store.db().query_row("SELECT t.conversation_id,s.task_id,x.generation,t.status FROM turns t JOIN turn_scopes s ON s.turn_id=t.id JOIN turn_inputs x ON x.turn_id=t.id WHERE t.id=?1",[requested],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().ok().flatten();
        let reason = match detail {
            Some((owner, task, _, _))
                if owner == conversation
                    && task != history.producer.id
                    && !history.reports.iter().any(|r| r.producer_task == task) =>
            {
                "wrong dependency; this reference is not a result of the requested dependency. Task ids, dependency names and ids from other dependencies are not valid here; leave result_ref out to read the current report, or copy one listed below"
            }
            Some((owner, _, _, status)) if owner == conversation && status != "done" => {
                "report turn has not completed"
            }
            Some((owner, _, generation, _))
                if owner == conversation && generation > history.producer.input_generation =>
            {
                "reference is outside the bound input generation"
            }
            Some((owner, _, _, _)) if owner == conversation => {
                "reference is outside this dependency's bound report history"
            }
            _ => {
                "unknown or unavailable report reference in this conversation; result_ref is optional and only ever a value from report_history (not a task id or evidence id): leave it out to read the current report"
            }
        };
        reference_error("result_ref", requested, reason, &available)
    }

    pub(super) fn evidence_reference_error(
        &self,
        conversation: &str,
        requested: &str,
        reason: &str,
    ) -> Error {
        let db = self.store.db();
        let available = db
            .prepare("SELECT e.id,e.label FROM agent_evidence e JOIN turns t ON t.id=e.turn_id WHERE t.conversation_id=?1 ORDER BY t.started_ms DESC,e.rowid DESC")
            .and_then(|mut q| {
                q.query_map([conversation], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<std::result::Result<Vec<_>, _>>()
            })
            .unwrap_or_default();
        let report_reference = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_results r JOIN turns t ON t.id=r.turn_id WHERE r.turn_id=?1 AND t.conversation_id=?2)",
                rusqlite::params![requested, conversation],
                |r| r.get::<_, bool>(0),
            )
            .unwrap_or(false);
        reference_error(
            "evidence",
            requested,
            if report_reference {
                "this is a report reference; use read_dependency_result.result_ref, not read_evidence"
            } else {
                reason
            },
            &available,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::reference_error;

    fn message(requested: &str) -> String {
        let available = vec![
            (
                "98baa82a-8d26-41a9-8662-2ddebf94947c".to_string(),
                "other".to_string(),
            ),
            (
                "77408913-d154-4f8e-85a5-0357b89bee88".to_string(),
                "recon".to_string(),
            ),
        ];
        reference_error("evidence", requested, "unknown", &available).to_string()
    }

    #[test]
    fn shifted_and_truncated_ids_get_a_leading_suggestion() {
        for typo in [
            "77408913-d154-4f8e-85a5-035b89bee88e",
            "77408913-d154-4f8e-85a5-035b89bee88",
        ] {
            let text = message(typo);
            assert!(
                text.starts_with("evidence: did you mean 77408913-d154-4f8e-85a5-0357b89bee88"),
                "{text}"
            );
            assert!(text.contains("nothing was substituted"));
        }
    }

    #[test]
    fn a_unique_shared_first_segment_gets_a_suggestion() {
        let available = vec![
            (
                "6c4cd4e7-5fac-4621-92fb-32c08c0e1729".to_string(),
                "round 1".to_string(),
            ),
            (
                "98baa82a-8d26-41a9-8662-2ddebf94947c".to_string(),
                "other".to_string(),
            ),
        ];
        let text = reference_error(
            "result_ref",
            "6c4cd4e7-0ebf-4b8b-b33e-db8c4caf549c",
            "unknown",
            &available,
        )
        .to_string();
        assert!(
            text.starts_with("result_ref: did you mean 6c4cd4e7-5fac-4621-92fb-32c08c0e1729"),
            "{text}"
        );
        let ambiguous = vec![
            (
                "6c4cd4e7-5fac-4621-92fb-32c08c0e1729".to_string(),
                "a".to_string(),
            ),
            (
                "6c4cd4e7-1111-4621-92fb-32c08c0e1729".to_string(),
                "b".to_string(),
            ),
        ];
        let text = reference_error(
            "result_ref",
            "6c4cd4e7-0ebf-4b8b-b33e-db8c4caf549c",
            "unknown",
            &ambiguous,
        )
        .to_string();
        assert!(!text.contains("did you mean"), "{text}");
    }

    #[test]
    fn unrelated_ids_get_no_suggestion() {
        assert!(!message("00000000-0000-4000-8000-000000000000").contains("did you mean"));
    }
}
