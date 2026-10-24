-- Reversible display groups. Source identities, links and observations stay intact.
CREATE TABLE name_groups (
    id TEXT PRIMARY KEY, kind TEXT NOT NULL CHECK(kind IN ('artists','albums')),
    name_key TEXT NOT NULL, representative_id TEXT NOT NULL,
    UNIQUE(kind,name_key)
);
CREATE TABLE name_group_members (
    kind TEXT NOT NULL, entity_id TEXT NOT NULL,
    group_id TEXT NOT NULL REFERENCES name_groups(id),
    default_group_id TEXT NOT NULL REFERENCES name_groups(id),
    PRIMARY KEY(kind,entity_id)
);
CREATE INDEX name_group_members_group ON name_group_members(group_id,entity_id);
INSERT INTO name_groups SELECT 'name-'||lower(hex(randomblob(16))),'artists',lower(trim(name)),MIN(id) FROM artists GROUP BY lower(trim(name));
INSERT INTO name_group_members SELECT 'artists',a.id,g.id,g.id FROM artists a JOIN name_groups g ON g.kind='artists' AND g.name_key=lower(trim(a.name));
CREATE TEMP VIEW album_group_keys AS
SELECT a.id,json_array(lower(trim(a.name)),COALESCE((SELECT json_group_array(name) FROM (SELECT DISTINCT lower(trim(ar.name)) AS name FROM tracks t JOIN track_artists ta ON ta.track_id=t.id JOIN artists ar ON ar.id=ta.artist_id WHERE t.album_id=a.id ORDER BY name)),'')) AS name_key FROM albums a;
INSERT INTO name_groups SELECT 'name-'||lower(hex(randomblob(16))),'albums',name_key,MIN(id) FROM album_group_keys GROUP BY name_key;
INSERT INTO name_group_members SELECT 'albums',a.id,g.id,g.id FROM album_group_keys a JOIN name_groups g ON g.kind='albums' AND g.name_key=a.name_key;
DROP VIEW album_group_keys;
-- Preserve pre-existing personal metadata; retain every original override as well.
INSERT OR IGNORE INTO local_corrections SELECT c.entity_kind,m.group_id,c.field,c.value_json FROM local_corrections c JOIN name_group_members m ON m.entity_id=c.entity_id AND m.kind=c.entity_kind||'s' ORDER BY c.entity_id;
INSERT OR IGNORE INTO artwork SELECT a.kind,m.group_id,a.source,a.bytes,a.mime,a.checked_at_ms,a.expires_at_ms FROM artwork a JOIN name_group_members m ON m.kind=a.kind AND m.entity_id=a.entity_id ORDER BY a.checked_at_ms DESC;
-- Existing API clients must obtain a fresh coherent copy with the grouped IDs.
UPDATE archive_state SET generation=lower(hex(randomblob(16))),retained_from=revision;
