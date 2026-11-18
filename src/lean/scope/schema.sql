CREATE TABLE scopes(
 id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id),
 summary TEXT NOT NULL, upto_message INTEGER NOT NULL, created_ms INTEGER NOT NULL);
CREATE TABLE plan_scopes(
 plan_id TEXT NOT NULL REFERENCES plans(id), revision INTEGER NOT NULL,
 scope_id TEXT NOT NULL REFERENCES scopes(id), PRIMARY KEY(plan_id,revision));
CREATE TABLE conversation_sources(
 id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id),
 reference TEXT NOT NULL, hash TEXT NOT NULL, created_ms INTEGER NOT NULL, UNIQUE(conversation_id,hash));
CREATE TABLE scope_documents(
 scope_id TEXT NOT NULL REFERENCES scopes(id), source_id TEXT NOT NULL REFERENCES conversation_sources(id),
 PRIMARY KEY(scope_id,source_id));
CREATE TABLE task_inputs(
 task_id TEXT PRIMARY KEY REFERENCES tasks(id), generation INTEGER NOT NULL DEFAULT 0);
CREATE TABLE scope_revision_proposals(
 request TEXT PRIMARY KEY REFERENCES plan_revision_jobs(id),
 scope_id TEXT NOT NULL REFERENCES scopes(id), base_scope TEXT NOT NULL REFERENCES scopes(id),
 fingerprint TEXT NOT NULL, affected TEXT NOT NULL, interrupted TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('awaiting_acceptance','accepted','rejected')));
