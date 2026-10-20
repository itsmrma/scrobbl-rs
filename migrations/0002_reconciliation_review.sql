ALTER TABLE listens ADD COLUMN needs_review INTEGER NOT NULL DEFAULT 0 CHECK (needs_review IN (0, 1));
CREATE INDEX listens_review ON listens(needs_review, played_at_ms DESC);
