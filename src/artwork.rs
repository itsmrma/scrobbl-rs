//! Durable import enrichment. One shared catalogue slot, paced requests, saved checkpoints.
use crate::{
    AppState, auth, db,
    error::{AppError, Result},
    jobs::{self, Job},
    library::{self, ArtworkFetch},
};
use sqlx::Row;
use std::time::Duration;

// Yield between images. Actual request limits are enforced separately per source.
pub const IMAGE_INTERVAL_MS: i64 = 1_000;

async fn background_slot(state: &AppState, lease: &Job, kind: &str, id: &str) -> Result<bool> {
    let mut tx = state.db.begin().await?;
    jobs::validate_lease(&mut tx, lease).await?;
    let cached: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM artwork WHERE kind=? AND entity_id=? AND bytes IS NOT NULL)",
    )
    .bind(kind)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if cached {
        return Ok(true);
    }
    let next: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key='artwork_background_next_at_ms'")
            .fetch_optional(&mut *tx)
            .await?;
    let next = next.and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
    let now = auth::now_ms();
    if next > now {
        tx.rollback().await?;
        wait(state, lease, "artwork_pacing", next - now).await?;
        return Ok(false);
    }
    sqlx::query("INSERT INTO settings(key,value) VALUES('artwork_background_next_at_ms',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
        .bind(now.saturating_add(IMAGE_INTERVAL_MS).to_string()).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(true)
}

