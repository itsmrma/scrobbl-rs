-- Successful provider images are kept with the local library, like owner images.
-- Missing images keep their retry time; no bytes or listening data are removed.
UPDATE artwork SET expires_at_ms=NULL WHERE source='spotify' AND bytes IS NOT NULL;
