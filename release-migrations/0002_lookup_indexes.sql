CREATE INDEX variants_observation ON observation_variants(observation_id,duration_kind,played_at_ms);
CREATE INDEX occurrences_file ON source_occurrences(file_id,member,row_number);
CREATE INDEX listens_acquired ON listens(first_acquired_at_ms,id);
