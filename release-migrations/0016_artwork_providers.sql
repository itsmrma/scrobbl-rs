-- Preserve existing images byte-for-byte; only widen the allowed provenance.
ALTER TABLE artwork RENAME TO artwork_previous;
DROP INDEX artwork_expiry;
CREATE TABLE artwork (
    kind TEXT NOT NULL CHECK(kind IN ('artists','albums')),
    entity_id TEXT NOT NULL,
    source TEXT NOT NULL CHECK(source IN ('spotify','custom','cover_art_archive','musicbrainz_image','wikidata')),
    bytes BLOB,
    mime TEXT CHECK(mime IN ('image/png','image/jpeg','image/webp')),
    checked_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER,
    provenance_json TEXT,
    PRIMARY KEY(kind,entity_id,source)
);
INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms,expires_at_ms)
    SELECT kind,entity_id,source,bytes,mime,checked_at_ms,expires_at_ms FROM artwork_previous;
DROP TABLE artwork_previous;
CREATE INDEX artwork_expiry ON artwork(source,expires_at_ms);
CREATE TABLE artwork_misses (
    kind TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    settings_version INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL,
    PRIMARY KEY(kind,entity_id)
);
