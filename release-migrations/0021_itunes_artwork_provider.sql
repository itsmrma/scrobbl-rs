-- Allow 'itunes' as a valid source in the artwork table.
ALTER TABLE artwork RENAME TO artwork_previous;
DROP INDEX IF EXISTS artwork_expiry;
CREATE TABLE artwork (
    kind TEXT NOT NULL CHECK(kind IN ('artists','albums')),
    entity_id TEXT NOT NULL,
    source TEXT NOT NULL CHECK(source IN ('spotify','custom','cover_art_archive','musicbrainz_image','wikidata','itunes')),
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
