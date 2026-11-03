use super::dependencies::page_text;
use super::*;

impl Core {
    pub(super) fn evidence_tool(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        tool: &str,
        args: &Value,
    ) -> Result<Value> {
        if tool == "record_evidence" {
            require(
                task_role(role).is_some_and(|(_, reviewer)| !reviewer),
                "only task workers may record execution evidence",
            )?;
            let label = required_text(args, "label")?;
            let text = required_text(args, "text")?;
            return Ok(
                json!({"recorded":true,"evidence":self.store.record_evidence(turn, &label, &text)?}),
            );
        }
        let id = required_text(args, "evidence")?;
        let reference = self.authorized_evidence(conversation, &id)?;
        let text = self.store.evidence_text(&reference)?;
        let binding = crate::hash(serde_json::to_vec(&(conversation, role, turn, &reference))?);
        let (text, cursor) = page_text(&text, &binding, args)?;
        Ok(json!({"evidence":reference,"text":text,"complete":cursor.is_none(),"cursor":cursor}))
    }

    fn authorized_evidence(
        &self,
        conversation: &str,
        id: &str,
    ) -> Result<store::EvidenceReference> {
        self.store.evidence(conversation, id).map_err(|_| {
            self.evidence_reference_error(
                conversation,
                id,
                "unknown or unavailable evidence reference in this conversation",
            )
        })
    }

    /// Every record made this turn is retained automatically. Explicit references
    /// let amendments retain earlier evidence without copying or reconstructing it.
    pub(super) fn completion_evidence(
        &self,
        conversation: &str,
        turn: &str,
        args: &Value,
        result: &mut TurnResult,
    ) -> Result<Vec<store::EvidenceReference>> {
        let requested: Vec<String> = match args.get("evidence").filter(|v| !v.is_null()) {
            Some(value) => serde_json::from_value(value.clone()).map_err(|e| {
                Error::Invalid(format!(
                    "evidence: expected a list of MCP evidence ids: {e}"
                ))
            })?,
            None => vec![],
        };
        let TurnResult::Completed { report, .. } = result else {
            require(
                requested.is_empty(),
                "evidence references are only supported by complete/select",
            )?;
            return Ok(vec![]);
        };
        require(
            requested.len() <= store::MAX_RESULT_EVIDENCE,
            "evidence: at most 64 references",
        )?;
        let mut references = self.store.result_evidence(turn)?;
        for id in requested {
            if !references.iter().any(|r| r.id == id) {
                references.push(self.authorized_evidence(conversation, &id)?);
            }
        }
        require(
            references.len() <= store::MAX_RESULT_EVIDENCE,
            "evidence: at most 64 references",
        )?;
        references.sort_by(|a, b| a.id.cmp(&b.id));
        if !references.is_empty() {
            report.push_str("\n\nMCP execution evidence (read_evidence; immutable records):");
            for reference in &references {
                report.push_str(&format!(
                    "\n- {}: {} ({} bytes, SHA-256 {}; producer turn {})",
                    reference.label,
                    reference.id,
                    reference.bytes,
                    reference.sha256,
                    reference.producer_turn
                ));
            }
        }
        Ok(references)
    }
}
