CREATE TABLE IF NOT EXISTS runs(
 id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id),
 goal TEXT NOT NULL, spec_revision INTEGER NOT NULL DEFAULT 0, state TEXT NOT NULL DEFAULT 'active', start_message INTEGER NOT NULL DEFAULT 0, created_ms INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS runs_by_conversation ON runs(conversation_id,created_ms);
CREATE TABLE IF NOT EXISTS plan_runs(plan_id TEXT PRIMARY KEY REFERENCES plans(id), run_id TEXT NOT NULL REFERENCES runs(id));
CREATE TABLE IF NOT EXISTS ledger_sources(
 id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id), message INTEGER REFERENCES messages(seq),
 locator TEXT, hash TEXT NOT NULL, CHECK((message IS NULL) != (locator IS NULL)));
CREATE TABLE IF NOT EXISTS ledger_entries(
 run_id TEXT NOT NULL REFERENCES runs(id), id TEXT NOT NULL, PRIMARY KEY(run_id,id));
CREATE TABLE IF NOT EXISTS ledger_versions(
 run_id TEXT NOT NULL, entry_id TEXT NOT NULL, version INTEGER NOT NULL, spec_revision INTEGER NOT NULL,
 source_id TEXT NOT NULL REFERENCES ledger_sources(id), start_byte INTEGER NOT NULL, end_byte INTEGER NOT NULL,
 original TEXT NOT NULL, interpretation TEXT NOT NULL, mandatory INTEGER NOT NULL, position INTEGER NOT NULL,
 disposition TEXT NOT NULL CHECK(disposition IN ('active','superseded_by_user')),
 PRIMARY KEY(run_id,entry_id,version), FOREIGN KEY(run_id,entry_id) REFERENCES ledger_entries(run_id,id));
CREATE INDEX IF NOT EXISTS ledger_versions_by_revision ON ledger_versions(run_id,spec_revision,entry_id);
CREATE TABLE IF NOT EXISTS ledger_criteria(
 run_id TEXT NOT NULL, entry_id TEXT NOT NULL, version INTEGER NOT NULL, id TEXT NOT NULL,
 text TEXT NOT NULL, mandatory INTEGER NOT NULL, integration INTEGER NOT NULL,
 PRIMARY KEY(run_id,entry_id,version,id), FOREIGN KEY(run_id,entry_id,version) REFERENCES ledger_versions(run_id,entry_id,version));
CREATE TABLE IF NOT EXISTS ledger_relations(
 run_id TEXT NOT NULL, entry_id TEXT NOT NULL, version INTEGER NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('qualifier','parent','dependency','task','implementer','verifier')), target TEXT NOT NULL,
 PRIMARY KEY(run_id,entry_id,version,kind,target), FOREIGN KEY(run_id,entry_id,version) REFERENCES ledger_versions(run_id,entry_id,version));
CREATE INDEX IF NOT EXISTS ledger_dependents ON ledger_relations(run_id,kind,target);
CREATE TABLE IF NOT EXISTS ledger_decisions(
 operation_id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id), spec_revision INTEGER NOT NULL,
 author TEXT NOT NULL, reason TEXT NOT NULL, request_hash TEXT NOT NULL, result TEXT NOT NULL, created_ms INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS ledger_proposals(
 id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id), request TEXT NOT NULL, diff_hash TEXT NOT NULL, created_ms INTEGER NOT NULL, author TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS task_inputs(
 task_id TEXT PRIMARY KEY REFERENCES tasks(id), run_id TEXT NOT NULL REFERENCES runs(id), generation INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS task_ledger_inputs(
 task_id TEXT NOT NULL REFERENCES tasks(id), entry_id TEXT NOT NULL, version INTEGER NOT NULL, PRIMARY KEY(task_id,entry_id));
CREATE TABLE IF NOT EXISTS ledger_evidence(
 id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id), entry_id TEXT NOT NULL, entry_version INTEGER NOT NULL,
 criterion_id TEXT NOT NULL, task_id TEXT NOT NULL REFERENCES tasks(id), generation INTEGER NOT NULL, attempt INTEGER NOT NULL,
 producer_turn TEXT NOT NULL REFERENCES turns(id), verifier_turn TEXT NOT NULL REFERENCES turns(id),
 result TEXT NOT NULL, artifact TEXT, fingerprint TEXT NOT NULL, assessment TEXT NOT NULL, approved INTEGER NOT NULL,
 created_ms INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS evidence_by_criterion ON ledger_evidence(run_id,entry_id,entry_version,criterion_id);
CREATE TABLE IF NOT EXISTS ledger_reviews(
 run_id TEXT NOT NULL REFERENCES runs(id), spec_revision INTEGER NOT NULL, kind TEXT NOT NULL,
 producer_turn TEXT NOT NULL REFERENCES turns(id), verifier_turn TEXT NOT NULL REFERENCES turns(id),
 fingerprint TEXT NOT NULL, approved INTEGER NOT NULL, note TEXT NOT NULL, created_ms INTEGER NOT NULL,
 id INTEGER PRIMARY KEY AUTOINCREMENT);
CREATE TABLE IF NOT EXISTS ledger_turns(
 turn_id TEXT PRIMARY KEY REFERENCES turns(id), run_id TEXT NOT NULL REFERENCES runs(id), spec_revision INTEGER NOT NULL, phase TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS ledger_source_coverage(
 turn_id TEXT NOT NULL REFERENCES turns(id), source_id TEXT NOT NULL REFERENCES ledger_sources(id), PRIMARY KEY(turn_id,source_id));
CREATE UNIQUE INDEX IF NOT EXISTS ledger_proposal_operations ON ledger_proposals(json_extract(request,'$.operation_id'));
CREATE TABLE IF NOT EXISTS ledger_proposal_tasks(
 proposal_id TEXT NOT NULL REFERENCES ledger_proposals(id), task_id TEXT NOT NULL REFERENCES tasks(id), PRIMARY KEY(proposal_id,task_id));

CREATE TABLE IF NOT EXISTS planning_runs(
 conversation_id TEXT PRIMARY KEY REFERENCES conversations(id), run_id TEXT NOT NULL REFERENCES runs(id));
CREATE TABLE IF NOT EXISTS requirements_resumptions(
 operation_id TEXT PRIMARY KEY REFERENCES ledger_decisions(operation_id), message INTEGER NOT NULL REFERENCES messages(seq));
CREATE TABLE IF NOT EXISTS solution_choices(
 run_id TEXT NOT NULL REFERENCES runs(id), spec_revision INTEGER NOT NULL,
 producer_turn TEXT NOT NULL REFERENCES turns(id), verifier_turn TEXT NOT NULL REFERENCES turns(id),
 decision TEXT NOT NULL, PRIMARY KEY(run_id,spec_revision));

CREATE TABLE IF NOT EXISTS ledger_rejections(
 proposal_id TEXT PRIMARY KEY REFERENCES ledger_proposals(id),
 comment_message INTEGER NOT NULL REFERENCES messages(seq),
 resume_message INTEGER NOT NULL REFERENCES messages(seq),
 diff_hash TEXT NOT NULL, created_ms INTEGER NOT NULL);
