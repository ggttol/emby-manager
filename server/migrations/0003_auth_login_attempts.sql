CREATE TABLE IF NOT EXISTS auth_login_attempts (
    id BIGSERIAL PRIMARY KEY,
    bucket TEXT NOT NULL,
    failed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_auth_login_attempts_bucket_failed
    ON auth_login_attempts(bucket, failed_at DESC);
