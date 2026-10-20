CREATE TABLE owner (id INTEGER PRIMARY KEY CHECK(id=1), password_hash TEXT NOT NULL, created_at_ms INTEGER NOT NULL);
CREATE TABLE sessions (digest TEXT PRIMARY KEY, csrf TEXT NOT NULL, expires_at_ms INTEGER NOT NULL);
CREATE TABLE oauth_states (digest TEXT PRIMARY KEY, session_digest TEXT NOT NULL, expires_at_ms INTEGER NOT NULL);
CREATE TABLE spotify_tokens (
    id INTEGER PRIMARY KEY CHECK(id=1), access_token TEXT NOT NULL, refresh_token TEXT NOT NULL, expires_at_ms INTEGER NOT NULL
);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE api_keys (digest TEXT PRIMARY KEY, name TEXT NOT NULL, expires_at_ms INTEGER, revoked_at_ms INTEGER);
-- Intent and private changes commit together. Recovery finishes the archive phase idempotently.
CREATE TABLE access_operations (id TEXT PRIMARY KEY, kind TEXT NOT NULL, completed INTEGER NOT NULL DEFAULT 0);