/// Called inside the import's final transaction. Multiple imports share a queue.
pub(crate) async fn enqueue_missing(conn: &mut sqlx::SqliteConnection) -> Result<()> {
    let pending: Option<String> = sqlx::query_scalar("SELECT id FROM jobs WHERE kind='artwork' AND state IN ('queued','running','waiting') ORDER BY created_at_ms,id LIMIT 1").fetch_optional(&mut *conn).await?;
    let id = pending.clone().unwrap_or_else(db::new_id);
    if pending.is_none() {
        sqlx::query("INSERT INTO jobs(id,kind,filename,state,created_at_ms,updated_at_ms,inventory_complete,phase) VALUES(?,'artwork','Download library artwork','queued',?,?,1,'artwork')").bind(&id).bind(auth::now_ms()).bind(auth::now_ms()).execute(&mut *conn).await?;
    }
    // Group identities avoid repeated downloads for every track/edition. A
    // valid negative cache is also a completed lookup, never a reason to retry.
    let inserted = sqlx::query(r#"INSERT INTO job_members(job_id,member,kind,state,expected_rows)
      SELECT ?,g.kind||':'||g.id,g.kind,'pending',1 FROM name_groups g
      WHERE g.kind IN ('artists','albums')
        AND ((g.kind='albums' AND EXISTS(SELECT 1 FROM name_group_members gm JOIN tracks t ON t.album_id=gm.entity_id JOIN listens l ON l.track_id=t.id WHERE gm.kind='albums' AND gm.group_id=g.id AND l.trashed_at_ms IS NULL))
          OR (g.kind='artists' AND EXISTS(SELECT 1 FROM name_group_members gm JOIN track_artists ta ON ta.artist_id=gm.entity_id JOIN listens l ON l.track_id=ta.track_id WHERE gm.kind='artists' AND gm.group_id=g.id AND l.trashed_at_ms IS NULL)))
        AND NOT EXISTS(SELECT 1 FROM artwork_misses m WHERE m.kind=g.kind AND m.entity_id=g.id AND m.expires_at_ms>?)
        AND NOT EXISTS(SELECT 1 FROM artwork a WHERE a.kind=g.kind AND a.entity_id=g.id AND a.bytes IS NOT NULL)
      ON CONFLICT(job_id,member) DO NOTHING"#).bind(&id).bind(auth::now_ms()).execute(&mut *conn).await?.rows_affected();
    if inserted > 0 {
        sqlx::query("UPDATE jobs SET retry_at_ms=0 WHERE id=? AND state='waiting' AND COALESCE(error_code,'')!='artwork_paused'")
            .bind(&id).execute(&mut *conn).await?;
    }
    if pending.is_none() {
        sqlx::query(
            "DELETE FROM jobs WHERE id=? AND NOT EXISTS(SELECT 1 FROM job_members WHERE job_id=?)",
        )
        .bind(&id)
        .bind(&id)
        .execute(conn)
        .await?;
    }
    Ok(())
}

async fn wait(state: &AppState, lease: &Job, code: &str, delay: i64) -> Result<()> {
    state
        .storage
        .pause(lease, Some(auth::now_ms().saturating_add(delay)), code)
        .await
}

async fn defer(
    state: &AppState,
    lease: &Job,
    member: &str,
    code: &str,
    at: Option<i64>,
) -> Result<()> {
    let mut tx = state.db.begin().await?;
    jobs::validate_lease(&mut tx, lease).await?;
    let attempts: i64 =
        sqlx::query_scalar("SELECT artwork_attempts FROM job_members WHERE job_id=? AND member=?")
            .bind(&lease.id)
            .bind(member)
            .fetch_one(&mut *tx)
            .await?;
    let delay = crate::artwork_sources::retry_backoff_ms(attempts);
    let retry = at.unwrap_or_else(|| auth::now_ms().saturating_add(delay));
    sqlx::query("UPDATE job_members SET artwork_retry_at_ms=?,artwork_attempts=artwork_attempts+1,error_code=? WHERE job_id=? AND member=?")
        .bind(retry).bind(code).bind(&lease.id).bind(member).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

/// Process one bounded item. Cancellation and maintenance fence every checkpoint.
pub async fn step(state: &AppState, lease: &Job) -> Result<bool> {
    step_with(state, lease, &crate::artwork_sources::PublicSources(state)).await
}

async fn step_with(
    state: &AppState,
    lease: &Job,
    sources: &impl crate::artwork_sources::ImageLookup,
) -> Result<bool> {
    let mut reservation = state.db.begin().await?;
    jobs::validate_lease(&mut reservation, lease).await?;
    sqlx::query("UPDATE job_members SET current_provider=NULL WHERE job_id=? AND current_provider IS NOT NULL")
        .bind(&lease.id)
        .execute(&mut *reservation)
        .await?;
    reservation.commit().await?;
    let item=sqlx::query("SELECT member,kind FROM job_members WHERE job_id=? AND state='pending' AND artwork_retry_at_ms<=? ORDER BY artwork_retry_at_ms,member LIMIT 1").bind(&lease.id).bind(auth::now_ms()).fetch_optional(&state.read_db).await?;
    let Some(item) = item else {
        let mut tx = state.db.begin().await?;
        jobs::validate_lease(&mut tx, lease).await?;
        let pending: Option<(i64, Option<String>)> = sqlx::query_as("SELECT artwork_retry_at_ms,error_code FROM job_members WHERE job_id=? AND state='pending' ORDER BY artwork_retry_at_ms,member LIMIT 1")
            .bind(&lease.id).fetch_optional(&mut *tx).await?;
        if let Some((at, code)) = pending {
            tx.rollback().await?;
            state
                .storage
                .pause(
                    lease,
                    Some(at.max(auth::now_ms() + 1000)),
                    code.as_deref().unwrap_or("artwork_provider_wait"),
                )
                .await?;
            return Ok(false);
        }
        // A concurrent import may have appended work since the read above.
        let done=sqlx::query("UPDATE jobs SET state='completed',phase='finished',outcome='complete',error_code=NULL,retry_at_ms=NULL,updated_at_ms=? WHERE id=? AND NOT EXISTS(SELECT 1 FROM job_members WHERE job_id=? AND state='pending')").bind(auth::now_ms()).bind(&lease.id).bind(&lease.id).execute(&mut *tx).await?.rows_affected()==1;
        tx.commit().await?;
        return Ok(!done);
    };
    let member: String = item.get("member");
    let kind: String = item.get("kind");
    let id = member
        .strip_prefix(&format!("{kind}:"))
        .ok_or(AppError::Internal)?;
    let id = crate::name_groups::canonical(&mut *state.read_db.acquire().await?, &kind, id).await;
    let result = match id {
        Ok(id) => {
            if !background_slot(state, lease, &kind, &id).await? {
                return Ok(false);
            }
            library::ensure_artwork_for_activity(state, &kind, &id, sources, &lease.id, &member)
                .await
        }
        Err(AppError::NotFound) => Ok(ArtworkFetch::Missing),
        Err(error) => Err(error),
    };
    let result = match result {
        Ok(ArtworkFetch::Busy) => {
            wait(state, lease, "artwork_capacity_wait", 1000).await?;
            return Ok(false);
        }
        Err(AppError::ArtworkWait { retry_at_ms }) => {
            defer(
                state,
                lease,
                &member,
                "artwork_provider_wait",
                Some(retry_at_ms),
            )
            .await?;
            return Ok(true);
        }
        Err(AppError::Database(error)) => return Err(AppError::Database(error)),
        Err(AppError::Conflict(error)) => return Err(AppError::Conflict(error)),
        Err(_) => {
            defer(state, lease, &member, "artwork_provider_wait", None).await?;
            return Ok(true);
        }
        Ok(result) => result,
    };
    let mut tx = state.db.begin().await?;
    jobs::validate_lease(&mut tx, lease).await?;
    sqlx::query("UPDATE job_members SET state='completed',artwork_retry_at_ms=0,error_code=NULL,processed=1,added=?,duplicates=?,skipped=? WHERE job_id=? AND member=? AND state='pending'").bind(i64::from(result==ArtworkFetch::Saved)).bind(i64::from(result==ArtworkFetch::Cached)).bind(i64::from(result==ArtworkFetch::Missing)).bind(&lease.id).bind(member).execute(&mut *tx).await?;
    sqlx::query("UPDATE jobs SET checkpoint=checkpoint+1,processed=processed+1,added=added+?,duplicates=duplicates+?,skipped=skipped+?,phase='artwork',error_code=NULL,retry_at_ms=NULL,updated_at_ms=? WHERE id=?").bind(i64::from(result==ArtworkFetch::Saved)).bind(i64::from(result==ArtworkFetch::Cached)).bind(i64::from(result==ArtworkFetch::Missing)).bind(auth::now_ms()).bind(&lease.id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(true)
}

pub async fn worker(state: AppState) {
    loop {
        if let Ok(Some(mut lease)) = state.storage.claim("artwork").await {
            loop {
                match step(&state, &lease).await {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(_) => {
                        let _=sqlx::query("UPDATE jobs SET state='queued',updated_at_ms=? WHERE id=? AND attempt=? AND state='running' AND cancel_requested=0").bind(auth::now_ms()).bind(&lease.id).bind(lease.attempt).execute(&state.db).await;
                        break;
                    }
                }
                // One download at a time; public providers retain their own
                // durable request pace and Retry-After.
                tokio::time::sleep(Duration::from_secs(1)).await;
                match state.storage.job(&lease.id).await {
                    Ok(current) if current.state == "running" => lease = current,
                    _ => break,
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        db::{DurationKind, Event},
        storage,
    };
    use serde_json::json;
    async fn setup() -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let store = storage::open(dir.path()).await.unwrap();
        let config = Config {
            bind_ip: "127.0.0.1".parse().unwrap(),
            external_origin: None,
            port: 3210,
            data_dir: dir.path().into(),
            sync_seconds: 120,
            now_playing_seconds: 30,
            import_limits: Default::default(),
            api_snapshot_limits: Default::default(),
            env_client_id: None,
            env_client_secret: None,
        };
        let state = AppState::new(store, config);
        let raw = json!({"uri":"spotify:track:0000000000000000000001","name":"Song","album":{"id":"0000000000000000000002","name":"Album","images":[]},"artists":[{"id":"0000000000000000000003","name":"Artist"}]});
        let event = Event {
            source: "spotify_api",
            source_key: "one".into(),
            source_timestamp_ms: 1000000,
            played_at_ms: 1000000,
            listened_ms: Some(180000),
            duration_kind: DurationKind::Estimated,
            counted: true,
            track: crate::spotify::parse_track(&raw).unwrap(),
            raw,
        };
        let mut tx = state.db.begin().await.unwrap();
        db::ingest(&mut tx, &[event]).await.unwrap();
        tx.commit().await.unwrap();
        (dir, state)
    }
    async fn enqueue(state: &AppState) {
        let mut tx = state.db.begin().await.unwrap();
        enqueue_missing(&mut tx).await.unwrap();
        tx.commit().await.unwrap();
    }
    async fn cache_all(state: &AppState) {
        sqlx::query("INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms) SELECT kind,id,'spotify',x'89504E470D0A1A0A','image/png',0 FROM name_groups WHERE kind IN ('artists','albums')").execute(&state.db).await.unwrap();
    }
    #[tokio::test]
    #[ignore = "contacts public artwork services; run explicitly, never uses the personal archive"]
    async fn public_artwork_smoke_downloads_album_and_artist_without_spotify() {
        let (dir, state) = setup().await;
        sqlx::query("UPDATE artists SET name='The Beatles'")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("UPDATE albums SET name='Abbey Road'")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO entity_links(kind,namespace,external_id,entity_id,provisional,evidence_json) SELECT 'artist','musicbrainz','b10bbbfc-cf9e-42e0-be17-e2c3e1d2600d',id,0,'{}' FROM artists").execute(&state.db).await.unwrap();
        for kind in ["albums", "artists"] {
            let id: String = sqlx::query_scalar("SELECT id FROM name_groups WHERE kind=?")
                .bind(kind)
                .fetch_one(&state.db)
                .await
                .unwrap();
            let result = library::ensure_artwork(&state, kind, &id).await;
            eprintln!("public smoke {kind}: {result:?}");
            assert_eq!(result.unwrap(), ArtworkFetch::Saved);
            let (source, bytes): (String, Vec<u8>) =
                sqlx::query_as("SELECT source,bytes FROM artwork WHERE kind=?")
                    .bind(kind)
                    .fetch_one(&state.db)
                    .await
                    .unwrap();
            eprintln!("public smoke {kind}: {source}, {} bytes", bytes.len());
            assert_ne!(source, "spotify");
            assert!(bytes.len() > 1000);
        }
        let config = state.config.clone();
        state.storage.close().await;
        drop(state);
        let state = AppState::new(storage::open(dir.path()).await.unwrap(), config);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM artwork WHERE bytes IS NOT NULL AND expires_at_ms IS NULL"
            )
            .fetch_one(&state.db)
            .await
            .unwrap(),
            2
        );
        state.storage.close().await;
    }
    struct Sources {
        calls: std::sync::Mutex<Vec<(crate::artwork_settings::ImageProvider, String)>>,
        album_outage: std::sync::atomic::AtomicBool,
    }
    impl crate::artwork_sources::ImageLookup for Sources {
        async fn lookup(
            &self,
            provider: crate::artwork_settings::ImageProvider,
            kind: &str,
            _: &str,
        ) -> Result<crate::artwork_sources::Lookup> {
            use crate::artwork_sources::{FoundImage, ImageCredit, Lookup};
            self.calls.lock().unwrap().push((provider, kind.into()));
            if kind == "albums" && self.album_outage.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(AppError::ArtworkWait {
                    retry_at_ms: auth::now_ms() + 120_000,
                });
            }
            if kind == "albums" {
                return Ok(Lookup::Missing);
            }
            Ok(Lookup::Found(FoundImage {
                bytes: b"\x89PNG\r\n\x1a\npublic".to_vec(),
                credit: ImageCredit {
                    provider: provider.key().into(),
                    source_url: "https://commons.wikimedia.org/wiki/File:Test.png".into(),
                    image_url: "https://upload.wikimedia.org/test.png".into(),
                    author: Some("Author".into()),
                    license: Some("CC0".into()),
                    license_url: None,
                    credit: None,
                },
            }))
        }
    }
    #[tokio::test]
    async fn confirmed_missing_album_completes_without_a_spotify_connection() {
        let (_dir, state) = setup().await;
        enqueue(&state).await;
        let sources = Sources {
            calls: Default::default(),
            album_outage: Default::default(),
        };
        let mut lease = state.storage.claim("artwork").await.unwrap().unwrap();
        while step_with(&state, &lease, &sources).await.unwrap() {
            lease = state.storage.job(&lease.id).await.unwrap();
            sqlx::query("DELETE FROM settings WHERE key='artwork_background_next_at_ms'")
                .execute(&state.db)
                .await
                .unwrap();
        }
        let done = state.storage.job(&lease.id).await.unwrap();
        assert_eq!(
            (
                done.state.as_str(),
                done.processed,
                done.added,
                done.skipped
            ),
            ("completed", 2, 1, 1)
        );
        assert_eq!(sources.calls.lock().unwrap().len(), 2);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM artwork_misses")
                .fetch_one(&state.db)
                .await
                .unwrap(),
            1
        );
        assert!(
            crate::db::setting(&state.db, "retry_at_ms")
                .await
                .unwrap()
                .is_none()
        );
        state.storage.close().await;
    }
    #[tokio::test]
    async fn upgrade_retires_spotify_settings_and_waits_preserving_images_pauses_and_api_limits() {
        let (dir, state) = setup().await;
        enqueue(&state).await;
        cache_all(&state).await;
        let legacy = json!({"version":7,"providers":[
            {"provider":"wikidata","enabled":false},
            {"provider":"cover_art_archive","enabled":true},
            {"provider":"musicbrainz_image","enabled":false},
            {"provider":"spotify","enabled":true}
        ]});
        let until = auth::now_ms() + 72_000_000;
        crate::db::set_setting(
            &mut state.db.acquire().await.unwrap(),
            "artwork_providers",
            &legacy.to_string(),
        )
        .await
        .unwrap();
        crate::db::set_setting(
            &mut state.db.acquire().await.unwrap(),
            "retry_at_ms",
            &until.to_string(),
        )
        .await
        .unwrap();
        crate::db::set_setting(
            &mut state.db.acquire().await.unwrap(),
            "artwork_host_retry:musicbrainz.org",
            &until.to_string(),
        )
        .await
        .unwrap();
        sqlx::query("UPDATE jobs SET state='waiting',error_code='artwork_connection_required',retry_at_ms=? WHERE kind='artwork'").bind(until).execute(&state.db).await.unwrap();
        sqlx::query("UPDATE job_members SET error_code='artwork_connection_required',artwork_retry_at_ms=?,current_provider='spotify'").bind(until).execute(&state.db).await.unwrap();
        sqlx::query("INSERT INTO jobs(id,kind,state,created_at_ms,updated_at_ms,error_code,retry_at_ms) VALUES('user-paused','artwork','waiting',0,0,'artwork_paused',?)").bind(until).execute(&state.db).await.unwrap();
        sqlx::query("INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms) VALUES('artists','personal','custom',x'89504E470D0A1A0A','image/png',0)").execute(&state.db).await.unwrap();
        sqlx::query("INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms,expires_at_ms,provenance_json) VALUES('albums','temp-spotify','spotify',x'89504E470D0A1A0A','image/png',0,?,'{\"provider\":\"spotify\"}')").bind(until).execute(&state.db).await.unwrap();
        sqlx::query("INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms,expires_at_ms) VALUES('albums','null-spotify','spotify',NULL,NULL,0,?)").bind(until).execute(&state.db).await.unwrap();
        crate::db::set_setting(
            &mut state.db.acquire().await.unwrap(),
            "artwork_retry_at_ms",
            &until.to_string(),
        )
        .await
        .unwrap();
        // Schemas 19 and 20 only change data. Recreate the state immediately before them.
        sqlx::query("DELETE FROM _sqlx_migrations WHERE version>=19")
            .execute(&state.db)
            .await
            .unwrap();
        let config = state.config.clone();
        state.storage.close().await;
        drop(state);
        let state = AppState::new(storage::open(dir.path()).await.unwrap(), config);
        let settings = crate::artwork_settings::read(&mut state.db.acquire().await.unwrap())
            .await
            .unwrap();
        assert_eq!(settings.version, 8);
        assert_eq!(
            settings
                .providers
                .iter()
                .map(|p| (p.provider.key(), p.enabled))
                .collect::<Vec<_>>(),
            vec![
                ("wikidata", false),
                ("cover_art_archive", true),
                ("musicbrainz_image", false)
            ]
        );
        assert_eq!(
            crate::db::setting(&state.db, "retry_at_ms").await.unwrap(),
            Some(until.to_string())
        );
        assert!(
            crate::db::setting(&state.db, "artwork_retry_at_ms")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            crate::db::setting(&state.db, "artwork_host_retry:musicbrainz.org")
                .await
                .unwrap(),
            Some(until.to_string())
        );
        let paused = state.storage.job("user-paused").await.unwrap();
        assert_eq!(paused.error_code.as_deref(), Some("artwork_paused"));
        assert_eq!(paused.retry_at_ms, Some(until));
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM artwork WHERE bytes=x'89504E470D0A1A0A'"
            )
            .fetch_one(&state.db)
            .await
            .unwrap(),
            3
        );
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM job_members WHERE artwork_retry_at_ms=0 AND error_code IS NULL AND current_provider IS NULL").fetch_one(&state.db).await.unwrap(), 2);
        let mut lease = state.storage.claim("artwork").await.unwrap().unwrap();
        while step(&state, &lease).await.unwrap() {
            lease = state.storage.job(&lease.id).await.unwrap();
        }
        assert_eq!(
            state.storage.job(&lease.id).await.unwrap().state,
            "completed"
        );
        state.storage.close().await;
        let state = AppState::new(
            storage::open(dir.path()).await.unwrap(),
            state.config.clone(),
        );
        assert_eq!(
            crate::artwork_settings::read(&mut state.db.acquire().await.unwrap())
                .await
                .unwrap()
                .version,
            8
        );
        state.storage.close().await;
    }
    #[tokio::test]
    async fn unavailable_album_does_not_block_artist_and_recovers_without_losing_checkpoint() {
        let (_dir, state) = setup().await;
        enqueue(&state).await;
        let sources = Sources {
            calls: Default::default(),
            album_outage: true.into(),
        };
        let lease = state.storage.claim("artwork").await.unwrap().unwrap();
        assert!(step_with(&state, &lease, &sources).await.unwrap());
        sqlx::query("DELETE FROM settings WHERE key='artwork_background_next_at_ms'")
            .execute(&state.db)
            .await
            .unwrap();
        assert!(step_with(&state, &lease, &sources).await.unwrap());
        let lease = state.storage.job(&lease.id).await.unwrap();
        assert_eq!(lease.processed, 1);
        assert!(!step_with(&state, &lease, &sources).await.unwrap());
        let waiting = state.storage.job(&lease.id).await.unwrap();
        assert_eq!(waiting.error_code.as_deref(), Some("artwork_provider_wait"));
        assert!(waiting.retry_at_ms.unwrap() > auth::now_ms() + 110_000);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM artwork_misses")
                .fetch_one(&state.db)
                .await
                .unwrap(),
            0
        );
        // On recovery use a real public-style Found response through the same cache/queue path.
        struct Recovered;
        impl crate::artwork_sources::ImageLookup for Recovered {
            async fn lookup(
                &self,
                provider: crate::artwork_settings::ImageProvider,
                _: &str,
                _: &str,
            ) -> Result<crate::artwork_sources::Lookup> {
                Sources {
                    calls: Default::default(),
                    album_outage: Default::default(),
                }
                .lookup(provider, "artists", "")
                .await
            }
        }
        sqlx::query("UPDATE job_members SET artwork_retry_at_ms=0 WHERE job_id=?")
            .bind(&lease.id)
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("UPDATE jobs SET retry_at_ms=0 WHERE id=?")
            .bind(&lease.id)
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("DELETE FROM settings WHERE key='artwork_background_next_at_ms'")
            .execute(&state.db)
            .await
            .unwrap();
        let lease = state.storage.claim("artwork").await.unwrap().unwrap();
        assert!(step_with(&state, &lease, &Recovered).await.unwrap());
        let lease = state.storage.job(&lease.id).await.unwrap();
        assert!(!step_with(&state, &lease, &Recovered).await.unwrap());
        let finished = state.storage.job(&lease.id).await.unwrap();
        assert_eq!(
            (finished.state.as_str(), finished.processed, finished.added),
            ("completed", 2, 2)
        );
        state.storage.close().await;
    }
    #[tokio::test]
    async fn host_cooldown_is_persistent_and_does_not_block_other_sources() {
        use crate::artwork_sources::{request_failed, request_slot};
        let (dir, state) = setup().await;
        let at = auth::now_ms() + 1_200_000;
        assert!(
            matches!(request_failed(&state,"coverartarchive.org",Some(at)).await.unwrap(), AppError::ArtworkWait { retry_at_ms } if retry_at_ms == at)
        );
        request_slot(&state, "musicbrainz.org").await.unwrap();
        let config = state.config.clone();
        state.storage.close().await;
        drop(state);
        let state = AppState::new(storage::open(dir.path()).await.unwrap(), config);
        assert!(
            matches!(request_slot(&state,"coverartarchive.org").await, Err(AppError::ArtworkWait { retry_at_ms }) if retry_at_ms == at)
        );
        request_slot(&state, "commons.wikimedia.org").await.unwrap();
        // A later shorter pause cannot erase an outstanding Retry-After.
        assert!(
            matches!(request_failed(&state,"coverartarchive.org",None).await.unwrap(), AppError::ArtworkWait { retry_at_ms } if retry_at_ms >= at)
        );
        state.storage.close().await;
    }
    #[tokio::test]
    async fn public_queue_does_not_inherit_a_minute_per_image() {
        let (_dir, state) = setup().await;
        enqueue(&state).await;
        let lease = state.storage.claim("artwork").await.unwrap().unwrap();
        let before = auth::now_ms();
        assert!(
            background_slot(&state, &lease, "albums", "one")
                .await
                .unwrap()
        );
        assert!(
            !background_slot(&state, &lease, "albums", "two")
                .await
                .unwrap()
        );
        let waiting = state.storage.job(&lease.id).await.unwrap();
        assert!(
            waiting.retry_at_ms.unwrap() <= before + 2000,
            "public requests already have a MusicBrainz limiter; no minute per image"
        );
    }
    #[tokio::test]
    async fn background_cadence_and_user_pause_survive_restart_without_losing_progress() {
        let (dir, state) = setup().await;
        enqueue(&state).await;
        cache_all(&state).await;
        let lease = state.storage.claim("artwork").await.unwrap().unwrap();
        assert!(step(&state, &lease).await.unwrap());
        let lease = state.storage.job(&lease.id).await.unwrap();
        sqlx::query("DELETE FROM artwork")
            .execute(&state.db)
            .await
            .unwrap();
        let before = auth::now_ms();
        assert!(
            background_slot(&state, &lease, "albums", "one")
                .await
                .unwrap()
        );
        assert!(
            !background_slot(&state, &lease, "albums", "two")
                .await
                .unwrap()
        );
        let paced = state.storage.job(&lease.id).await.unwrap();
        assert_eq!(paced.error_code.as_deref(), Some("artwork_pacing"));
        assert!(paced.retry_at_ms.unwrap() >= before + IMAGE_INTERVAL_MS);
        // Keep both persisted deadlines pending regardless of restart speed.
        let deadline = i64::MAX;
        sqlx::query("UPDATE settings SET value=? WHERE key='artwork_background_next_at_ms'")
            .bind(deadline.to_string())
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("UPDATE jobs SET retry_at_ms=? WHERE id=?")
            .bind(deadline)
            .bind(&lease.id)
            .execute(&state.db)
            .await
            .unwrap();
        state.storage.pause_artwork(&lease.id).await.unwrap();
        assert!(step(&state, &lease).await.is_err());
        let id = lease.id;
        let config = state.config.clone();
        state.storage.close().await;
        drop(state);
        let state = AppState::new(storage::open(dir.path()).await.unwrap(), config);
        enqueue(&state).await;
        let paused = state.storage.job(&id).await.unwrap();
        assert_eq!(paused.error_code.as_deref(), Some("artwork_paused"));
        assert_eq!(paused.processed, 1);
        assert!(state.storage.claim("artwork").await.unwrap().is_none());
        state.storage.resume_artwork(&id).await.unwrap();
        assert!(
            state.storage.claim("artwork").await.unwrap().is_none(),
            "resuming must not bypass the saved cooldown"
        );
        sqlx::query("UPDATE jobs SET retry_at_ms=0 WHERE id=?")
            .bind(&id)
            .execute(&state.db)
            .await
            .unwrap();
        let resumed = state.storage.claim("artwork").await.unwrap().unwrap();
        assert_eq!(resumed.processed, 1);
        assert!(
            !background_slot(&state, &resumed, "albums", "two")
                .await
                .unwrap(),
            "the saved cadence survives restart too"
        );
    }
    #[tokio::test]
    async fn queue_coalesces_imports_reuses_saved_images_and_completes_without_spotify() {
        let (_dir, state) = setup().await;
        enqueue(&state).await;
        enqueue(&state).await;
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM jobs WHERE kind='artwork'")
                .fetch_one(&state.db)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM job_members")
                .fetch_one(&state.db)
                .await
                .unwrap(),
            2
        );
        // The browser saved these while the queue was pending: no provider call.
        cache_all(&state).await;
        let mut lease = state.storage.claim("artwork").await.unwrap().unwrap();
        while step(&state, &lease).await.unwrap() {
            lease = state.storage.job(&lease.id).await.unwrap();
        }
        let finished = state.storage.job(&lease.id).await.unwrap();
        assert_eq!(
            (
                finished.state.as_str(),
                finished.processed,
                finished.duplicates
            ),
            ("completed", 2, 2)
        );
        enqueue(&state).await;
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM jobs WHERE kind='artwork'")
                .fetch_one(&state.db)
                .await
                .unwrap(),
            1
        );
        // Personal images and a valid negative cache do not enter a new queue.
        sqlx::query("DELETE FROM artwork WHERE kind='artists'")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO artwork_misses VALUES('artists',(SELECT id FROM name_groups WHERE kind='artists'),0,?)")
            .bind(auth::now_ms() + 100000)
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("UPDATE artwork SET source='custom' WHERE kind='albums'")
            .execute(&state.db)
            .await
            .unwrap();
        enqueue(&state).await;
        assert!(state.storage.claim("artwork").await.unwrap().is_none());
    }
    #[tokio::test]
    async fn queue_checkpoint_survives_restart_and_cancel_and_maintenance_are_fenced() {
        let (dir, state) = setup().await;
        enqueue(&state).await;
        cache_all(&state).await;
        let lease = state.storage.claim("artwork").await.unwrap().unwrap();
        assert!(step(&state, &lease).await.unwrap());
        assert!(
            step(&state, &lease).await.is_err(),
            "a stale checkpoint cannot acknowledge another image"
        );
        let config = state.config.clone();
        let id = lease.id;
        state.storage.close().await;
        drop(state);
        let state = AppState::new(storage::open(dir.path()).await.unwrap(), config);
        let lease = state.storage.claim("artwork").await.unwrap().unwrap();
        assert_eq!(lease.id, id);
        assert_eq!(lease.processed, 1);
        assert_eq!(lease.attempt, 2);
        sqlx::query("UPDATE archive_state SET maintenance=1")
            .execute(&state.db)
            .await
            .unwrap();
        assert!(step(&state, &lease).await.is_err());
        sqlx::query("UPDATE archive_state SET maintenance=0")
            .execute(&state.db)
            .await
            .unwrap();
        state.storage.cancel(&id).await.unwrap();
        assert!(step(&state, &lease).await.is_err());
        state.storage.retry(&id).await.unwrap();
        let mut lease = state.storage.claim("artwork").await.unwrap().unwrap();
        while step(&state, &lease).await.unwrap() {
            lease = state.storage.job(&id).await.unwrap();
        }
        let complete = state.storage.job(&id).await.unwrap();
        assert_eq!(complete.processed, 2);
        assert_eq!(complete.duplicates, 2);
    }
}
