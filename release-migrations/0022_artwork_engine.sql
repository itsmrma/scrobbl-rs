-- Rewritten image engine: Deezer and Wikimedia become sources, pacing moves
-- in memory, and every absence is reconsidered by the faster providers.
ALTER TABLE artwork RENAME TO artwork_previous;
DROP INDEX IF EXISTS artwork_expiry;
CREATE TABLE artwork (
    kind TEXT NOT NULL CHECK(kind IN ('artists','albums')),
    entity_id TEXT NOT NULL,
    source TEXT NOT NULL CHECK(source IN ('spotify','custom','cover_art_archive','musicbrainz_image','wikidata','itunes','deezer','wikimedia')),
    bytes BLOB,
    mime TEXT CHECK(mime IN ('image/png','image/jpeg','image/webp')),
    checked_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER,
    provenance_json TEXT,
    PRIMARY KEY(kind,entity_id,source)
);
INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms,expires_at_ms,provenance_json)
    SELECT kind,entity_id,source,bytes,mime,checked_at_ms,expires_at_ms,provenance_json FROM artwork_previous;
DROP TABLE artwork_previous;
CREATE INDEX artwork_expiry ON artwork(source,expires_at_ms);

-- Rows without bytes were placeholders of retired flows; they only hid work.
DELETE FROM artwork WHERE bytes IS NULL AND source != 'custom';
DELETE FROM artwork_misses;
DELETE FROM settings WHERE key LIKE 'artwork_host_retry:%'
    OR key LIKE 'artwork_host_failures:%'
    OR key IN ('musicbrainz_next_at_ms','artwork_background_next_at_ms','artwork_retry_at_ms');
UPDATE job_members SET artwork_retry_at_ms=0, artwork_attempts=0, error_code=NULL, current_provider=NULL
    WHERE state='pending' AND job_id IN (SELECT id FROM jobs WHERE kind='artwork');
UPDATE jobs SET retry_at_ms=0, error_code=NULL
    WHERE kind='artwork' AND state='waiting' AND COALESCE(error_code,'') != 'artwork_paused';
