-- Each variant owns one original payload. Observations and accepted occurrences
-- reference the same bytes; invalid/skipped rows still own their complete input.
-- Rebuild the three tables atomically, preserving all identities and provenance.
CREATE TABLE original_payloads (id INTEGER PRIMARY KEY, raw_json TEXT NOT NULL);
INSERT INTO original_payloads SELECT rowid,raw_json FROM observation_variants;
CREATE TEMP TABLE payload_offsets AS SELECT
    COALESCE((SELECT MAX(rowid) FROM observation_variants),0) AS variants,
    COALESCE((SELECT MAX(rowid) FROM observations),0) AS observations;
CREATE TEMP TABLE observation_payload_map AS
SELECT o.rowid AS old_rowid, COALESCE((
    SELECT v.rowid FROM observation_variants v
    WHERE v.observation_id=o.id AND v.raw_json=o.raw_json LIMIT 1
), (SELECT variants FROM payload_offsets)+o.rowid) AS payload_id
FROM observations o;
CREATE UNIQUE INDEX observation_payload_map_row ON observation_payload_map(old_rowid);
INSERT INTO original_payloads
SELECT m.payload_id,o.raw_json FROM observations o
JOIN observation_payload_map m ON m.old_rowid=o.rowid
WHERE m.payload_id>(SELECT variants FROM payload_offsets);
CREATE TEMP TABLE occurrence_payload_map AS
SELECT p.rowid AS old_rowid, CASE WHEN v.raw_json=p.raw_json THEN v.rowid
    ELSE (SELECT variants+observations FROM payload_offsets)+p.rowid END AS payload_id
FROM source_occurrences p LEFT JOIN observation_variants v ON v.id=p.variant_id;
CREATE UNIQUE INDEX occurrence_payload_map_row ON occurrence_payload_map(old_rowid);
INSERT INTO original_payloads
SELECT m.payload_id,p.raw_json FROM source_occurrences p
JOIN occurrence_payload_map m ON m.old_rowid=p.rowid
WHERE m.payload_id>(SELECT variants+observations FROM payload_offsets);

CREATE TABLE observations_compact (
    id TEXT NOT NULL UNIQUE, source TEXT NOT NULL, source_key TEXT NOT NULL,
    identity_version INTEGER NOT NULL DEFAULT 1,
    listen_id TEXT NOT NULL REFERENCES listens(id), source_timestamp_ms INTEGER NOT NULL,
    payload_id INTEGER NOT NULL REFERENCES original_payloads(id),
    PRIMARY KEY(source,source_key), UNIQUE(source,listen_id)
);
INSERT INTO observations_compact
SELECT o.id,o.source,o.source_key,o.identity_version,o.listen_id,o.source_timestamp_ms,m.payload_id
FROM observations o JOIN observation_payload_map m ON m.old_rowid=o.rowid;
CREATE TABLE observation_variants_compact (
    id TEXT PRIMARY KEY, observation_id TEXT NOT NULL REFERENCES observations_compact(id),
    fingerprint TEXT NOT NULL, payload_id INTEGER NOT NULL REFERENCES original_payloads(id),
    played_at_ms INTEGER NOT NULL, listened_ms INTEGER, duration_kind TEXT NOT NULL,
    counted INTEGER NOT NULL, policy_version INTEGER NOT NULL,
    UNIQUE(observation_id,fingerprint)
);
INSERT INTO observation_variants_compact
SELECT id,observation_id,fingerprint,rowid,played_at_ms,listened_ms,duration_kind,counted,policy_version
FROM observation_variants;
CREATE TABLE source_occurrences_compact (
    job_id TEXT NOT NULL REFERENCES jobs(id), file_id TEXT REFERENCES source_files(id),
    member TEXT NOT NULL, row_number INTEGER NOT NULL CHECK(row_number>0),
    variant_id TEXT REFERENCES observation_variants_compact(id),
    payload_id INTEGER NOT NULL REFERENCES original_payloads(id), outcome TEXT NOT NULL, reason TEXT,
    PRIMARY KEY(job_id,member,row_number)
);
INSERT INTO source_occurrences_compact
SELECT p.job_id,p.file_id,p.member,p.row_number,p.variant_id,m.payload_id,p.outcome,p.reason
FROM source_occurrences p JOIN occurrence_payload_map m ON m.old_rowid=p.rowid;
DROP TABLE source_occurrences;
DROP TABLE observation_variants;
DROP TABLE observations;
ALTER TABLE observations_compact RENAME TO observations;
ALTER TABLE observation_variants_compact RENAME TO observation_variants;
ALTER TABLE source_occurrences_compact RENAME TO source_occurrences;
CREATE INDEX observations_listen ON observations(listen_id);
CREATE INDEX observations_source_time ON observations(source,source_timestamp_ms,listen_id);
CREATE INDEX variants_observation ON observation_variants(observation_id,duration_kind,played_at_ms);
CREATE INDEX occurrences_variant ON source_occurrences(variant_id);
CREATE INDEX occurrences_file ON source_occurrences(file_id,member,row_number);
DROP TABLE observation_payload_map;
DROP TABLE occurrence_payload_map;
DROP TABLE payload_offsets;
-- VACUUM cannot run inside this migration transaction. Startup/trial restoration
-- retries compaction outside it, including after a crash following this commit.
INSERT INTO settings VALUES('original_payload_compaction_pending','1');
