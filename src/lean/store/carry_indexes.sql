-- Additive indexes for authoritative demand-driven carry selection. Installed
-- transactionally on both new and existing supported databases; no history rewrite.
CREATE INDEX IF NOT EXISTS tasks_carry_by_identity ON tasks(
 conversation_id,CASE WHEN json_valid(spec) THEN json_extract(spec,'$.id') END,created_ms
) WHERE (status IN ('approved','stale') OR (status IN ('coordinating','failed','cancelled','needs_user','waiting','queued') AND (report IS NOT NULL OR result IS NOT NULL)));
CREATE INDEX IF NOT EXISTS tasks_carry_latest ON tasks(conversation_id,created_ms)
 WHERE (status IN ('approved','stale') OR (status IN ('coordinating','failed','cancelled','needs_user','waiting','queued') AND (report IS NOT NULL OR result IS NOT NULL)));
CREATE INDEX IF NOT EXISTS tasks_plan_identity ON tasks(plan_id,CASE WHEN json_valid(spec) THEN json_extract(spec,'$.id') END) WHERE status<>'superseded';
