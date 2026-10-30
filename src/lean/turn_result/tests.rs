#[cfg(test)]
mod tests {
    use super::*;
    use crate::lean::tasks::TaskContract;
    use serde_json::json;

    fn task() -> TaskContract {
        serde_json::from_value(
            json!({"id":"local","title":"API","goal":"Define API","kind":"implementation"}),
        )
        .unwrap()
    }

    #[test]
    fn a_verifier_is_shown_the_recorded_selection() {
        let selection = Selection {
            alternatives: vec![],
            selected: "A".into(),
            rationale: "cheapest".into(),
        };
        let shown = report_with_deliverables("prose", Some(&selection));
        assert!(shown.starts_with("prose"));
        assert!(shown.contains("\"selected\": \"A\"") && shown.contains("cheapest"));
        assert_eq!(report_with_deliverables("prose", None), "prose");
    }

    #[test]
    fn completion_requires_a_summary_and_the_worker_role() {
        let result: TurnResult =
            serde_json::from_value(json!({"type":"completed","summary":"API","report":"Defined"}))
                .unwrap();
        result.validate_task(&task(), false).unwrap();
        assert!(result.validate_task(&task(), true).is_err());
        let result: TurnResult =
            serde_json::from_value(json!({"type":"completed","summary":" ","report":"Defined"}))
                .unwrap();
        assert!(result.validate_task(&task(), false).is_err());
        assert!(serde_json::from_value::<TurnResult>(json!({"text":"done"})).is_err());
    }

    #[test]
    fn a_stored_completion_with_legacy_outputs_still_loads() {
        let result: TurnResult = serde_json::from_value(
            json!({"type":"completed","summary":"API","report":"Defined","outputs":{"api":"x"}}),
        )
        .unwrap();
        assert_eq!(result.text(), "Defined");
    }

    #[test]
    fn review_must_account_for_each_selected_local_risk() {
        let result: TurnResult = serde_json::from_value(
            json!({"type":"review","report":"Reviewed","verdict":{"approved":true,"findings":[]}}),
        )
        .unwrap();
        assert!(result.validate_task(&task(), true).is_err());
    }

    #[test]
    fn a_selection_names_one_of_its_distinct_alternatives() {
        let args = json!({"selected": "A", "rationale": "Preserves contracts", "alternatives": [
            {"id": "A", "approach": "Reuse", "tradeoffs": "Less flexibility"},
            {"id": "B", "approach": "Replace", "tradeoffs": "Changes contracts"}]});
        let TurnResult::Completed {
            summary, selection, ..
        } = task_tool_result("select", &args).unwrap()
        else {
            panic!("a selection completes the task");
        };
        assert_eq!(summary, "Selected A: Reuse");
        assert!(selection.is_some());
        let mut unknown = args.clone();
        unknown["selected"] = json!("C");
        assert!(task_tool_result("select", &unknown).is_err());
        let mut duplicate = args.clone();
        duplicate["alternatives"][1]["id"] = json!("A");
        assert!(task_tool_result("select", &duplicate).is_err());
        let mut single = args;
        single["alternatives"].as_array_mut().unwrap().pop();
        assert!(task_tool_result("select", &single).is_err());
    }

    #[test]
    fn a_stored_selection_of_the_longer_form_still_loads() {
        let stored = json!({"criteria": ["c"], "alternatives": [
            {"id": "A", "approach": "a", "evaluations": ["e"], "eligible": true, "tradeoffs": "t"}],
            "selected": "A", "rationale": "r", "reconsider_when": "w"});
        let selection: Selection = serde_json::from_value(stored).unwrap();
        assert_eq!(selection.chosen().unwrap().approach, "a");
    }

