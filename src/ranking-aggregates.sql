-- selected contains exactly the same assigned-calendar/selection predicates as History.
-- Aggregate narrow listen values once, then join metadata for the much smaller track set.
, by_track AS MATERIALIZED (
    SELECT track_id, COUNT(*) AS records, SUM(counted) AS plays,
           COALESCE(SUM(CASE WHEN counted=1 THEN listened_ms ELSE 0 END),0) AS listened_ms,
           COALESCE(SUM(CASE WHEN counted=1 AND duration_kind='actual' THEN listened_ms ELSE 0 END),0) AS actual_ms,
           COALESCE(SUM(CASE WHEN counted=1 AND duration_kind='estimated' THEN listened_ms ELSE 0 END),0) AS estimated_ms,
           SUM(CASE WHEN counted=1 AND duration_kind='unknown' THEN 1 ELSE 0 END) AS unknown_durations
    FROM selected GROUP BY track_id
), artist_ranks AS (
    SELECT ar.id, ar.name, CASE WHEN COUNT(*)=1 THEN MIN(t.title) ELSE CAST(COUNT(*) AS TEXT) || ' tracks' END AS subtitle, NULL AS cover_url,
           SUM(b.plays) AS plays, SUM(b.listened_ms) AS listened_ms, SUM(b.actual_ms) AS actual_ms, SUM(b.estimated_ms) AS estimated_ms, SUM(b.unknown_durations) AS unknown_durations
    FROM by_track b JOIN track_artists ta ON ta.track_id=b.track_id
    JOIN artists ar ON ar.id=ta.artist_id JOIN tracks t ON t.id=b.track_id
    WHERE b.plays>0 GROUP BY ar.id
), album_ranks AS (
    SELECT a.id, a.name, NULL AS subtitle, a.cover_url,
           SUM(b.plays) AS plays, SUM(b.listened_ms) AS listened_ms, SUM(b.actual_ms) AS actual_ms, SUM(b.estimated_ms) AS estimated_ms, SUM(b.unknown_durations) AS unknown_durations
    FROM by_track b JOIN tracks t ON t.id=b.track_id JOIN albums a ON a.id=t.album_id
    WHERE b.plays>0 GROUP BY a.id
), track_ranks AS (
    SELECT t.id, t.title AS name, a.name AS subtitle, a.cover_url, b.plays, b.listened_ms, b.actual_ms, b.estimated_ms, b.unknown_durations
    FROM by_track b JOIN tracks t ON t.id=b.track_id LEFT JOIN albums a ON a.id=t.album_id
    WHERE b.plays>0
)
