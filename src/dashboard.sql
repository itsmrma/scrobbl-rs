SELECT json_object(
    'totals', (SELECT json_object(
        'records',COALESCE(SUM(records),0), 'listens',COALESCE(SUM(plays),0),
        'tracks',COALESCE(SUM(plays>0),0), 'actual_ms',COALESCE(SUM(actual_ms),0),
        'estimated_ms',COALESCE(SUM(estimated_ms),0), 'unknown_durations',COALESCE(SUM(unknown_durations),0)
    ) FROM by_track),
    'rankings', json_object(
        'artists', (SELECT json_group_array(json_object('id',id,'name',name,'subtitle',subtitle,'cover_url',cover_url,'plays',plays,'listened_ms',listened_ms,'actual_ms',actual_ms,'estimated_ms',estimated_ms,'unknown_durations',unknown_durations,'position',position)) FROM (SELECT *, RANK() OVER (ORDER BY /*PRIMARY*/ DESC) AS position FROM artist_ranks ORDER BY /*ORDER*/ LIMIT 5)),
        'albums', (SELECT json_group_array(json_object('id',id,'name',name,'subtitle',subtitle,'cover_url',cover_url,'plays',plays,'listened_ms',listened_ms,'actual_ms',actual_ms,'estimated_ms',estimated_ms,'unknown_durations',unknown_durations,'position',position)) FROM (SELECT *, RANK() OVER (ORDER BY /*PRIMARY*/ DESC) AS position FROM album_ranks ORDER BY /*ORDER*/ LIMIT 5)),
        'tracks', (SELECT json_group_array(json_object('id',id,'name',name,'subtitle',subtitle,'cover_url',cover_url,'plays',plays,'listened_ms',listened_ms,'actual_ms',actual_ms,'estimated_ms',estimated_ms,'unknown_durations',unknown_durations,'position',position)) FROM (SELECT *, RANK() OVER (ORDER BY /*PRIMARY*/ DESC) AS position FROM track_ranks ORDER BY /*ORDER*/ LIMIT 5))
    )
)
