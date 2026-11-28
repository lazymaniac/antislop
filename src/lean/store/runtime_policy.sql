CREATE TABLE IF NOT EXISTS agent_answer_waits(
    request TEXT PRIMARY KEY REFERENCES agent_questions(id),
    disposition TEXT NOT NULL, target TEXT NOT NULL,
    released INTEGER NOT NULL DEFAULT 0);
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
CREATE TABLE IF NOT EXISTS session_documents (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    producer_task TEXT NOT NULL REFERENCES tasks(id),
    path TEXT NOT NULL,
    source_commit TEXT NOT NULL,
    content_ref TEXT NOT NULL CHECK (json_valid(content_ref)),
    created_ms INTEGER NOT NULL,
    UNIQUE(producer_task, path, source_commit)
);
CREATE INDEX IF NOT EXISTS session_documents_by_conversation
    ON session_documents(conversation_id, created_ms, id);
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
CREATE TABLE IF NOT EXISTS approval_modes(
 conversation_id TEXT PRIMARY KEY REFERENCES conversations(id),
 mode TEXT NOT NULL CHECK(mode IN ('supervised','autonomous')));
CREATE TABLE IF NOT EXISTS teams(
 conversation_id TEXT PRIMARY KEY REFERENCES conversations(id),
 team TEXT NOT NULL, reason TEXT NOT NULL, version INTEGER NOT NULL, updated_ms INTEGER NOT NULL,
 user_selected INTEGER NOT NULL DEFAULT 0 CHECK(user_selected IN (0,1)),
 origin TEXT NOT NULL DEFAULT 'unknown' CHECK(origin IN ('unknown','user','planner')));
CREATE TABLE IF NOT EXISTS approvals(
 id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id),
 turn_id TEXT NOT NULL, role TEXT NOT NULL, provider TEXT NOT NULL,
 kind TEXT NOT NULL, summary TEXT NOT NULL, options TEXT NOT NULL, fingerprint TEXT NOT NULL,
 decider TEXT NOT NULL CHECK(decider IN ('user','orchestrator')),
 state TEXT NOT NULL CHECK(state IN ('open','answered','closed')),
 choice TEXT, answered_by TEXT, created_ms INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS approvals_by_conversation ON approvals(conversation_id,state);
CREATE TABLE IF NOT EXISTS request_operations(
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 operation_id TEXT NOT NULL, kind TEXT NOT NULL, result TEXT NOT NULL,
 created_ms INTEGER NOT NULL,
 PRIMARY KEY(conversation_id,operation_id));
CREATE TABLE IF NOT EXISTS runtime_activations(
 id TEXT PRIMARY KEY, signature TEXT NOT NULL, executable TEXT NOT NULL, build TEXT NOT NULL,
 project TEXT NOT NULL, config TEXT NOT NULL, launch_path TEXT NOT NULL,
 task TEXT, conversation TEXT, generation INTEGER, config_revision TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('validating','waiting_for_idle','switching','applied','failed','cancelled','interrupted')),
 error TEXT, log TEXT, diagnostic_tail TEXT, created_ms INTEGER NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS runtime_activation_active ON runtime_activations((1))
 WHERE state IN ('validating','waiting_for_idle','switching');
CREATE TABLE IF NOT EXISTS runtime_activation_conversations(
 operation TEXT NOT NULL REFERENCES runtime_activations(id), conversation TEXT NOT NULL,
 plan TEXT, cancelled INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(operation,conversation));
CREATE TABLE IF NOT EXISTS runtime_activation_events(
 seq INTEGER PRIMARY KEY, operation TEXT NOT NULL REFERENCES runtime_activations(id),
 state TEXT NOT NULL, error TEXT, created_ms INTEGER NOT NULL);

CREATE TABLE IF NOT EXISTS agent_answer_receipts(
 request TEXT PRIMARY KEY REFERENCES agent_questions(id) ON DELETE CASCADE, conversation TEXT NOT NULL,
 signature TEXT NOT NULL, result TEXT NOT NULL, resume_pending INTEGER NOT NULL DEFAULT 1);

CREATE TABLE IF NOT EXISTS session_titles(
 conversation TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
 title TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS message_classifications(
 message INTEGER PRIMARY KEY REFERENCES messages(seq) ON DELETE CASCADE,
 kind TEXT NOT NULL, reason TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS runtime_validation_processes(
 operation TEXT NOT NULL REFERENCES runtime_activations(id) ON DELETE CASCADE,
 phase TEXT NOT NULL, pid INTEGER NOT NULL, started_ms INTEGER NOT NULL, executable TEXT NOT NULL,
 PRIMARY KEY(operation,phase));
CREATE TABLE IF NOT EXISTS message_images(
  message_seq INTEGER NOT NULL REFERENCES messages(seq) ON DELETE CASCADE,
  image_id TEXT NOT NULL, mime_type TEXT NOT NULL, bytes INTEGER NOT NULL,
  name TEXT NOT NULL, PRIMARY KEY(message_seq,image_id));

CREATE TABLE IF NOT EXISTS validation_runs(
 id TEXT PRIMARY KEY,
 conversation TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
 turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
 identity TEXT NOT NULL, tree TEXT, spec TEXT NOT NULL CHECK(json_valid(spec)),
 status TEXT NOT NULL CHECK(status IN ('running','passed','failed','changed','interrupted','invalidated','uncacheable')),
 exit_code INTEGER, problem TEXT NOT NULL DEFAULT '',
 evidence_id TEXT REFERENCES agent_evidence(id) ON DELETE SET NULL,
 created_ms INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS validation_runs_by_identity ON validation_runs(conversation,identity);
CREATE TABLE IF NOT EXISTS validation_observations(
 validation_id TEXT PRIMARY KEY REFERENCES validation_runs(id) ON DELETE CASCADE,
 observation TEXT NOT NULL CHECK(json_valid(observation))
);
CREATE TABLE IF NOT EXISTS validation_uses(
 turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
 validation_id TEXT NOT NULL REFERENCES validation_runs(id) ON DELETE CASCADE,
 PRIMARY KEY(turn_id,validation_id));
