-- Cover Overview aggregation and assigned-calendar selection without fetching each
-- wide listen row or joining music metadata before grouping.
CREATE INDEX listens_overview ON listens(
    track_id,counted,duration_kind,listened_ms,local_date,calendar_id,played_at_ms,needs_review
) WHERE trashed_at_ms IS NULL;
