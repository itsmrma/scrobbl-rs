-- Provider artwork is a bounded, expiring cache. Owner images have no expiry.
CREATE TABLE artwork (
    kind TEXT NOT NULL CHECK(kind IN ('artists','albums')),
    entity_id TEXT NOT NULL,
    source TEXT NOT NULL CHECK(source IN ('spotify','custom')),
    bytes BLOB,
    mime TEXT CHECK(mime IN ('image/png','image/jpeg','image/webp')),
    checked_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER,
    PRIMARY KEY(kind,entity_id,source)
);
CREATE INDEX artwork_expiry ON artwork(source,expires_at_ms);
