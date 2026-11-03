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
    let suggestion = available.iter().find(|(id, _)| {
        id.len() == requested.len()
            && id
                .bytes()
                .zip(requested.bytes())
                .filter(|(a, b)| a != b)
                .count()
                <= 3
    });
    let hint = suggestion
        .map(|(id, _)| format!("; possible typo: copy {id} exactly; nothing was substituted"))
        .unwrap_or_default();
    let choices = available
        .iter()
        .take(12)
        .map(|(id, note)| format!("{id} ({note})"))
        .collect::<Vec<_>>()
        .join(", ");
    Error::Invalid(format!(
        "{field}: {format}{reason}{hint}; available in this scope: [{choices}]"
    ))
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
                "wrong dependency; this reference is not a result of the requested dependency"
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
            _ => "unknown or unavailable report reference in this conversation",
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
