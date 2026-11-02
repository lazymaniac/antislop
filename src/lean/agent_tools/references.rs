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
        role: &str,
        turn: &str,
        requested: &str,
        reason: &str,
    ) -> Error {
        let mut available = self.store.result_evidence(turn).unwrap_or_default();
        let mut report_ids = std::collections::HashSet::new();
        if let Ok(scope) = self.dependency_reader_scope(conversation, role, turn) {
            for task in scope.tasks {
                if task.status != status::APPROVED
                    && !task_role(role).is_some_and(|(id, _)| id == task.id)
                {
                    continue;
                }
                if let Ok(history) = self.store.dependency_reports(&task) {
                    for report in history.reports {
                        report_ids.insert(report.result_ref.clone());
                        available.extend(
                            self.store
                                .result_evidence(&report.result_ref)
                                .unwrap_or_default(),
                        );
                    }
                }
            }
        }
        available.sort_by(|a, b| a.id.cmp(&b.id));
        available.dedup_by(|a, b| a.id == b.id);
        let available = available
            .into_iter()
            .map(|r| (r.id, r.label))
            .collect::<Vec<_>>();
        reference_error(
            "evidence",
            requested,
            if report_ids.contains(requested) {
                "this is a report reference; use read_dependency_result.result_ref, not read_evidence"
            } else {
                reason
            },
            &available,
        )
    }
}
