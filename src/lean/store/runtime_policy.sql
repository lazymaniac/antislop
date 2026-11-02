CREATE TABLE IF NOT EXISTS provider_blocks(
    provider TEXT NOT NULL, profile TEXT NOT NULL, reason TEXT NOT NULL,
    created_ms INTEGER NOT NULL, PRIMARY KEY(provider,profile));
CREATE TABLE IF NOT EXISTS agent_answer_waits(
    request TEXT PRIMARY KEY REFERENCES agent_questions(id),
    disposition TEXT NOT NULL, target TEXT NOT NULL,
    released INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS ledger_runtime_constraints(
    run_id TEXT NOT NULL, entry_id TEXT NOT NULL, version INTEGER NOT NULL,
    constraint_kind TEXT NOT NULL,
    PRIMARY KEY(run_id,entry_id,version,constraint_kind),
    FOREIGN KEY(run_id,entry_id,version) REFERENCES ledger_versions(run_id,entry_id,version));
CREATE TABLE IF NOT EXISTS task_result_sources(
    task_id TEXT NOT NULL REFERENCES tasks(id), generation INTEGER NOT NULL,
    producer_task_id TEXT NOT NULL REFERENCES tasks(id),
    PRIMARY KEY(task_id,generation));
CREATE TABLE IF NOT EXISTS task_source_revisions(
    task_id TEXT NOT NULL REFERENCES tasks(id), generation INTEGER NOT NULL,
    revision TEXT NOT NULL, PRIMARY KEY(task_id,generation));
CREATE TABLE IF NOT EXISTS replan_blockers(
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    basis TEXT NOT NULL, resume_condition TEXT NOT NULL,
    PRIMARY KEY(conversation_id,basis));
CREATE TABLE IF NOT EXISTS artifact_deliveries(
    conversation_id TEXT NOT NULL REFERENCES conversations(id), path TEXT NOT NULL,
    task_id TEXT NOT NULL REFERENCES tasks(id), generation INTEGER NOT NULL, result_commit TEXT NOT NULL,
    PRIMARY KEY(conversation_id,path));
CREATE TABLE IF NOT EXISTS task_artifact_inputs(
    task_id TEXT NOT NULL REFERENCES tasks(id), generation INTEGER NOT NULL, path TEXT NOT NULL,
    source_task TEXT REFERENCES tasks(id), source_generation INTEGER, source_commit TEXT,
    PRIMARY KEY(task_id,generation,path));
CREATE TABLE IF NOT EXISTS agent_evidence(
 id TEXT PRIMARY KEY, turn_id TEXT NOT NULL REFERENCES turns(id), label TEXT NOT NULL,
 bytes INTEGER NOT NULL CHECK(bytes>0 AND bytes<=262144), sha256 TEXT NOT NULL,
 UNIQUE(turn_id,label));
CREATE TABLE IF NOT EXISTS result_evidence(
 turn_id TEXT NOT NULL REFERENCES agent_results(turn_id), evidence_id TEXT NOT NULL REFERENCES agent_evidence(id),
 PRIMARY KEY(turn_id,evidence_id));
CREATE INDEX IF NOT EXISTS result_evidence_by_record ON result_evidence(evidence_id);
CREATE TABLE IF NOT EXISTS task_resume_results(
 task_id TEXT NOT NULL REFERENCES tasks(id), generation INTEGER NOT NULL,
 source_task TEXT NOT NULL REFERENCES tasks(id), result_turn TEXT REFERENCES agent_results(turn_id),
 PRIMARY KEY(task_id,generation));
