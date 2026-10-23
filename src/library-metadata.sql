WITH artists AS (
 SELECT g.id,COALESCE((SELECT json_extract(value_json,'$') FROM local_corrections WHERE entity_kind='artist' AND entity_id=g.id AND field='name'),a.name) AS name,a.metadata_rank
 FROM name_groups g JOIN main.artists a ON a.id=g.representative_id WHERE g.kind='artists' AND EXISTS(SELECT 1 FROM name_group_members m WHERE m.group_id=g.id)
), albums AS (
 SELECT g.id,COALESCE((SELECT json_extract(value_json,'$') FROM local_corrections WHERE entity_kind='album' AND entity_id=g.id AND field='name'),a.name) AS name,'/api/v1/artwork/albums/'||g.id AS cover_url,a.metadata_rank
 FROM name_groups g JOIN main.albums a ON a.id=g.representative_id WHERE g.kind='albums' AND EXISTS(SELECT 1 FROM name_group_members m WHERE m.group_id=g.id)
), tracks AS (
 SELECT t.id,COALESCE((SELECT json_extract(value_json,'$') FROM local_corrections WHERE entity_kind='track' AND entity_id=t.id AND field='name'),t.title) AS title,COALESCE(m.group_id,t.album_id) AS album_id,t.duration_ms,t.spotify_url,t.structured_metadata,t.metadata_rank
 FROM main.tracks t LEFT JOIN name_group_members m ON m.kind='albums' AND m.entity_id=t.album_id
), track_artists AS (
 SELECT ta.track_id,m.group_id AS artist_id,MIN(ta.ordinal) AS ordinal FROM main.track_artists ta JOIN name_group_members m ON m.kind='artists' AND m.entity_id=ta.artist_id GROUP BY ta.track_id,m.group_id
)
