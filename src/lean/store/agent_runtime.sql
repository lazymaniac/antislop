-- One durable record per exchange, submission, contract version or review job.
CREATE TABLE agent_questions (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    turn_id TEXT NOT NULL REFERENCES turns(id),
    request_key TEXT NOT NULL,
    question TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending', 'awaiting_user', 'answered', 'invalidated')),
    answer TEXT,
    UNIQUE (turn_id, request_key)
);
CREATE TABLE agent_results (
    turn_id TEXT PRIMARY KEY REFERENCES turns(id),
    result TEXT NOT NULL CHECK (json_valid(result))
);
CREATE TABLE turn_inputs (
    turn_id TEXT PRIMARY KEY REFERENCES turns(id),
    generation INTEGER NOT NULL CHECK (generation >= 0)
);
CREATE TABLE plan_heads (
    plan_id TEXT PRIMARY KEY REFERENCES plans(id),
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0)
);
CREATE TABLE task_contracts (
    task_id TEXT NOT NULL REFERENCES tasks(id),
    version INTEGER NOT NULL CHECK (version >= 0),
    spec TEXT NOT NULL CHECK (json_valid(spec)),
    PRIMARY KEY (task_id, version)
);
CREATE TABLE plan_authorizations (
    plan_id TEXT PRIMARY KEY REFERENCES plans(id),
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    scope TEXT NOT NULL CHECK (json_valid(scope)),
    write_scope TEXT NOT NULL CHECK (json_valid(write_scope))
);
CREATE TABLE plan_revision_jobs (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    plan_id TEXT NOT NULL REFERENCES plans(id),
    request_key TEXT NOT NULL,
    proposal TEXT NOT NULL CHECK (json_valid(proposal)),
    status TEXT NOT NULL CHECK (status IN ('reviewing', 'applied', 'rejected', 'interrupted')),
    error TEXT,
    UNIQUE (conversation_id, request_key)
);
CREATE UNIQUE INDEX one_revision_review_per_conversation
    ON plan_revision_jobs(conversation_id) WHERE status = 'reviewing';
CREATE TABLE integration_attempts (
    task_id TEXT NOT NULL REFERENCES tasks(id),
    generation INTEGER NOT NULL,
    basis TEXT NOT NULL,
    PRIMARY KEY (task_id, generation, basis)
);
CREATE TRIGGER invalidate_questions_on_new_inputs
AFTER UPDATE OF generation ON task_inputs
WHEN OLD.generation <> NEW.generation
BEGIN
    UPDATE agent_questions SET state = 'invalidated'
    WHERE state IN ('pending', 'awaiting_user') AND turn_id IN (
        SELECT s.turn_id FROM turn_scopes s JOIN turn_inputs i ON i.turn_id = s.turn_id
        WHERE s.task_id = NEW.task_id AND i.generation <> NEW.generation
    );
END;
