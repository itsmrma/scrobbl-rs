-- The owner chose normalized history and fingerprints over raw JSON retention.
-- Startup hashes legacy payloads in bounded transactions before opening readers,
-- then VACUUMs. JSON null explicitly represents unavailable original bytes.
ALTER TABLE original_payloads ADD COLUMN fingerprint TEXT;
ALTER TABLE source_files ADD COLUMN retained INTEGER NOT NULL DEFAULT 1 CHECK(retained IN (0,1));
INSERT OR REPLACE INTO settings VALUES('compact_retention_pending','1');
INSERT OR REPLACE INTO settings VALUES('original_payload_compaction_pending','1');
