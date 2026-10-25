-- Per-item waits keep one unavailable fallback from blocking the library.
ALTER TABLE job_members ADD COLUMN artwork_retry_at_ms INTEGER NOT NULL DEFAULT 0;
ALTER TABLE job_members ADD COLUMN artwork_attempts INTEGER NOT NULL DEFAULT 0;
CREATE INDEX artwork_member_frontier ON job_members(job_id,state,artwork_retry_at_ms,member);
-- Retire the old minute-per-image reservation, retaining actual provider limits.
DELETE FROM settings WHERE key='artwork_background_next_at_ms';
UPDATE jobs SET retry_at_ms=0 WHERE kind='artwork' AND state='waiting' AND error_code='artwork_pacing';
