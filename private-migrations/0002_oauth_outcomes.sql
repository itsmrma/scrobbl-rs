-- Keep a session-bound outcome so the original tab can follow its own attempt.
ALTER TABLE oauth_states ADD COLUMN outcome TEXT NOT NULL DEFAULT 'pending';
ALTER TABLE oauth_states ADD COLUMN message TEXT;
