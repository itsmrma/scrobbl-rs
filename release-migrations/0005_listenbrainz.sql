-- Ordinals preserve repeated identical payloads within one acquisition. Incremented
-- in the same transaction as originals/checkpoints, so crash replay is harmless.
CREATE TABLE job_identity_counts (
    job_id TEXT NOT NULL REFERENCES jobs(id),
    identity_key TEXT NOT NULL,
    occurrences INTEGER NOT NULL,
    PRIMARY KEY(job_id,identity_key)
);
CREATE TABLE listenbrainz_gate (singleton INTEGER PRIMARY KEY CHECK(singleton=1), next_request_ms INTEGER NOT NULL);
INSERT INTO listenbrainz_gate VALUES(1,0);
CREATE TABLE listenbrainz_pages (
    job_id TEXT NOT NULL REFERENCES jobs(id), member TEXT NOT NULL,
    file_id TEXT NOT NULL REFERENCES source_files(id), PRIMARY KEY(job_id,member)
);