    #[test]
    fn the_runtime_derives_the_verdict_and_stable_finding_ids() {
        let ok = json!({"status": "ok", "note": "checked"});
        let args = json!({"report": "Reviewed", "checks": {"goal": ok}, "findings": []});
        let TurnResult::Review { verdict, .. } = task_tool_result("review", &args).unwrap() else {
            panic!("a review");
        };
        assert!(verdict.approved);
        assert!(verdict.resolved.is_empty());
        let mut fixed = args.clone();
        fixed["resolved"] = json!(["goal:1234abcd"]);
        let TurnResult::Review { verdict, .. } = task_tool_result("review", &fixed).unwrap() else {
            panic!("a review");
        };
        assert_eq!(verdict.resolved, ["goal:1234abcd"]);
        let finding =
            json!({"check": "goal", "severity": "medium", "issue": "wrong", "where": "src/a.rs:1"});
        let mut blocked = args.clone();
        blocked["findings"] = json!([finding.clone(), finding]);
        let TurnResult::Review { verdict, .. } = task_tool_result("review", &blocked).unwrap()
        else {
            panic!("a review");
        };
        assert!(!verdict.approved);
        let ids: Vec<_> = verdict.findings.iter().map(|f| f["id"].clone()).collect();
        assert!(ids[0].as_str().unwrap().starts_with("goal:"));
        assert_ne!(ids[0], ids[1]);
        let again = task_tool_result("review", &blocked).unwrap();
        let TurnResult::Review { verdict: same, .. } = again else {
            panic!("a review")
        };
        assert_eq!(
            same.findings[0]["id"], ids[0],
            "the same finding keeps its id"
        );
        let mut open = args;
        open["checks"]["goal"] = json!({"status": "blocked", "note": "no data"});
        let TurnResult::Review { verdict, .. } = task_tool_result("review", &open).unwrap() else {
            panic!("a review");
        };
        assert!(!verdict.approved, "an unsettled check does not approve");
    }

    #[test]
    fn the_runtime_names_a_question_by_its_content() {
        let args = json!({"context": "Scope decides the work.", "question": "Which scope?",
            "options": [{"label": "Small", "description": "Sooner"}, {"label": "Full"}]});
        let TurnResult::Question { question } = tool_result("ask_user", &args).unwrap() else {
            panic!("a question");
        };
        let TurnResult::Question { question: again } = tool_result("ask_user", &args).unwrap()
        else {
            panic!("a question");
        };
        assert_eq!(question.id, again.id, "the same question keeps its id");
        let mut other = args.clone();
        other["question"] = json!("Which other scope?");
        let TurnResult::Question { question: other } = tool_result("ask_user", &other).unwrap()
        else {
            panic!("a question");
        };
        assert_ne!(question.id, other.id);
        let mut single = args;
        single["options"].as_array_mut().unwrap().pop();
        assert!(tool_result("ask_user", &single).is_err());
    }

    #[test]
    fn a_plain_review_is_approved_unless_a_finding_blocks() {
        let low = json!({"severity": "low", "issue": "naming", "where": "t2"});
        let args = json!({"report": "Checked the plan", "findings": [low]});
        let TurnResult::Review { verdict, .. } = tool_result("review", &args).unwrap() else {
            panic!("a review");
        };
        assert!(verdict.approved && verdict.coverage.is_empty());
        assert!(
            verdict.findings[0]["id"]
                .as_str()
                .unwrap()
                .starts_with("finding:")
        );
        let mut blocked = args;
        blocked["findings"][0]["severity"] = json!("high");
        let encoded = tool_result("review", &blocked).unwrap().encode().unwrap();
        assert!(!TurnResult::review_verdict(&encoded).unwrap().approved);
        assert!(TurnResult::review_verdict("Plan review: approved.").is_none());
    }

    #[test]
    fn an_answer_carries_the_comments_it_settled_and_a_plain_one_stays_text() {
        let plain = tool_result("finish_turn", &json!({"text": "Done"})).unwrap();
        assert_eq!(plain.encode().unwrap(), "Done");
        let settled = tool_result(
            "finish_turn",
            &json!({"text": "Revised", "addressed": [{"id": 3, "how": "split t1"}]}),
        )
        .unwrap();
        let decoded = TurnResult::decode(&settled.encode().unwrap());
        assert_eq!(decoded.addressed(), Some(&[3][..]));
        assert_eq!(decoded.text(), "Revised");
        assert!(tool_result("finish_turn", &json!({"text": " "})).is_err());
    }
}
