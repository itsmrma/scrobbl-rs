ALTER TABLE jobs ADD COLUMN phase TEXT NOT NULL DEFAULT 'processing';
ALTER TABLE jobs ADD COLUMN outcome TEXT;
ALTER TABLE jobs ADD COLUMN inventory_complete INTEGER NOT NULL DEFAULT 0;
ALTER TABLE jobs ADD COLUMN below_threshold INTEGER NOT NULL DEFAULT 0;
CREATE TABLE job_inputs (
    job_id TEXT NOT NULL REFERENCES jobs(id), ordinal INTEGER NOT NULL,
    file_id TEXT NOT NULL REFERENCES source_files(id), filename TEXT NOT NULL,
    PRIMARY KEY(job_id,ordinal)
);
ALTER TABLE job_members ADD COLUMN file_id TEXT REFERENCES source_files(id);
ALTER TABLE job_members ADD COLUMN originals INTEGER NOT NULL DEFAULT 0;
ALTER TABLE job_members ADD COLUMN variants INTEGER NOT NULL DEFAULT 0;
ALTER TABLE job_members ADD COLUMN duplicates INTEGER NOT NULL DEFAULT 0;
ALTER TABLE job_members ADD COLUMN added INTEGER NOT NULL DEFAULT 0;
ALTER TABLE job_members ADD COLUMN reconciled INTEGER NOT NULL DEFAULT 0;
ALTER TABLE job_members ADD COLUMN invalid INTEGER NOT NULL DEFAULT 0;
ALTER TABLE job_members ADD COLUMN skipped INTEGER NOT NULL DEFAULT 0;
ALTER TABLE job_members ADD COLUMN below_threshold INTEGER NOT NULL DEFAULT 0;
CREATE INDEX observations_source_time ON observations(source,source_timestamp_ms,listen_id);
