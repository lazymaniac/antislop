use super::*;

#[path = "tests/agent_exchange.rs"]
mod agent_exchange;
#[path = "tests/events_and_recovery.rs"]
mod events_and_recovery;
#[path = "tests/rows_and_feedback.rs"]
mod rows_and_feedback;
#[path = "tests/schema_and_comments.rs"]
mod schema_and_comments;

#[path = "tests/plan_revision.rs"]
mod plan_revision;

impl Store {
    pub(in crate::lean) fn test_completion(&self, task: &Task, report: &str) {
        let conversation = self.plan_conversation(&task.plan_id).unwrap();
        let turn = self
            .start_turn(
                &conversation,
                &format!("worker:{}", task.id),
                "fixture",
                0,
                Some(&task.plan_id),
            )
            .unwrap();
        self.save_turn_result(
            &turn.id,
            &crate::lean::turn_result::TurnResult::Completed {
                summary: report
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("Fixture result")
                    .into(),
                report: report.into(),
                commit_message: String::new(),
                selection: None,
            },
        )
        .unwrap();
        self.finish_turn(&turn.id, TurnStatus::Done, None).unwrap();
    }

    pub(in crate::lean) fn test_result_source(&self, task: &Task, producer: &Task) {
        self.db().execute("INSERT INTO task_result_sources(task_id,generation,producer_task_id) VALUES(?1,?2,?3)",params![task.id,task.input_generation,producer.id]).unwrap();
    }
}
