-- Earlier versions cached transient provider errors as absent images for a day.
-- Discard those negative entries so they can be checked again after backoff.
DELETE FROM artwork WHERE source='spotify' AND bytes IS NULL;
