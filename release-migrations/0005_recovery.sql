CREATE TABLE recovery_assets (
    id TEXT PRIMARY KEY, job_id TEXT NOT NULL REFERENCES jobs(id),
    kind TEXT NOT NULL CHECK(kind IN ('backup','export','uploaded_backup')),
    format TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'pending',
    created_at_ms INTEGER NOT NULL, size_bytes INTEGER,
    manifest_json TEXT, verified_at_ms INTEGER, verification_error TEXT
);
