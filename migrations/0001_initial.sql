CREATE TABLE owner (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    password_hash TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
CREATE TABLE sessions (
    digest TEXT PRIMARY KEY,
    csrf TEXT NOT NULL,
    expires_at_ms INTEGER NOT NULL
);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE spotify_tokens (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    access_token TEXT NOT NULL,
    refresh_token TEXT NOT NULL,
    expires_at_ms INTEGER NOT NULL
);
CREATE TABLE oauth_states (
    digest TEXT PRIMARY KEY,
    session_digest TEXT NOT NULL,
    expires_at_ms INTEGER NOT NULL
);
CREATE TABLE albums (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    cover_url TEXT
);
CREATE TABLE artists (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL
);
CREATE TABLE tracks (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    album_id TEXT REFERENCES albums(id),
    duration_ms INTEGER CHECK (duration_ms >= 0),
    spotify_url TEXT,
    structured_metadata INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE track_artists (
    track_id TEXT NOT NULL REFERENCES tracks(id),
    artist_id TEXT NOT NULL REFERENCES artists(id),
    ordinal INTEGER NOT NULL,
    PRIMARY KEY (track_id, artist_id)
);
CREATE TABLE listens (
    id INTEGER PRIMARY KEY,
    track_id TEXT NOT NULL REFERENCES tracks(id),
    played_at_ms INTEGER NOT NULL,
    listened_ms INTEGER CHECK (listened_ms >= 0),
    duration_kind TEXT NOT NULL CHECK (duration_kind IN ('actual', 'estimated', 'unknown')),
    counted INTEGER NOT NULL CHECK (counted IN (0, 1))
);
CREATE INDEX listens_date ON listens(played_at_ms DESC, id DESC);
CREATE INDEX listens_track_date ON listens(track_id, played_at_ms);
CREATE TABLE observations (
    source TEXT NOT NULL CHECK (source IN ('spotify_api', 'spotify_import')),
    source_key TEXT NOT NULL,
    listen_id INTEGER NOT NULL REFERENCES listens(id),
    source_timestamp_ms INTEGER NOT NULL,
    raw_json TEXT NOT NULL,
    PRIMARY KEY (source, source_key),
    UNIQUE (source, listen_id)
);
CREATE INDEX observations_listen ON observations(listen_id);
CREATE TABLE import_jobs (
    id INTEGER PRIMARY KEY,
    filename TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    report_json TEXT NOT NULL
);
