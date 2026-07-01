ALTER TABLE smart_action_runs
    ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'smart_action_runs',
    ADD COLUMN IF NOT EXISTS tab TEXT NOT NULL DEFAULT 'smart-actions',
    ADD COLUMN IF NOT EXISTS action_label TEXT NOT NULL DEFAULT '查看详情';

CREATE INDEX IF NOT EXISTS idx_smart_action_runs_source
    ON smart_action_runs(source, updated_at DESC);
