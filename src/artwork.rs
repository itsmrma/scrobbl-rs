//! Durable import enrichment. One shared catalogue slot, paced requests, saved checkpoints.
use crate::{
    AppState, auth, db,
    error::{AppError, Result},
    jobs::{self, Job},
    library::{self, ArtworkFetch},
};
use sqlx::Row;
use std::time::Duration;

// Optional enrichment must remain a trickle beside live playback/collection.
pub const IMAGE_INTERVAL_MS: i64 = 60_000;

async fn background_slot(state: &AppState, lease: &Job, kind: &str, id: &str) -> Result<bool> {
    let mut tx = state.db.begin().await?;
    jobs::validate_lease(&mut tx, lease).await?;
    let cached: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM artwork WHERE kind=? AND entity_id=? AND (source='custom' OR expires_at_ms IS NULL OR expires_at_ms>?))")
        .bind(kind).bind(id).bind(auth::now_ms()).fetch_one(&mut *tx).await?;
    if cached {
        return Ok(true);
    }
    let settings = crate::artwork_settings::read(&mut tx).await?;
    if !settings
        .enabled(kind)
        .any(|p| p != crate::artwork_settings::ImageProvider::Spotify)
    {
        let connected: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM spotify_tokens)")
            .fetch_one(&state.private_db)
            .await?;
        if !connected {
            return Ok(true);
        }
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
    sqlx::query(r#"INSERT INTO job_members(job_id,member,kind,state,expected_rows)
      SELECT ?,g.kind||':'||g.id,g.kind,'pending',1 FROM name_groups g
      WHERE g.kind IN ('artists','albums')
        AND ((g.kind='albums' AND EXISTS(SELECT 1 FROM name_group_members gm JOIN tracks t ON t.album_id=gm.entity_id JOIN listens l ON l.track_id=t.id WHERE gm.kind='albums' AND gm.group_id=g.id AND l.trashed_at_ms IS NULL))
          OR (g.kind='artists' AND EXISTS(SELECT 1 FROM name_group_members gm JOIN track_artists ta ON ta.artist_id=gm.entity_id JOIN listens l ON l.track_id=ta.track_id WHERE gm.kind='artists' AND gm.group_id=g.id AND l.trashed_at_ms IS NULL)))
        AND NOT EXISTS(SELECT 1 FROM artwork_misses m WHERE m.kind=g.kind AND m.entity_id=g.id AND m.expires_at_ms>?)
        AND NOT EXISTS(SELECT 1 FROM artwork a WHERE a.kind=g.kind AND a.entity_id=g.id AND (a.source='custom' OR a.expires_at_ms IS NULL OR a.expires_at_ms>?))
      ON CONFLICT(job_id,member) DO NOTHING"#).bind(&id).bind(auth::now_ms()).bind(auth::now_ms()).execute(&mut *conn).await?;
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

async fn wait(state: &AppState, lease: &Job, code: &str, fallback: i64) -> Result<()> {
    let mut retry = auth::now_ms().saturating_add(fallback);
    for key in ["retry_at_ms", "artwork_retry_at_ms"] {
        if let Some(at) = db::setting(&state.db, key)
            .await?
            .and_then(|v| v.parse::<i64>().ok())
        {
            retry = retry.max(at);
        }
    }
    state.storage.pause(lease, Some(retry), code).await
}

/// Process one bounded item. Cancellation and maintenance fence every checkpoint.
pub async fn step(state: &AppState, lease: &Job) -> Result<bool> {
    let mut reservation = state.db.begin().await?;
    jobs::validate_lease(&mut reservation, lease).await?;
    reservation.commit().await?;
    let item=sqlx::query("SELECT member,kind FROM job_members WHERE job_id=? AND state='pending' ORDER BY member LIMIT 1").bind(&lease.id).fetch_optional(&state.read_db).await?;
    let Some(item) = item else {
        let mut tx = state.db.begin().await?;
        jobs::validate_lease(&mut tx, lease).await?;
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
            library::ensure_artwork(state, &kind, &id).await
        }
        Err(AppError::NotFound) => Ok(ArtworkFetch::Missing),
        Err(error) => Err(error),
    };
    let result = match result {
        Ok(ArtworkFetch::Busy) => {
            wait(state, lease, "artwork_capacity_wait", 1000).await?;
            return Ok(false);
        }
        Err(AppError::Unauthorized) => {
            wait(state, lease, "artwork_connection_required", 60_000).await?;
            return Ok(false);
        }
        Err(AppError::Conflict(error)) => return Err(AppError::Conflict(error)),
        Err(_) => {
            wait(state, lease, "artwork_provider_wait", 60_000).await?;
            return Ok(false);
        }
        Ok(result) => result,
    };
    let mut tx = state.db.begin().await?;
    jobs::validate_lease(&mut tx, lease).await?;
    sqlx::query("UPDATE job_members SET state='completed',processed=1,added=?,duplicates=?,skipped=? WHERE job_id=? AND member=? AND state='pending'").bind(i64::from(result==ArtworkFetch::Saved)).bind(i64::from(result==ArtworkFetch::Cached)).bind(i64::from(result==ArtworkFetch::Missing)).bind(&lease.id).bind(member).execute(&mut *tx).await?;
    sqlx::query("UPDATE jobs SET checkpoint=checkpoint+1,processed=processed+1,added=added+?,duplicates=duplicates+?,skipped=skipped+?,phase='artwork',error_code=NULL,retry_at_ms=NULL,updated_at_ms=? WHERE id=?").bind(i64::from(result==ArtworkFetch::Saved)).bind(i64::from(result==ArtworkFetch::Cached)).bind(i64::from(result==ArtworkFetch::Missing)).bind(auth::now_ms()).bind(&lease.id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(true)
}

pub async fn worker(state: AppState) {
    loop {
        // Connecting Spotify wakes retained work immediately, without repeating
        // failed authentication requests or waiting out an artificial cooldown.
        if sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM spotify_tokens)")
            .fetch_one(&state.private_db)
            .await
            .unwrap_or(false)
        {
            let _=sqlx::query("UPDATE jobs SET retry_at_ms=0 WHERE kind='artwork' AND state='waiting' AND error_code='artwork_connection_required'").execute(&state.db).await;
        }
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
                // One download at a time; the existing shared Spotify limiter
                // also paces each catalogue request and respects Retry-After.
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
        sqlx::query("INSERT INTO spotify_tokens VALUES(1,'test','test',9999999999999)")
            .execute(&state.private_db)
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
        sqlx::query("UPDATE artwork SET bytes=NULL,mime=NULL,expires_at_ms=? WHERE kind='artists'")
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
    #[tokio::test]
    async fn queue_waits_for_connection_and_respects_shared_provider_retry_after() {
        let (_dir, mut state) = setup().await;
        let mut settings = crate::artwork_settings::ArtworkSettings::default();
        for p in &mut settings.providers {
            p.enabled = p.provider == crate::artwork_settings::ImageProvider::Spotify;
        }
        db::set_setting(
            &mut state.db.acquire().await.unwrap(),
            "artwork_providers",
            &serde_json::to_string(&settings).unwrap(),
        )
        .await
        .unwrap();
        enqueue(&state).await;
        let lease = state.storage.claim("artwork").await.unwrap().unwrap();
        assert!(!step(&state, &lease).await.unwrap());
        let waiting = state.storage.job(&lease.id).await.unwrap();
        assert_eq!(waiting.state, "waiting");
        assert_eq!(
            waiting.error_code.as_deref(),
            Some("artwork_connection_required")
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM artwork")
                .fetch_one(&state.db)
                .await
                .unwrap(),
            0
        );
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = calls.clone();
        let app = axum::Router::new().route(
            "/{*path}",
            axum::routing::get(move || {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    (
                        axum::http::StatusCode::TOO_MANY_REQUESTS,
                        [("retry-after", "1200")],
                        axum::Json(json!({"error":"wait"})),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        state.spotify.api_base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        sqlx::query("INSERT INTO spotify_tokens VALUES(1,'test','test',9999999999999)")
            .execute(&state.private_db)
            .await
            .unwrap();
        state.storage.cancel(&lease.id).await.unwrap();
        state.storage.retry(&lease.id).await.unwrap();
        let lease = state.storage.claim("artwork").await.unwrap().unwrap();
        let started = auth::now_ms();
        assert!(!step(&state, &lease).await.unwrap());
        let waiting = state.storage.job(&lease.id).await.unwrap();
        assert_eq!(waiting.state, "waiting");
        assert!(waiting.retry_at_ms.unwrap() >= started + 1_199_000);
        assert!(state.storage.claim("artwork").await.unwrap().is_none());
        assert!(step(&state, &lease).await.is_err());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM artwork")
                .fetch_one(&state.db)
                .await
                .unwrap(),
            0
        );
        server.abort();
    }
}
