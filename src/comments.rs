//! Version-bound user annotations on graph nodes and their inspectable records.
use crate::{Result, domain::Run, hash, require};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Agent,
    Task,
    Relation,
    Artifact,
    Finding,
    Message,
    Evidence,
    Check,
    Gate,
    Session,
    Turn,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElementRef {
    pub kind: ElementKind,
    pub id: String,
    pub version_hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Element {
    pub reference: ElementRef,
    pub label: String,
    pub agent_ids: BTreeSet<String>,
    pub context: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub run_id: String,
    #[serde(default)]
    pub spec_revision: u64,
    pub target: ElementRef,
    pub context: Value,
    pub text: String,
    pub author: String,
    pub created_at: u128,
    pub resolved: bool,
    pub deleted: bool,
    pub reactions: Vec<Reaction>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReactionKind {
    Answer,
    Question,
    ProposedTask,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedReaction {
    pub comment_id: String,
    pub kind: ReactionKind,
    pub text: String,
    pub task_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reaction {
    pub round_id: String,
    pub response: ProposedReaction,
}

pub fn elements(run: &Run) -> Vec<Element> {
    let mut result = vec![];
    let mut add = |kind, id: String, label: String, agents: BTreeSet<String>, context: Value| {
        result.push(Element {
            reference: ElementRef {
                kind,
                id,
                version_hash: hash(
                    serde_json::to_vec(&(run.spec_revision, &context))
                        .expect("element serialization"),
                ),
            },
            label,
            agent_ids: agents,
            context,
        });
    };
    let mut relation_ids = BTreeSet::new();
    if let Some(team) = &run.team {
        for agent in &team.agents {
            add(
                ElementKind::Agent,
                agent.id.clone(),
                format!("Agent: {}", agent.id),
                [agent.id.clone()].into(),
                json!({"agent":agent,"tasks":team.tasks.iter().filter(|t| t.owner_id == agent.id || t.verifier_id == agent.id).collect::<Vec<_>>()}),
            );
            for (kind, target) in agent
                .supervisor_id
                .as_ref()
                .map(|id| ("supervised_by", id))
                .into_iter()
                .chain(agent.verifier_id.as_ref().map(|id| ("verified_by", id)))
            {
                let data = json!({"kind":kind,"from":agent.id,"to":target});
                let id = hash(serde_json::to_vec(&data).unwrap());
                if relation_ids.insert(id.clone()) {
                    add(
                        ElementKind::Relation,
                        id,
                        format!("Relation: {} {kind} {target}", agent.id),
                        [agent.id.clone(), target.clone()].into(),
                        data,
                    );
                }
            }
        }
        for task in &team.tasks {
            let agents = [task.owner_id.clone(), task.verifier_id.clone()].into();
            add(
                ElementKind::Task,
                task.id.clone(),
                format!("Task: {}", task.id),
                agents,
                json!(task),
            );
            for dependency in &task.dependencies {
                if let Some(upstream) = team.tasks.iter().find(|t| &t.id == dependency) {
                    let data = json!({"kind":"depends_on","from":task.id,"to":dependency});
                    let id = hash(serde_json::to_vec(&data).unwrap());
                    add(
                        ElementKind::Relation,
                        id,
                        format!("Dependency: {} -> {dependency}", task.id),
                        [task.owner_id.clone(), upstream.owner_id.clone()].into(),
                        data,
                    );
                }
            }
        }
        for (from, to) in &team.communicates_with {
            let data = json!({"kind":"communicates_with","from":from,"to":to});
            add(
                ElementKind::Relation,
                hash(serde_json::to_vec(&data).unwrap()),
                format!("Communication: {from} -> {to}"),
                [from.clone(), to.clone()].into(),
                data,
            );
        }
    } else {
        let visible: BTreeSet<_> = run
            .session_ledger
            .sessions
            .values()
            .filter(|session| {
                !run.diff_reviews
                    .iter()
                    .any(|review| session.scope.agent_id == format!("diff-review:{}", review.id))
            })
            .map(|s| s.scope.agent_id.clone())
            .collect();
        for id in visible {
            // Management context contains identities and contracts, never child transcripts.
            let scopes: Vec<_> = run
                .session_ledger
                .sessions
                .values()
                .filter(|s| s.scope.agent_id == id)
                .map(|s| &s.scope)
                .collect();
            add(
                ElementKind::Agent,
                id.clone(),
                format!("Agent: {id}"),
                [id.clone()].into(),
                json!({"id":id,"scopes":scopes}),
            );
        }
    }
    for review in &run.diff_reviews {
        let id = format!("diff-review:{}", review.id);
        add(
            ElementKind::Agent,
            id.clone(),
            format!("Diff reviewer: {}", review.id),
            [id].into(),
            json!({"review_id":review.id,"state":review.state,
                "profile":review.profile,"diff_hash":review.diff_hash,"response":review.response}),
        );
    }
    let agent_for = |session: &str| {
        run.session_ledger
            .sessions
            .get(session)
            .map(|s| s.scope.agent_id.clone())
    };
    for session in run.session_ledger.sessions.values() {
        add(
            ElementKind::Session,
            session.id.clone(),
            format!("Session: {}", session.id),
            [session.scope.agent_id.clone()].into(),
            json!(session),
        );
    }
    for turn in run.session_ledger.turns.values() {
        let agents = run
            .session_ledger
            .sessions
            .get(&turn.session_id)
            .map(|s| s.scope.agent_id.clone())
            .into_iter()
            .collect();
        add(
            ElementKind::Turn,
            turn.id.clone(),
            format!("Turn: {}", turn.id),
            agents,
            json!({"id":turn.id,"session_id":turn.session_id,"prompt_hash":turn.prompt_hash,"schema_hash":turn.schema_hash,
                "state":turn.state,"output_ref":turn.output_ref,"provider_completed":turn.provider_completed}),
        );
    }
    for artifact in &run.artifacts {
        add(
            ElementKind::Artifact,
            artifact.id.clone(),
            format!("Artifact: {}", artifact.id),
            agent_for(&artifact.session_id).into_iter().collect(),
            json!(artifact),
        );
    }
    for review in &run.reviews {
        let review_id = hash(serde_json::to_vec(review).unwrap());
        let agents: BTreeSet<_> = [&review.executor_session, &review.verifier_session]
            .into_iter()
            .filter_map(|s| agent_for(s))
            .collect();
        for (index, finding) in review.findings.iter().enumerate() {
            add(
                ElementKind::Finding,
                format!("{review_id}:{index}"),
                format!("Finding: {finding}"),
                agents.clone(),
                json!({"review_hash":review_id,"finding":finding}),
            );
        }
        for check in &review.checks {
            add(
                ElementKind::Check,
                format!("{review_id}:{}", check.check_id),
                format!("Check: {}", check.check_id),
                agents.clone(),
                json!({"review_hash":review_id,"check":check}),
            );
        }
    }
    for message in run.communication_ledger.messages.values() {
        let mut agents = BTreeSet::from([message.recipient_agent_id.clone()]);
        if let crate::communication::Sender::Agent { agent_id } = &message.sender {
            agents.insert(agent_id.clone());
        }
        add(
            ElementKind::Message,
            message.id.clone(),
            format!("Message: {}", message.id),
            agents,
            json!({"record":message,"content":run.message_bodies.get(&message.id)}),
        );
    }
    for evidence in &run.evidence {
        let agents = run
            .artifacts
            .iter()
            .filter(|a| a.hash == evidence.artifact_hash)
            .filter_map(|a| agent_for(&a.session_id))
            .collect();
        add(
            ElementKind::Evidence,
            evidence.id.clone(),
            format!("Evidence: {}", evidence.id),
            agents,
            json!(evidence),
        );
    }
    for (id, gate) in &run.quality_gates {
        let agents = run
            .reviews
            .iter()
            .filter(|r| r.artifact_hash == gate.artifact_hash)
            .flat_map(|r| [&r.executor_session, &r.verifier_session])
            .filter_map(|s| agent_for(s))
            .collect();
        add(
            ElementKind::Gate,
            id.clone(),
            format!("Gate: {id}"),
            agents,
            json!(gate),
        );
    }
    result
}

pub fn add(run: &mut Run, target: &ElementRef, text: &str) -> Result<()> {
    require(
        !text.trim().is_empty() && !text.contains('\0') && text.len() < crate::transport::MAX_FRAME,
        "invalid element comment text",
    )?;
    let element = elements(run)
        .into_iter()
        .find(|e| e.reference == *target)
        .ok_or_else(|| {
            crate::Error::Invalid("element is missing or changed; refresh before commenting".into())
        })?;
    run.element_comments.push(Comment {
        id: crate::id(),
        run_id: run.id.clone(),
        spec_revision: run.spec_revision,
        target: target.clone(),
        context: element.context,
        text: text.into(),
        author: "local_user".into(),
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        resolved: false,
        deleted: false,
        reactions: vec![],
    });
    Ok(())
}
pub fn input_hash(run: &Run) -> String {
    hash(
        serde_json::to_vec(
            &run.element_comments
                .iter()
                .map(|c| (&c.id, &c.target, &c.context, &c.text, c.resolved, c.deleted))
                .collect::<Vec<_>>(),
        )
        .expect("comment inputs serialization"),
    )
}
pub fn validate_reactions(
    run: &Run,
    reactions: &[ProposedReaction],
    proposed: Option<&crate::team::TeamPlan>,
) -> Result<()> {
    let expected: BTreeSet<_> = run
        .element_comments
        .iter()
        .filter(|c| !c.deleted && !c.resolved)
        .map(|c| c.id.as_str())
        .collect();
    let mut seen = BTreeSet::new();
    for reaction in reactions {
        require(
            expected.contains(reaction.comment_id.as_str())
                && seen.insert(reaction.comment_id.as_str())
                && !reaction.text.trim().is_empty(),
            "missing, duplicate or invented comment reaction",
        )?;
        match reaction.kind {
            ReactionKind::ProposedTask => require(
                reaction.task_id.as_ref().is_some_and(|id| {
                    proposed
                        .or(run.team.as_ref())
                        .is_some_and(|team| team.tasks.iter().any(|t| &t.id == id))
                }),
                "comment reaction proposes an unknown task",
            )?,
            _ => require(
                reaction.task_id.is_none(),
                "answer/question cannot assert a scheduled task",
            )?,
        }
    }
    require(
        seen == expected,
        "orchestrator omitted unresolved element comments",
    )
}
