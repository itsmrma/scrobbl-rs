ALTER TABLE api_keys ADD COLUMN id TEXT;
ALTER TABLE api_keys ADD COLUMN application TEXT NOT NULL DEFAULT 'Other application';
ALTER TABLE api_keys ADD COLUMN created_at_ms INTEGER NOT NULL DEFAULT 0;
ALTER TABLE api_keys ADD COLUMN last_used_at_ms INTEGER;
UPDATE api_keys SET id=lower(hex(randomblob(32)));
CREATE UNIQUE INDEX api_keys_id ON api_keys(id);
CREATE TABLE api_cursors (
    token TEXT PRIMARY KEY, key_digest TEXT NOT NULL REFERENCES api_keys(digest),
    value TEXT NOT NULL, expires_at_ms INTEGER NOT NULL
);
CREATE INDEX api_cursor_expiry ON api_cursors(expires_at_ms);
CREATE TABLE api_rate_limits (
    key_digest TEXT PRIMARY KEY REFERENCES api_keys(digest),
    window_ms INTEGER NOT NULL, requests INTEGER NOT NULL
);
