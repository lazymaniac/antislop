use super::*;
use crate::{
    Error,
    config::{RoleKind, WorkKind},
    lean::{Core, TurnSetup, agent, git, tasks},
};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

impl Core {
    pub fn ledger_fingerprint(&self, conversation: &str) -> Result<String> {
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        git::integrated_fingerprint(&project)
    }
    pub fn ledger_query(&self, mut query: LedgerQuery, kind: LedgerKind) -> Result<LedgerPage> {
        if query.run_id.is_empty() {
            query.run_id = self
                .store
                .planning_run(&query.conversation)?
                .ok_or_else(|| {
                    Error::Invalid(
                        "no run; requirements are extracted when a plan is proposed".into(),
                    )
                })?
                .id;
        }
        let fp = self
            .ledger_fingerprint(&query.conversation)
            .unwrap_or_default();
        self.store.list_ledger(&query, kind, &fp)
    }
    pub fn apply_user_ledger_change(
        self: &Arc<Self>,
        proposal_id: &str,
        diff_hash: &str,
        change: &LedgerChange,
    ) -> Result<ChangeResult> {
        // The same mutex protects both scope publication and the last check before Git integration.
        let _integration = self.integrating.lock().expect("integration mutex");
        self.store
            .check_ledger_proposal(proposal_id, diff_hash, change)?;
        let result = match self.store.applied_ledger_change(change)? {
            Some(saved) => saved,
            None => {
                let current = self.store.preview_ledger_change(change, &self.state)?;
                require(
                    super::diff_hash(&current)? == diff_hash,
                    "ledger impact changed; preview the current diff before applying",
                )?;
                self.store.apply_ledger_change(change, &self.state, true)?
            }
        };
        let flags = self.task_cancels.lock().expect("task cancel mutex");
        for task in &result.affected_tasks {
            if let Some((_, flag)) = flags.get(task) {
                flag.store(true, Ordering::SeqCst);
            }
        }
        drop(flags);
        for id in &result.affected_tasks {
            if let Some(task) = self.store.task_by_id(id)? {
                self.announce_task(
                    &change.conversation,
                    &task,
                    Some("scope changed; previous result requires reassessment"),
                );
            }
        }
        drop(_integration);
        if self
            .store
            .has_pending_requirements_approval(&change.conversation)?
        {
            self.wake(&change.conversation);
        }
        Ok(result)
    }
    pub(in crate::lean) fn ledger_context(
        &self,
        conversation: &str,
        run_id: &str,
    ) -> Result<String> {
        Ok(format!(
            "\nRun and ledger inputs (stable identities; keep their versions):\n{}\nSources: {}\nEntries: {}\nNever weaken an obligation. Propose changes for user approval. Worker approval does not verify requirements.\n",
            serde_json::to_string(&self.store.run(conversation, run_id)?)?,
            serde_json::to_string(&self.store.ledger_sources(conversation, run_id)?)?,
            serde_json::to_string(&self.store.ledger_entries(conversation, run_id, None)?.iter().map(|e|json!({"id":e.spec.id,"version":e.version,"spec_revision":e.spec_revision,"spec":e.spec})).collect::<Vec<_>>())?
        ) + &self.store.solution_choice(conversation, run_id)?.map(|choice| format!("\nReviewed implementation options: {}\n", serde_json::to_string(&choice).unwrap_or_default())).unwrap_or_default())
    }
    pub(super) fn requirements_turn(
        self: &Arc<Self>,
        conversation: &str,
        run_id: &str,
        phase: &str,
        verifier: bool,
        prompt: String,
        cancel: &AtomicBool,
    ) -> Result<(String, String)> {
        let progress = match (phase, verifier) {
            ("extraction", false) => {
                "Preparing requirements from the interview and source documents."
            }
            ("extraction", true) => "Checking requirements independently before planning.",
            ("options", false) => "Comparing implementation alternatives.",
            ("options", true) => "Checking the solution comparison independently.",
            (_, false) => "Auditing the integrated implementation against requirements.",
            (_, true) => "Verifying the implementation audit independently.",
        };
        self.store.append_event(
            conversation,
            None,
            None,
            "planning_progress",
            &json!({"text":progress}),
        )?;
        let role = format!(
            "requirements-{phase}-{}",
            if verifier { "verifier" } else { "analyst" }
        );
        let setup = TurnSetup {
            conversation,
            plan_id: None,
            role: role.clone(),
            profile: self.config_snapshot().resolve(
                if phase == "options" {
                    WorkKind::Options
                } else {
                    WorkKind::Requirements
                },
                if verifier {
                    RoleKind::Verifier
                } else {
                    RoleKind::Executor
                },
            ),
            cwd: PathBuf::from(self.store.conversation(conversation)?.project),
            add_dirs: vec![],
            access: agent::Access::ReadOnly,
            upto_message: 0,
        };
        let source_ids = self
            .store
            .ledger_sources(conversation, run_id)?
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>();
        let revision = self.store.run(conversation, run_id)?.spec_revision;
        let context = self.ledger_context(conversation, run_id)?;
        let answer = self
            .turn(setup, cancel, |_| format!("{prompt}\n{context}"))?
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "{role} failed: {}",
                    self.store
                        .last_turn(conversation, &role)
                        .ok()
                        .flatten()
                        .and_then(|t| t.error)
                        .unwrap_or_else(|| "no response".into())
                ))
            })?;
        let turn = self
            .store
            .last_turn(conversation, &role)?
            .ok_or_else(|| Error::Invalid("requirements turn missing".into()))?;
        self.store.bind_ledger_turn(
            conversation,
            run_id,
            &turn.id,
            revision,
            phase,
            &source_ids,
        )?;
        Ok((answer, turn.id))
    }
    pub(super) fn full_ledger_sources(&self, conversation: &str, run_id: &str) -> Result<String> {
        let mut out = String::new();
        let messages = self.store.messages_after(conversation, 0)?;
        for source in self.store.ledger_sources(conversation, run_id)? {
            if let Some(seq) = source.message {
                if let Some(index) = messages.iter().position(|message| message.seq == seq)
                    && messages[index].text.starts_with("Interview answer (")
                    && let Some(question) = messages[..index]
                        .iter()
                        .rev()
                        .find(|message| message.author == "orchestrator")
                {
                    out.push_str(&format!(
                        "\nInterview question context for source {} (interpret the selected answer using its option description; this is context, not a separate user obligation; byte ranges refer only to the user source):\n{}\n",
                        source.id, question.text
                    ));
                }
                out.push_str(&format!(
                    "\nFull user source {}:\n{}\n",
                    source.id,
                    self.store
                        .ledger_source_text(conversation, run_id, &source.id, &self.state)?
                ));
            } else {
                out.push_str(&format!("\nRead the full immutable document {} at {}. Hash {}. Do not rely on excerpts.\n",source.id,self.state.join(source.locator.unwrap_or_default()).display(),source.hash));
            }
        }
        if let Some(workflow) = self.store.workflow(conversation)?
            && !workflow.summary.is_empty()
        {
            out.push_str(&format!(
                "\nInterview summary (derived context; verify against user sources and selected option descriptions, never treat additions as user authorization):\n{}\n",
                workflow.summary
            ));
        }
        Ok(out)
    }
    /// Extraction is separate from task planning and has a separate verifier session.
    pub(in crate::lean) fn ensure_requirements(
        self: &Arc<Self>,
        conversation: &str,
        cancel: &AtomicBool,
    ) -> Result<Run> {
        let user = self.store.messages_by(conversation, "user", 0)?;
        let r = match self.store.planning_run(conversation)? {
            Some(r) => r,
            None => self.store.create_run(
                conversation,
                &tasks::bound(
                    &user
                        .first()
                        .ok_or_else(|| Error::Invalid("run needs a user request".into()))?
                        .text,
                    1000,
                ),
            )?,
        };
        self.store.select_planning_run(conversation, &r.id)?;
        if let Some(task) = self.store.latest_plan(conversation)?.first()
            && self.store.plan_run(&task.plan_id)?.is_none()
            && let Some(document) = self.store.plan_document(conversation, &task.plan_id)?
        {
            self.store.register_document_source(conversation,&r.id,&format!("Legacy agent plan (input only; not verification or authority to weaken user requirements):\n{document}"),&self.state)?;
        }
        let mut new_source = false;
        let known = self.store.ledger_sources(conversation, &r.id)?;
        for message in user.iter().filter(|m| m.seq >= r.start_message) {
            if !known.iter().any(|s| s.message == Some(message.seq)) {
                self.store
                    .register_message_source(conversation, &r.id, message.seq)?;
                new_source = true;
            }
        }
        if self.store.extraction_current(conversation, &r.id)? && !new_source {
            return Ok(r);
        }
        let sources = self.full_ledger_sources(conversation, &r.id)?;
        let producer = if let Some(producer) = self
            .store
            .approved_requirements_producer(conversation, &r.id, r.spec_revision)?
            .filter(|_| !new_source)
        {
            producer
        } else {
            let existing = self.store.ledger_entries(conversation, &r.id, None)?;
            let review_feedback =
                self.store
                    .messages_by(conversation, "requirements-reviewer", r.start_message)?;
            let review_feedback = review_feedback
                .last()
                .map(|message| message.text.as_str())
                .unwrap_or("");
            let prompt = format!(
                "Act as requirements analyst before planning. Read ALL sources, including middle sections. Preserve qualifiers, exceptions, negative obligations, original byte ranges, parents and dependencies. parent must reference an entry of the SAME kind, or be null. Link plan_item entries to requirements using requirement_ids, never parent. All references must exist in the combined existing and proposed ledger. Split acceptance criteria with globally unique stable IDs; composite parents need mandatory integration criteria. Preserve every supplied plan step. A prescribed implementation method is also a separate requirement. Technical steps need task_ids, technical_goal, check and criteria. task_ids are proposed planner IDs. Return a ```ledger JSON block with {{\"run_id\":\"{}\",\"expected_spec_revision\":{},\"entries\":[EntrySpec...]}}. EntrySpec fields: id, kind (requirement|plan_item), source (source_id,start,end byte range on grapheme boundaries), interpretation, qualifiers, parent, depends_on, mandatory, implementation_owners, verification_owners, criteria (id,text,mandatory,integration), requirement_ids, task_ids, position, technical_goal, check, disposition (active). Include only additions or proposed semantic changes. Existing obligations must retain stable IDs. Never silently supersede or weaken obligations. For a planning-only request, distinguish acceptance tests the plan must specify from results checked during later execution. Do not invent implementation tasks that have not been planned. Previous independent review feedback (correct omissions; semantic changes still require user approval): {review_feedback}. Sources:\n{sources}",
                r.id, r.spec_revision
            );
            let mut feedback = String::new();
            let (change, producer) = loop {
                if cancel.load(Ordering::SeqCst) {
                    return Err(Error::Invalid("requirements extraction cancelled".into()));
                }
                let (answer, producer) = self.requirements_turn(
                    conversation,
                    &r.id,
                    "extraction",
                    false,
                    format!("{prompt}{feedback}"),
                    cancel,
                )?;
                let validated = (|| -> Result<LedgerChange> {
                    let mut extraction: Extraction = parse_block(&answer, "ledger")?;
                    require(
                        extraction.run_id == r.id
                            && extraction.expected_spec_revision == r.spec_revision,
                        "analyst returned wrong run or revision",
                    )?;
                    extraction
                        .entries
                        .retain(|entry| !existing.iter().any(|old| old.spec == *entry));
                    require(
                        !extraction.entries.is_empty() || !existing.is_empty(),
                        "requirements extraction is empty",
                    )?;
                    let change = LedgerChange {
                        conversation: conversation.into(),
                        run_id: r.id.clone(),
                        expected_spec_revision: r.spec_revision,
                        operation_id: crate::id(),
                        reason: "source extraction".into(),
                        entries: extraction.entries,
                    };
                    if !change.entries.is_empty() {
                        self.store.preview_ledger_change(&change, &self.state)?;
                    }
                    Ok(change)
                })();
                match validated {
                    Ok(change) => break (change, producer),
                    Err(error @ (Error::Invalid(_) | Error::Json(_))) if feedback.is_empty() => {
                        self.store.append_event(
                        conversation,
                        None,
                        None,
                        "planning_progress",
                        &json!({"text": format!("Correcting requirements extraction: {error}")}),
                    )?;
                        feedback = format!(
                            "\nThe previous extraction was rejected: {error}. Correct the complete ledger JSON without dropping obligations or weakening criteria. Previous response:\n{answer}"
                        );
                    }
                    Err(error) => return Err(error),
                }
            };
            if !change.entries.is_empty() {
                if change
                    .entries
                    .iter()
                    .any(|e| existing.iter().any(|old| old.spec.id == e.id))
                {
                    let (proposal, diff) =
                        self.store
                            .propose_agent_ledger_change(&change, &self.state, &producer)?;
                    self.store.append_event(conversation,None,None,"ledger_approval_required",&json!({"proposal":proposal,"run":r.id,"revision":r.spec_revision,"affected_tasks":diff.affected_tasks}))?;
                    return Err(Error::Invalid("requirements analyst proposed a semantic scope change; open /requirements and press p to review the exact diff before planning".into()));
                }
                let _integration = self.integrating.lock().expect("integration mutex");
                self.store
                    .apply_analyst_ledger_change(&change, &self.state, &producer)?;
            }
            producer
        };
        let current = self.store.run(conversation, &r.id)?;
        require(
            !self
                .store
                .ledger_entries(conversation, &r.id, None)?
                .is_empty(),
            "requirements extraction is empty",
        )?;
        let (answer,verifier)=self.requirements_turn(conversation,&r.id,"extraction",true,format!(
            "Independently compare the requirement entries to every full source and the interview context. This is extraction before task planning: the task/plan-item ledger may legitimately be absent. Do not reject solely because a second ledger or execution evidence does not exist yet. Selected interview options include their descriptions; use that context before declaring ambiguity. For planning-only requests, completion criteria describe acceptance tests for later implementation, not a demand to implement now. Do not add obligations unsupported by user sources or project instructions. Find omitted obligations, lost qualifiers, ambiguous interpretation, missing plan steps, prescribed methods and missing composite integration criteria. Do not accept analyst assertions as evidence. Read full document artifacts. Return a ```verdict JSON block {{\"approved\":true|false,\"findings\":[{{\"issue\":\"...\"}}]}}. Full sources:\n{sources}"),cancel)?;
        let verdict = tasks::parse_verdict(&answer);
        self.store.record_ledger_review(
            conversation,
            &r.id,
            current.spec_revision,
            "extraction",
            &producer,
            &verifier,
            "",
            verdict.approved,
            &answer,
        )?;
        if !verdict.approved
            && !cancel.load(Ordering::SeqCst)
            && self
                .store
                .count_since_user(conversation, "requirements-reviewer")?
                == 0
        {
            self.store.add_message(conversation, "requirements-reviewer", &format!(
                "Independent requirements review rejected run {} revision {}. Revisit the extraction using this feedback and the full interview context, then regenerate the plan. Do not weaken obligations or bypass scope-change approval. Findings: {}",
                r.id, current.spec_revision, tasks::findings_text(&verdict.findings)
            ))?;
        }
        require(
            verdict.approved,
            &format!(
                "requirements source coverage rejected: {}",
                tasks::findings_text(&verdict.findings)
            ),
        )?;
        Ok(current)
    }
    pub(in crate::lean) fn validate_ledger_plan(
        &self,
        conversation: &str,
        plan: &str,
        run_id: &str,
    ) -> Result<()> {
        require(
            self.store.extraction_current(conversation, run_id)?,
            "requirements source coverage needs independent verification",
        )?;
        let tasks = self.store.plan(plan)?;
        if tasks
            .iter()
            .all(|t| self.store.task_has_ledger_inputs(&t.id).unwrap_or(false))
        {
            for t in &tasks {
                self.store.assert_task_inputs(t)?;
            }
            Ok(())
        } else {
            self.store.bind_task_ledger(conversation, run_id, plan)
        }
    }
    /// Auditors inspect the primary integrated tree. Worktree approval is never imported.
    pub(in crate::lean) fn compliance_audit(
        self: &Arc<Self>,
        conversation: &str,
        plan: &str,
        cancel: &AtomicBool,
    ) -> Result<Vec<String>> {
        let Some(run_id) = self.store.plan_run(plan)? else {
            return Ok(vec!["legacy plan has no verified requirement ledger; extract and replan before claiming success".into()]);
        };
        let revision = self.store.run(conversation, &run_id)?.spec_revision;
        let fingerprint = self.ledger_fingerprint(conversation)?;
        let sources = self.full_ledger_sources(conversation, &run_id)?;
        let tasks = self.store.run_tasks(conversation, &run_id)?;
        let (answer,producer)=self.requirements_turn(conversation,&run_id,"audit",false,format!(
            "Audit the integrated project against every full source and both ledgers. Search for obligations omitted in extraction. Inspect actual files and run relevant read-only checks. Do not change files. Integrated fingerprint: {fingerprint}. Task inputs and reports: {}. Return ```compliance JSON {{\"approved\":true|false,\"findings\":[...],\"evidence\":[EvidenceProposal...]}}. Each evidence proposal: entry_id,entry_version,criterion_id,task_id (UUID),generation,attempt (round),producer_turn (leave empty; runtime supplies),result (concrete checks and observed results, never just done),artifact (optional),integrated_fingerprint. Include technical step execution and composite integration criteria. Evidence must concern this integrated tree. Full sources:\n{sources}",serde_json::to_string(&tasks)?),cancel)?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Audit {
            approved: bool,
            findings: Vec<serde_json::Value>,
            evidence: Vec<EvidenceProposal>,
        }
        let mut proposed: Audit = parse_block(&answer, "compliance")?;
        for e in &mut proposed.evidence {
            e.producer_turn = producer.clone();
            require(
                e.integrated_fingerprint == fingerprint,
                "auditor inspected a different integrated result",
            )?;
        }
        let (answer,verifier)=self.requirements_turn(conversation,&run_id,"audit",true,format!(
            "Verify this audit independently against ALL full sources, both ledgers and actual integrated files. Find omitted obligations. Independently reproduce/check every evidence result. Reject empty, stale, failed or worktree-only evidence. Read-only access. Integrated fingerprint: {fingerprint}. Analyst findings: {}. Proposed evidence: {}. Return ```compliance_verdict JSON {{\"approved\":true|false,\"note\":\"coverage and gaps\",\"assessments\":[{{\"evidence\":EvidenceProposal,\"approved\":true|false,\"note\":\"independent observation\"}}]}}. Preserve exact proposal fields; runtime owns session identity. Full sources:\n{sources}",serde_json::to_string(&proposed.findings)?,serde_json::to_string(&proposed.evidence)?),cancel)?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Review {
            approved: bool,
            note: String,
            assessments: Vec<Assessment>,
        }
        let reviewed: Review = parse_block(&answer, "compliance_verdict")?;
        let _integration = self.integrating.lock().expect("integration mutex");
        require(
            self.ledger_fingerprint(conversation)? == fingerprint,
            "integrated files changed during compliance audit",
        )?;
        require(
            self.store.run(conversation, &run_id)?.spec_revision == revision,
            "scope changed during compliance audit",
        )?;
        for assessment in &reviewed.assessments {
            require(
                proposed.evidence.contains(&assessment.evidence),
                "verifier assessed evidence not proposed by this audit",
            )?;
            self.store
                .add_ledger_evidence(conversation, &run_id, &verifier, assessment)?;
        }
        self.store.record_ledger_review(
            conversation,
            &run_id,
            revision,
            "audit",
            &producer,
            &verifier,
            &fingerprint,
            proposed.approved && reviewed.approved,
            &reviewed.note,
        )?;
        let gaps = self
            .store
            .requirements_gaps(conversation, &run_id, &fingerprint)?;
        self.store
            .set_run_completion(conversation, &run_id, revision, gaps.is_empty())?;
        self.store.append_event(
            conversation,
            None,
            Some(plan),
            "requirements_complete",
            &json!({"run":run_id,"spec_revision":revision,"approved":gaps.is_empty(),"gaps":gaps}),
        )?;
        Ok(gaps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn extraction_repairs_invalid_parent_once_and_preserves_atomicity() {
        for (repairs, reject_review) in [(true, false), (false, false), (true, true)] {
            let dir = tempfile::tempdir().unwrap();
            let script = dir.path().join("analyst");
            std::fs::write(&script, r#"#!/usr/bin/env python3
import json,sys,uuid,os
prompt=sys.argv[-1]
lines=prompt.rsplit('Run and ledger inputs (stable identities; keep their versions):\n',1)[1].splitlines()
run=json.loads(lines[0])
assert 'selected option details' in prompt
if prompt.startswith('Independently compare'):
    assert 'may legitimately be absent' in prompt
    answer='```verdict\n'+json.dumps({'approved':not os.path.exists('reject-review'),'findings':[{'issue':'missing qualifier'}] if os.path.exists('reject-review') else []})+'\n```'
else:
    assert 'SAME kind' in prompt
    n=int(open('attempts').read()) if os.path.exists('attempts') else 0
    open('attempts','w').write(str(n+1))
    if n == 1:
        assert 'entry p has unknown or incompatible parent r' in prompt, prompt
        assert 'Previous response:' in prompt
    source=json.loads(lines[1].removeprefix('Sources: '))[0]['id']
    base={'source':{'source_id':source,'start':0,'end':4},'interpretation':'Keep scope','criteria':[{'id':'r.c','text':'Check scope','mandatory':True,'integration':True}]}
    r=dict(base,id='r',kind='requirement')
    p=dict(base,id='p',kind='plan_item',parent=None if n and os.path.exists('repair') else 'r',requirement_ids=['r'],task_ids=['t'],criteria=[{'id':'p.c','text':'Check task','mandatory':True}])
    answer='```ledger\n'+json.dumps({'run_id':run['id'],'expected_spec_revision':run['spec_revision'],'entries':[r,p]})+'\n```'
print(json.dumps({'type':'thread.started','thread_id':str(uuid.uuid4())}))
print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
print(json.dumps({'type':'turn.completed','usage':{}}))
"#).unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            if repairs {
                std::fs::write(dir.path().join("repair"), "").unwrap();
            }
            if reject_review {
                std::fs::write(dir.path().join("reject-review"), "").unwrap();
            }
            let config = std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/examples/antislop.toml"
            ))
            .unwrap()
            .replace(
                "binary = \"codex\"",
                &format!("binary = {:?}", script.to_str().unwrap()),
            )
            .replace("provider = \"agy\"", "provider = \"codex\"");
            let core = Core::open(
                &dir.path().join("state"),
                Config::from_toml(&config).unwrap(),
            )
            .unwrap();
            let conversation = core
                .store
                .create_conversation(dir.path().to_str().unwrap())
                .unwrap()
                .id;
            core.store
                .add_message(&conversation, "orchestrator", "selected option details")
                .unwrap();
            core.store
                .add_message(&conversation, "user", "Interview answer (focus)\nKeep")
                .unwrap();
            let result = core.ensure_requirements(&conversation, &AtomicBool::new(false));
            assert_eq!(
                std::fs::read_to_string(dir.path().join("attempts")).unwrap(),
                "2"
            );
            let run = core.store.latest_run(&conversation).unwrap().unwrap();
            let entries = core
                .store
                .ledger_entries(&conversation, &run.id, None)
                .unwrap();
            if reject_review {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("missing qualifier")
                );
                assert_eq!(entries.len(), 2);
                assert_eq!(
                    core.store
                        .count_since_user(&conversation, "requirements-reviewer")
                        .unwrap(),
                    1
                );
                let repeated = core
                    .ensure_requirements(&conversation, &AtomicBool::new(false))
                    .unwrap_err();
                assert!(
                    repeated.to_string().contains("missing qualifier"),
                    "unchanged entries must not create a scope approval: {repeated}"
                );
                assert_eq!(
                    core.store
                        .count_since_user(&conversation, "requirements-reviewer")
                        .unwrap(),
                    1
                );
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("attempts")).unwrap(),
                    "3"
                );
            } else if repairs {
                result.unwrap();
                assert_eq!(entries.len(), 2);
                assert_eq!(run.spec_revision, 1);
                assert!(
                    core.store
                        .extraction_current(&conversation, &run.id)
                        .unwrap()
                );
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("entry p has unknown or incompatible parent r")
                );
                assert!(entries.is_empty());
                assert_eq!(run.spec_revision, 0);
            }
        }
    }
}
