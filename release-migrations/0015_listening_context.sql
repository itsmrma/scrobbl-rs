-- Selected non-identifying Spotify export fields, independent of raw payload retention.
ALTER TABLE observation_variants ADD COLUMN context_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(context_json) AND json_type(context_json)='object');
