-- Retire the Spotify image fallback without deleting saved images or API waits.
UPDATE settings
SET value = json_set(value,
    '$.providers', json((
        SELECT json_group_array(json(p.value))
        FROM json_each(settings.value, '$.providers') AS p
        WHERE json_extract(p.value, '$.provider') != 'spotify'
    )),
    '$.version', json_extract(value, '$.version') + 1)
WHERE key = 'artwork_providers'
  AND EXISTS (
      SELECT 1 FROM json_each(settings.value, '$.providers') AS p
      WHERE json_extract(p.value, '$.provider') = 'spotify'
  );

-- Reconsider pending work under public sources. Their host waits still apply.
-- A user-paused activity remains paused; counters and saved bytes are retained.
UPDATE job_members
SET artwork_retry_at_ms = 0, artwork_attempts = 0,
    error_code = NULL, current_provider = NULL
WHERE state = 'pending'
  AND error_code IN ('artwork_connection_required', 'artwork_provider_wait')
  AND job_id IN (SELECT id FROM jobs WHERE kind = 'artwork');

UPDATE jobs
SET retry_at_ms = 0, error_code = NULL
WHERE kind = 'artwork' AND state = 'waiting'
  AND error_code IN ('artwork_connection_required', 'artwork_provider_wait');
