-- Release format. The prototype migrations remain unchanged and are never run here.
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE archive_state (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    instance_id TEXT NOT NULL, generation TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0, retained_from INTEGER NOT NULL DEFAULT 0,
    maintenance INTEGER NOT NULL DEFAULT 0 CHECK(maintenance IN (0,1))
);
INSERT INTO archive_state(singleton,instance_id,generation) VALUES(1,lower(hex(randomblob(16))),lower(hex(randomblob(16))));
CREATE TABLE calendars (
    id TEXT PRIMARY KEY, timezone TEXT NOT NULL,
    week_start INTEGER NOT NULL CHECK(week_start BETWEEN 0 AND 6),
    rules_version INTEGER NOT NULL
);
INSERT INTO calendars VALUES('initial','Europe/Rome',0,1);
INSERT INTO settings VALUES('calendar_id','initial');
CREATE TABLE albums (id TEXT PRIMARY KEY, name TEXT NOT NULL, cover_url TEXT, metadata_rank TEXT NOT NULL);
CREATE TABLE artists (id TEXT PRIMARY KEY, name TEXT NOT NULL, metadata_rank TEXT NOT NULL);
CREATE TABLE tracks (
    id TEXT PRIMARY KEY, title TEXT NOT NULL, album_id TEXT REFERENCES albums(id),
    duration_ms INTEGER CHECK(duration_ms>=0), spotify_url TEXT,
    structured_metadata INTEGER NOT NULL DEFAULT 0, metadata_rank TEXT NOT NULL
);
CREATE TABLE track_artists (
    track_id TEXT NOT NULL REFERENCES tracks(id), artist_id TEXT NOT NULL REFERENCES artists(id),
    ordinal INTEGER NOT NULL, PRIMARY KEY(track_id,artist_id)
);
CREATE INDEX track_artists_artist ON track_artists(artist_id,track_id);
CREATE INDEX tracks_album ON tracks(album_id);
-- External evidence and provisional lookup keys do not become internal identities.
CREATE TABLE entity_links (
    kind TEXT NOT NULL CHECK(kind IN ('track','album','artist')),
    namespace TEXT NOT NULL, external_id TEXT NOT NULL, entity_id TEXT NOT NULL,
    provisional INTEGER NOT NULL CHECK(provisional IN (0,1)), evidence_json TEXT NOT NULL,
    PRIMARY KEY(kind,namespace,external_id)
);
CREATE INDEX entity_links_entity ON entity_links(kind,entity_id);
CREATE TABLE listens (
    id TEXT PRIMARY KEY, track_id TEXT NOT NULL REFERENCES tracks(id),
    played_at_ms INTEGER NOT NULL, first_acquired_at_ms INTEGER NOT NULL,
    calendar_id TEXT NOT NULL REFERENCES calendars(id),
    local_date TEXT NOT NULL, week_date TEXT NOT NULL,
    policy_version INTEGER NOT NULL DEFAULT 1,
    anchor_source TEXT NOT NULL,
    listened_ms INTEGER CHECK(listened_ms>=0),
    duration_kind TEXT NOT NULL CHECK(duration_kind IN ('actual','estimated','unknown')),
    counted INTEGER NOT NULL CHECK(counted IN (0,1)),
    needs_review INTEGER NOT NULL DEFAULT 0 CHECK(needs_review IN (0,1)),
    trashed_at_ms INTEGER, revision INTEGER NOT NULL DEFAULT 0,
    CHECK((duration_kind='unknown' AND listened_ms IS NULL) OR (duration_kind!='unknown' AND listened_ms IS NOT NULL))
);
CREATE INDEX listens_date ON listens(played_at_ms DESC,id DESC);
CREATE INDEX listens_track_date ON listens(track_id,played_at_ms);
CREATE INDEX listens_calendar_date ON listens(local_date,calendar_id,played_at_ms,id);
CREATE INDEX listens_week ON listens(week_date,calendar_id);
CREATE INDEX listens_review ON listens(needs_review,played_at_ms DESC);
CREATE TABLE observations (
    id TEXT NOT NULL UNIQUE, source TEXT NOT NULL, source_key TEXT NOT NULL,
    identity_version INTEGER NOT NULL DEFAULT 1,
    listen_id TEXT NOT NULL REFERENCES listens(id), source_timestamp_ms INTEGER NOT NULL,
    raw_json TEXT NOT NULL, PRIMARY KEY(source,source_key), UNIQUE(source,listen_id)
);
CREATE INDEX observations_listen ON observations(listen_id);
CREATE TABLE observation_variants (
    id TEXT PRIMARY KEY, observation_id TEXT NOT NULL REFERENCES observations(id),
    fingerprint TEXT NOT NULL, raw_json TEXT NOT NULL,
    played_at_ms INTEGER NOT NULL, listened_ms INTEGER, duration_kind TEXT NOT NULL,
    counted INTEGER NOT NULL, policy_version INTEGER NOT NULL,
    UNIQUE(observation_id,fingerprint)
);
CREATE TABLE source_files (
    id TEXT PRIMARY KEY, content_hash TEXT NOT NULL UNIQUE, size_bytes INTEGER NOT NULL CHECK(size_bytes>=0),
    received_at_ms INTEGER NOT NULL
);
CREATE TABLE jobs (
    id TEXT PRIMARY KEY, kind TEXT NOT NULL, file_id TEXT REFERENCES source_files(id),
    filename TEXT, parameters_json TEXT NOT NULL DEFAULT '{}',
    state TEXT NOT NULL CHECK(state IN ('queued','running','waiting','completed','failed','cancelled')),
    created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 0, checkpoint INTEGER NOT NULL DEFAULT 0 CHECK(checkpoint>=0),
    processed INTEGER NOT NULL DEFAULT 0, added INTEGER NOT NULL DEFAULT 0,
    reconciled INTEGER NOT NULL DEFAULT 0, duplicates INTEGER NOT NULL DEFAULT 0,
    originals INTEGER NOT NULL DEFAULT 0, variants INTEGER NOT NULL DEFAULT 0,
    invalid INTEGER NOT NULL DEFAULT 0, skipped INTEGER NOT NULL DEFAULT 0,
    ambiguous INTEGER NOT NULL DEFAULT 0,
    cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK(cancel_requested IN (0,1)),
    retry_at_ms INTEGER, error_code TEXT
);
CREATE INDEX jobs_frontier ON jobs(kind,state,retry_at_ms,created_at_ms);
CREATE TABLE job_members (
    job_id TEXT NOT NULL REFERENCES jobs(id), member TEXT NOT NULL,
    kind TEXT NOT NULL, state TEXT NOT NULL,
    expected_rows INTEGER, processed INTEGER NOT NULL DEFAULT 0,
    error_code TEXT, PRIMARY KEY(job_id,member)
);
CREATE TABLE source_occurrences (
    job_id TEXT NOT NULL REFERENCES jobs(id), file_id TEXT REFERENCES source_files(id),
    member TEXT NOT NULL, row_number INTEGER NOT NULL CHECK(row_number>0),
    variant_id TEXT REFERENCES observation_variants(id), raw_json TEXT NOT NULL,
    outcome TEXT NOT NULL, reason TEXT,
    PRIMARY KEY(job_id,member,row_number)
);
CREATE INDEX occurrences_variant ON source_occurrences(variant_id);
CREATE TABLE conflicts (
    id TEXT PRIMARY KEY, listen_id TEXT NOT NULL REFERENCES listens(id),
    kind TEXT NOT NULL, evidence_json TEXT NOT NULL, resolved_at_ms INTEGER,
    UNIQUE(listen_id,kind)
);
CREATE TABLE local_corrections (
    entity_kind TEXT NOT NULL, entity_id TEXT NOT NULL, field TEXT NOT NULL,
    value_json TEXT NOT NULL, PRIMARY KEY(entity_kind,entity_id,field)
);
CREATE TABLE entity_groups (id TEXT PRIMARY KEY, kind TEXT NOT NULL CHECK(kind IN ('track','album')));
CREATE TABLE group_members (
    group_id TEXT NOT NULL REFERENCES entity_groups(id), entity_id TEXT NOT NULL,
    PRIMARY KEY(group_id,entity_id)
);
CREATE TABLE identity_decisions (
    id TEXT PRIMARY KEY, kind TEXT NOT NULL, from_id TEXT NOT NULL, to_id TEXT NOT NULL,
    evidence_json TEXT NOT NULL, created_at_ms INTEGER NOT NULL
);
CREATE TABLE changes (
    revision INTEGER NOT NULL, listen_id TEXT NOT NULL,
    operation TEXT NOT NULL CHECK(operation IN ('upsert','remove')),
    replacement_id TEXT, PRIMARY KEY(revision,listen_id)
);
CREATE INDEX changes_listen ON changes(listen_id,revision);
CREATE TABLE calendar_staging (
    job_id TEXT NOT NULL REFERENCES jobs(id), listen_id TEXT NOT NULL REFERENCES listens(id),
    local_date TEXT NOT NULL, week_date TEXT NOT NULL,
    PRIMARY KEY(job_id,listen_id)
);
CREATE TABLE import_jobs (
    id INTEGER PRIMARY KEY, filename TEXT NOT NULL, created_at_ms INTEGER NOT NULL, report_json TEXT NOT NULL
);
