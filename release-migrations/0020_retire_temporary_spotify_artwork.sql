-- Retire temporary Spotify fallback cache rows, legacy retry setting, and unread Spotify CDN album links.
-- Personal images and legacy permanent images (expires_at_ms IS NULL AND provenance_json IS NULL) stay intact.
DELETE FROM artwork
WHERE bytes IS NULL
   OR expires_at_ms IS NOT NULL
   OR (source = 'spotify' AND provenance_json IS NOT NULL);

DELETE FROM settings WHERE key = 'artwork_retry_at_ms';

UPDATE albums SET cover_url = NULL WHERE cover_url IS NOT NULL;
