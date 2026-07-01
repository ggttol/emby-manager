CREATE TABLE IF NOT EXISTS smart_action_runs (
    id UUID PRIMARY KEY,
    action_type TEXT NOT NULL,
    status TEXT NOT NULL,
    subject JSONB NOT NULL,
    title TEXT NOT NULL,
    summary TEXT NOT NULL,
    recommendation JSONB NOT NULL,
    evidence JSONB NOT NULL,
    plan JSONB NOT NULL,
    risk JSONB NOT NULL,
    policy JSONB NOT NULL,
    verification JSONB NOT NULL,
    task_id UUID NULL REFERENCES task_runs(id),
    result JSONB,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_smart_action_runs_status
    ON smart_action_runs(status, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_smart_action_runs_action_type
    ON smart_action_runs(action_type, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_smart_action_runs_subject
    ON smart_action_runs USING gin(subject);

CREATE INDEX IF NOT EXISTS idx_smart_action_runs_evidence
    ON smart_action_runs USING gin(evidence);

CREATE TABLE IF NOT EXISTS smart_action_policies (
    key TEXT PRIMARY KEY,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    mode TEXT NOT NULL DEFAULT 'confirm',
    max_risk TEXT NOT NULL DEFAULT 'medium',
    params JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
