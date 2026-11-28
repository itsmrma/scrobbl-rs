//! Library artwork: one resolver for browser and background, and the queue
//! that completes the library by itself.
//!
//! Rewritten from scratch. The old engine serialized every image behind one
//! global slot, paced each request through SQLite, slept a second between
//! images and asked the slowest sources (MusicBrainz at one request per
//! second) first. Now:
//!
//! - providers run fast-first (Deezer, iTunes, then MusicBrainz sources) and
//!   each host is paced on its own, in memory;
//! - the queue resolves several entities at once and starts by itself;
//! - a browser request starts a download that survives the request, and waits
//!   for an in-flight download of the same image instead of failing;
//! - temporary provider trouble is never cached as "no image".
use crate::{
    AppState,
    artwork_settings::{self, ArtworkSettings},
    artwork_sources::{ImageLookup, Lookup, PublicSources},
    auth, db,
    error::{AppError, Result},
    jobs::{self, Job},
};
use axum::{Json, extract::State, http::HeaderMap};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Entities resolved at once by the background queue. Providers keep their
/// own per-host pace, so this only overlaps waiting on different hosts.
const PARALLEL: usize = 4;
/// A confirmed "no provider has it" is retried after three days.
const MISS_TTL_MS: i64 = 3 * 86_400_000;
/// How long a page waits for a cold image before asking the browser to retry.
const BROWSER_WAIT: Duration = Duration::from_secs(8);

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum ArtworkFetch {
    Cached,
    Saved,
    Missing,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackfillResult {
    pub ok: bool,
    pub queued: i64,
    pub queued_albums: i64,
    pub queued_artists: i64,
}

pub async fn backfill(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<BackfillResult>> {
    auth::require_write(&state, &headers).await?;
    crate::library::writable(&state).await?;
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision")
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::Conflict("Archive is in maintenance.".into()))?;
    sqlx::query(
        "DELETE FROM artwork_misses WHERE entity_id NOT IN (SELECT entity_id FROM artwork WHERE bytes IS NOT NULL)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE job_members SET state='pending', artwork_retry_at_ms=0, artwork_attempts=0, error_code=NULL, processed=0, skipped=0 \
         WHERE job_id IN (SELECT id FROM jobs WHERE kind='artwork' AND state IN ('queued','running','waiting')) \
           AND (state='pending' OR state='failed' OR (state='completed' AND skipped=1))"
    )
    .execute(&mut *tx)
    .await?;
    enqueue_missing(&mut tx).await?;
    sqlx::query(
        "UPDATE jobs SET retry_at_ms=0, error_code=CASE WHEN error_code='artwork_paused' THEN NULL ELSE error_code END \
         WHERE kind='artwork' AND state='waiting'"
    )
    .execute(&mut *tx)
    .await?;
    let (queued, queued_albums, queued_artists): (i64, i64, i64) = sqlx::query_as(
        "SELECT \
            COUNT(*), \
            COALESCE(SUM(CASE WHEN jm.kind='albums' THEN 1 ELSE 0 END), 0), \
            COALESCE(SUM(CASE WHEN jm.kind='artists' THEN 1 ELSE 0 END), 0) \
         FROM job_members jm JOIN jobs j ON j.id=jm.job_id \
         WHERE j.kind='artwork' AND jm.state='pending' AND j.state IN ('queued','running','waiting')"
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    state.storage.wake_workers();
    Ok(Json(BackfillResult {
        ok: true,
        queued,
        queued_albums,
        queued_artists,
    }))
}

async fn cached(state: &AppState, kind: &str, id: &str) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM artwork WHERE kind=? AND entity_id=? AND bytes IS NOT NULL)",
    )
    .bind(kind)
    .bind(id)
    .fetch_one(&state.read_db)
    .await?)
}

/// One lock per image: a second caller waits for the first download instead
/// of starting a duplicate or giving up.
async fn entity_gate(
    state: &AppState,
    kind: &str,
    id: &str,
) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    let mut locks = state.artwork_gate.lock().await;
    locks.retain(|_, lock| lock.strong_count() > 0);
    let key = format!("{kind}:{id}");
    if let Some(lock) = locks.get(&key).and_then(std::sync::Weak::upgrade) {
        return lock;
    }
    let lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
    locks.insert(key, std::sync::Arc::downgrade(&lock));
    lock
}

/// Find and save the image of one artist or album group.
pub(crate) async fn resolve_with(
    state: &AppState,
    kind: &str,
    id: &str,
    sources: &impl ImageLookup,
    activity: Option<(&str, &str)>,
) -> Result<ArtworkFetch> {
    if cached(state, kind, id).await? {
        return Ok(ArtworkFetch::Cached);
    }
    let gate = entity_gate(state, kind, id).await;
    let _gate = gate.lock().await;
    if cached(state, kind, id).await? {
        return Ok(ArtworkFetch::Cached);
    }
    crate::library::writable(state).await?;
    let settings = artwork_settings::read(&mut *state.read_db.acquire().await?).await?;
    let missing: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM artwork_misses WHERE kind=? AND entity_id=? AND settings_version=? AND expires_at_ms>?)").bind(kind).bind(id).bind(settings.version).bind(auth::now_ms()).fetch_one(&state.read_db).await?;
    if missing {
        return Ok(ArtworkFetch::Missing);
    }
    let target = {
        let mut conn = state.read_db.acquire().await?;
        crate::artwork_sources::target(&mut conn, kind, id).await?
    };
    let Some(target) = target else {
        return Ok(ArtworkFetch::Missing);
    };
    let _capacity = state
        .artwork_fetches
        .acquire()
        .await
        .map_err(|_| AppError::Internal)?;
    let mut earliest_wait: Option<i64> = None;
    let mut last_error: Option<AppError> = None;
    for provider in settings.enabled(kind) {
        if let Some((job, member)) = activity {
            let _ = sqlx::query("UPDATE job_members SET current_provider=? WHERE job_id=? AND member=? AND state='pending'")
                .bind(provider.key()).bind(job).bind(member).execute(&state.db).await;
        }
        match sources.lookup(provider, &target).await {
            Ok(Lookup::Found(image)) => {
                return save(state, &settings, kind, id, provider.key(), image).await;
            }
            Ok(Lookup::Missing) => {}
            Err(AppError::ArtworkWait { retry_at_ms }) => {
                earliest_wait = Some(earliest_wait.map_or(retry_at_ms, |at| at.min(retry_at_ms)));
            }
            Err(error @ (AppError::Database(_) | AppError::Conflict(_))) => return Err(error),
            Err(error) => last_error = Some(error),
        }
    }
    // A provider that could not answer may still have the image: never cache
    // that as an absence.
    if let Some(retry_at_ms) = earliest_wait {
        return Err(AppError::ArtworkWait { retry_at_ms });
    }
    if let Some(error) = last_error {
        return Err(error);
    }
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision").fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::Conflict("Archive is in maintenance.".into()))?;
    if artwork_settings::read(&mut tx).await?.version == settings.version {
        sqlx::query("INSERT INTO artwork_misses VALUES(?,?,?,?) ON CONFLICT(kind,entity_id) DO UPDATE SET settings_version=excluded.settings_version,expires_at_ms=excluded.expires_at_ms").bind(kind).bind(id).bind(settings.version).bind(auth::now_ms() + MISS_TTL_MS).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(ArtworkFetch::Missing)
}

async fn save(
    state: &AppState,
    settings: &ArtworkSettings,
    kind: &str,
    id: &str,
    provider: &str,
    image: crate::artwork_sources::FoundImage,
) -> Result<ArtworkFetch> {
    let mime = crate::library::image_type(&image.bytes)?;
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE archive_state SET revision=revision WHERE singleton=1 AND maintenance=0 RETURNING revision").fetch_optional(&mut *tx).await?.ok_or_else(|| AppError::Conflict("Archive is in maintenance.".into()))?;
    // Settings changed while downloading: that result belongs to old choices.
    if artwork_settings::read(&mut tx).await?.version != settings.version {
        return Err(AppError::ArtworkWait {
            retry_at_ms: auth::now_ms(),
        });
    }
    // A personal image saved meanwhile always wins.
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM artwork WHERE kind=? AND entity_id=? AND bytes IS NOT NULL)",
    )
    .bind(kind)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    if exists {
        return Ok(ArtworkFetch::Cached);
    }
    sqlx::query("INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms,expires_at_ms,provenance_json) VALUES(?,?,?,?,?,?,NULL,?) ON CONFLICT(kind,entity_id,source) DO UPDATE SET bytes=excluded.bytes,mime=excluded.mime,checked_at_ms=excluded.checked_at_ms,expires_at_ms=NULL,provenance_json=excluded.provenance_json")
        .bind(kind).bind(id).bind(provider).bind(&image.bytes).bind(mime).bind(auth::now_ms()).bind(serde_json::to_string(&image.credit).map_err(|_| AppError::Internal)?).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM artwork_misses WHERE kind=? AND entity_id=?")
        .bind(kind)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(ArtworkFetch::Saved)
}

#[cfg(test)]
pub(crate) async fn resolve(state: &AppState, kind: &str, id: &str) -> Result<ArtworkFetch> {
    resolve_with(state, kind, id, &PublicSources, None).await
}

/// Page path: the download runs in its own task so a closed tab or a slow
/// provider never wastes it; the page waits a few seconds, then retries.
pub(crate) async fn resolve_for_browser(
    state: &AppState,
    kind: &str,
    id: &str,
) -> Result<ArtworkFetch> {
    let (task_state, kind, id) = (state.clone(), kind.to_owned(), id.to_owned());
    let task =
        tokio::spawn(
            async move { resolve_with(&task_state, &kind, &id, &PublicSources, None).await },
        );
    match tokio::time::timeout(BROWSER_WAIT, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(AppError::Internal),
        Err(_) => Err(AppError::ArtworkWait {
            retry_at_ms: auth::now_ms() + 2_000,
        }),
    }
}

/// 30 s, 1 min, 2 min … capped at one hour, unless a provider said when.
fn retry_at(attempts: i64, at: Option<i64>) -> i64 {
    let backoff = 30_000_i64
        .saturating_mul(1_i64 << attempts.clamp(0, 7))
        .min(3_600_000);
    at.unwrap_or_else(|| auth::now_ms().saturating_add(backoff))
}

/// Resolve up to [`PARALLEL`] due items. Cancellation and maintenance fence
/// every checkpoint. `Ok(false)` means the job is done or waiting.
pub async fn step(state: &AppState, lease: &Job) -> Result<bool> {
    step_with(state, lease, &PublicSources).await
}

async fn step_with(state: &AppState, lease: &Job, sources: &impl ImageLookup) -> Result<bool> {
    let mut check = state.db.begin().await?;
    jobs::validate_lease(&mut check, lease).await?;
    check.commit().await?;
    let items: Vec<(String, String)> = sqlx::query_as("SELECT member,kind FROM job_members WHERE job_id=? AND state='pending' AND artwork_retry_at_ms<=? ORDER BY artwork_attempts,artwork_retry_at_ms,member LIMIT ?")
        .bind(&lease.id).bind(auth::now_ms()).bind(PARALLEL as i64).fetch_all(&state.read_db).await?;
    if items.is_empty() {
        let mut tx = state.db.begin().await?;
        jobs::validate_lease(&mut tx, lease).await?;
        let pending: Option<i64> = sqlx::query_scalar(
            "SELECT MIN(artwork_retry_at_ms) FROM job_members WHERE job_id=? AND state='pending'",
        )
        .bind(&lease.id)
        .fetch_one(&mut *tx)
        .await?;
        if let Some(at) = pending {
            tx.rollback().await?;
            state
                .storage
                .pause(
                    lease,
                    Some(at.max(auth::now_ms() + 1000)),
                    "artwork_provider_wait",
                )
                .await?;
            return Ok(false);
        }
        // A concurrent import may have appended work since the read above.
        let done=sqlx::query("UPDATE jobs SET state='completed',phase='finished',outcome='complete',error_code=NULL,retry_at_ms=NULL,updated_at_ms=? WHERE id=? AND NOT EXISTS(SELECT 1 FROM job_members WHERE job_id=? AND state='pending')").bind(auth::now_ms()).bind(&lease.id).bind(&lease.id).execute(&mut *tx).await?.rows_affected()==1;
        tx.commit().await?;
        return Ok(!done);
    }
    let results = futures_util::future::join_all(items.iter().map(|(member, kind)| async move {
        let Some(id) = member.strip_prefix(&format!("{kind}:")) else {
            return Err(AppError::Internal);
        };
        let id = match crate::name_groups::canonical(&mut *state.read_db.acquire().await?, kind, id)
            .await
        {
            Ok(id) => id,
            Err(AppError::NotFound) => return Ok(ArtworkFetch::Missing),
            Err(error) => return Err(error),
        };
        resolve_with(state, kind, &id, sources, Some((&lease.id, member))).await
    }))
    .await;
    // One checkpoint for the whole batch, so the lease stays current.
    let mut tx = state.db.begin().await?;
    jobs::validate_lease(&mut tx, lease).await?;
    let (mut added, mut duplicates, mut skipped, mut processed) = (0, 0, 0, 0);
    for ((member, _), result) in items.iter().zip(results) {
        let fetch = match result {
            Ok(fetch) => fetch,
            Err(error @ (AppError::Database(_) | AppError::Conflict(_))) => return Err(error),
            Err(error) => {
                let at = match error {
                    AppError::ArtworkWait { retry_at_ms } => Some(retry_at_ms),
                    _ => None,
                };
                let attempts: i64 = sqlx::query_scalar(
                    "SELECT artwork_attempts FROM job_members WHERE job_id=? AND member=?",
                )
                .bind(&lease.id)
                .bind(member)
                .fetch_one(&mut *tx)
                .await?;
                sqlx::query("UPDATE job_members SET artwork_retry_at_ms=?,artwork_attempts=artwork_attempts+1,error_code='artwork_provider_wait',current_provider=NULL WHERE job_id=? AND member=?")
                    .bind(retry_at(attempts, at)).bind(&lease.id).bind(member).execute(&mut *tx).await?;
                continue;
            }
        };
        let flags = (
            i64::from(fetch == ArtworkFetch::Saved),
            i64::from(fetch == ArtworkFetch::Cached),
            i64::from(fetch == ArtworkFetch::Missing),
        );
        let changed = sqlx::query("UPDATE job_members SET state='completed',artwork_retry_at_ms=0,error_code=NULL,current_provider=NULL,processed=1,added=?,duplicates=?,skipped=? WHERE job_id=? AND member=? AND state='pending'").bind(flags.0).bind(flags.1).bind(flags.2).bind(&lease.id).bind(member).execute(&mut *tx).await?.rows_affected();
        if changed == 1 {
            added += flags.0;
            duplicates += flags.1;
            skipped += flags.2;
            processed += 1;
        }
    }
    sqlx::query("UPDATE jobs SET checkpoint=checkpoint+1,processed=processed+?,added=added+?,duplicates=duplicates+?,skipped=skipped+?,phase='artwork',error_code=NULL,retry_at_ms=NULL,updated_at_ms=? WHERE id=?").bind(processed).bind(added).bind(duplicates).bind(skipped).bind(auth::now_ms()).bind(&lease.id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(true)
}

/// Queue everything still missing, so a fresh install or an upgrade
/// completes the library without anyone pressing a button.
async fn enqueue_on_start(state: &AppState) -> Result<()> {
    let mut tx = state.db.begin().await?;
    let writable: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM archive_state WHERE singleton=1 AND maintenance=0)",
    )
    .fetch_one(&mut *tx)
    .await?;
    if writable {
        enqueue_missing(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn worker(state: AppState) {
    let _ = enqueue_on_start(&state).await;
    loop {
        let notified = state.storage.jobs_notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
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
                match state.storage.job(&lease.id).await {
                    Ok(current) if current.state == "running" => lease = current,
                    _ => break,
                }
            }
            continue;
        }
        let wait = state
            .storage
            .next_retry_delay(&["artwork"], Duration::from_secs(60))
            .await;
        tokio::select! {
            _ = &mut notified => {},
            _ = tokio::time::sleep(wait) => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        artwork_settings::ImageProvider,
        artwork_sources::{FoundImage, ImageCredit, Target},
        config::Config,
        db::{DurationKind, Event},
        storage,
    };
    use serde_json::json;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    fn event(index: u32, title: &str, album: (&str, &str), artists: &[(&str, &str)]) -> Event {
        let raw = json!({
            "uri": format!("spotify:track:{index:022}"),
            "name": title,
            "album": {"id": album.0, "name": album.1, "images": []},
            "artists": artists.iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>(),
        });
        Event {
            source: "spotify_api",
            source_key: format!("event-{index}"),
            source_timestamp_ms: 1_000_000 + i64::from(index),
            played_at_ms: 1_000_000 + i64::from(index),
            listened_ms: Some(180_000),
            duration_kind: DurationKind::Estimated,
            counted: true,
            track: crate::spotify::parse_track(&raw).unwrap(),
            raw,
        }
    }

    async fn setup_with(events: Vec<Event>) -> (tempfile::TempDir, AppState) {
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
        let mut tx = state.db.begin().await.unwrap();
        db::ingest(&mut tx, &events).await.unwrap();
        tx.commit().await.unwrap();
        (dir, state)
    }

    async fn setup() -> (tempfile::TempDir, AppState) {
        setup_with(vec![event(
            1,
            "Song",
            ("0000000000000000000002", "Album"),
            &[("0000000000000000000003", "Artist")],
        )])
        .await
    }

    async fn enqueue(state: &AppState) {
        let mut tx = state.db.begin().await.unwrap();
        enqueue_missing(&mut tx).await.unwrap();
        tx.commit().await.unwrap();
    }

    async fn group(state: &AppState, kind: &str) -> String {
        sqlx::query_scalar("SELECT id FROM name_groups WHERE kind=? LIMIT 1")
            .bind(kind)
            .fetch_one(&state.db)
            .await
            .unwrap()
    }

    fn png(provider: ImageProvider) -> Lookup {
        Lookup::Found(FoundImage {
            bytes: b"\x89PNG\r\n\x1a\npublic".to_vec(),
            credit: ImageCredit {
                provider: provider.key().into(),
                source_url: "https://example.org/source".into(),
                image_url: "https://example.org/image.png".into(),
                author: None,
                license: None,
                license_url: None,
                credit: None,
            },
        })
    }

    /// Scripted providers: `answers` maps provider key + kind to an outcome.
    #[derive(Default)]
    struct Script {
        calls: Mutex<Vec<(ImageProvider, String)>>,
        answers: Mutex<Vec<(ImageProvider, &'static str, &'static str)>>,
        delay_ms: u64,
        concurrent: AtomicUsize,
        max_concurrent: AtomicUsize,
    }
    impl Script {
        fn new(answers: &[(ImageProvider, &'static str, &'static str)]) -> Self {
            Self {
                answers: Mutex::new(answers.to_vec()),
                ..Default::default()
            }
        }
    }
    impl ImageLookup for Script {
        async fn lookup(&self, provider: ImageProvider, target: &Target) -> Result<Lookup> {
            self.calls
                .lock()
                .unwrap()
                .push((provider, target.kind.clone()));
            let now = self.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_concurrent.fetch_max(now, Ordering::SeqCst);
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            self.concurrent.fetch_sub(1, Ordering::SeqCst);
            let answer = self
                .answers
                .lock()
                .unwrap()
                .iter()
                .find(|(p, kind, _)| *p == provider && *kind == target.kind)
                .map(|(_, _, answer)| *answer)
                .unwrap_or("missing");
            match answer {
                "found" => Ok(png(provider)),
                "wait" => Err(AppError::ArtworkWait {
                    retry_at_ms: auth::now_ms() + 120_000,
                }),
                "error" => Err(AppError::BadRequest("provider broke".into())),
                _ => Ok(Lookup::Missing),
            }
        }
    }

    async fn drain(state: &AppState, sources: &Script) -> Job {
        let mut lease = state.storage.claim("artwork").await.unwrap().unwrap();
        while step_with(state, &lease, sources).await.unwrap() {
            lease = state.storage.job(&lease.id).await.unwrap();
        }
        state.storage.job(&lease.id).await.unwrap()
    }

    #[tokio::test]
    async fn fast_providers_are_asked_first_and_misses_fall_through() {
        let (_dir, state) = setup().await;
        let script = Script::new(&[
            (ImageProvider::Itunes, "albums", "found"),
            (ImageProvider::Deezer, "artists", "found"),
        ]);
        enqueue(&state).await;
        let done = drain(&state, &script).await;
        assert_eq!(
            (done.state.as_str(), done.processed, done.added),
            ("completed", 2, 2)
        );
        // Album and artist resolve concurrently: check the order per kind.
        let calls = script.calls.lock().unwrap().clone();
        let order = |kind: &str| -> Vec<ImageProvider> {
            calls
                .iter()
                .filter(|(_, k)| k == kind)
                .map(|(p, _)| *p)
                .collect()
        };
        assert_eq!(
            order("albums"),
            vec![ImageProvider::Deezer, ImageProvider::Itunes],
            "albums try Deezer, then iTunes"
        );
        assert_eq!(
            order("artists"),
            vec![ImageProvider::Deezer],
            "the artist is found by Deezer at once"
        );
        let sources: Vec<(String, String)> =
            sqlx::query_as("SELECT kind,source FROM artwork ORDER BY kind")
                .fetch_all(&state.db)
                .await
                .unwrap();
        assert_eq!(
            sources,
            vec![
                ("albums".into(), "itunes".into()),
                ("artists".into(), "deezer".into())
            ]
        );
    }

    #[tokio::test]
    async fn confirmed_absence_is_cached_but_a_provider_wait_never_is() {
        let (_dir, state) = setup().await;
        // Albums: nobody has it. Artists: Deezer is cooling down.
        let script = Script::new(&[(ImageProvider::Deezer, "artists", "wait")]);
        enqueue(&state).await;
        let lease = state.storage.claim("artwork").await.unwrap().unwrap();
        assert!(step_with(&state, &lease, &script).await.unwrap());
        let lease = state.storage.job(&lease.id).await.unwrap();
        assert_eq!((lease.processed, lease.skipped), (1, 1));
        assert!(!step_with(&state, &lease, &script).await.unwrap());
        let waiting = state.storage.job(&lease.id).await.unwrap();
        assert_eq!(waiting.state, "waiting");
        assert_eq!(waiting.error_code.as_deref(), Some("artwork_provider_wait"));
        let misses: Vec<String> = sqlx::query_scalar("SELECT kind FROM artwork_misses")
            .fetch_all(&state.db)
            .await
            .unwrap();
        assert_eq!(misses, vec!["albums".to_string()]);

        // Recovery: the provider answers and the same queue completes.
        sqlx::query("UPDATE job_members SET artwork_retry_at_ms=0")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("UPDATE jobs SET retry_at_ms=0")
            .execute(&state.db)
            .await
            .unwrap();
        let done = drain(
            &state,
            &Script::new(&[(ImageProvider::Deezer, "artists", "found")]),
        )
        .await;
        assert_eq!(
            (done.state.as_str(), done.processed, done.added),
            ("completed", 2, 1)
        );
    }

    #[tokio::test]
    async fn the_queue_resolves_several_images_at_once() {
        let events = (0..8)
            .map(|i| {
                event(
                    i,
                    "Song",
                    (&format!("00000000000000000000a{i}"), &format!("Album {i}")),
                    &[("0000000000000000000003", "Artist")],
                )
            })
            .collect();
        let (_dir, state) = setup_with(events).await;
        enqueue(&state).await;
        let mut script = Script::new(&[(ImageProvider::Deezer, "albums", "found")]);
        script.delay_ms = 50;
        let done = drain(&state, &script).await;
        assert_eq!((done.state.as_str(), done.added), ("completed", 8));
        assert!(
            script.max_concurrent.load(Ordering::SeqCst) > 1,
            "images are no longer downloaded one at a time"
        );
    }

    #[tokio::test]
    async fn a_second_request_waits_for_the_download_in_flight() {
        let (_dir, state) = setup().await;
        let album = group(&state, "albums").await;
        let mut script = Script::new(&[(ImageProvider::Deezer, "albums", "found")]);
        script.delay_ms = 100;
        let (first, second) = tokio::join!(
            resolve_with(&state, "albums", &album, &script, None),
            resolve_with(&state, "albums", &album, &script, None),
        );
        let mut results = [first.unwrap(), second.unwrap()];
        results.sort_by_key(|r| *r == ArtworkFetch::Cached);
        assert_eq!(results, [ArtworkFetch::Saved, ArtworkFetch::Cached]);
        assert_eq!(
            script.calls.lock().unwrap().len(),
            1,
            "one download, not two"
        );
    }

    #[tokio::test]
    async fn settings_change_and_personal_images_fence_a_download() {
        let (_dir, state) = setup().await;
        let album = group(&state, "albums").await;
        sqlx::query("INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms) VALUES('albums',?,'custom',x'89504E470D0A1A0A','image/png',0)")
            .bind(&album)
            .execute(&state.db)
            .await
            .unwrap();
        let script = Script::new(&[(ImageProvider::Deezer, "albums", "found")]);
        assert_eq!(
            resolve_with(&state, "albums", &album, &script, None)
                .await
                .unwrap(),
            ArtworkFetch::Cached
        );
        assert!(script.calls.lock().unwrap().is_empty());

        let artist = group(&state, "artists").await;
        let settings = artwork_settings::read(&mut state.db.acquire().await.unwrap())
            .await
            .unwrap();
        let image = match png(ImageProvider::Deezer) {
            Lookup::Found(image) => image,
            Lookup::Missing => unreachable!(),
        };
        crate::db::set_setting(
            &mut state.db.acquire().await.unwrap(),
            "artwork_providers",
            &json!({"version": settings.version + 1, "providers": settings.providers}).to_string(),
        )
        .await
        .unwrap();
        assert!(matches!(
            save(&state, &settings, "artists", &artist, "deezer", image).await,
            Err(AppError::ArtworkWait { .. })
        ));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM artwork WHERE kind='artists'")
                .fetch_one(&state.db)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn queue_coalesces_imports_and_skips_saved_images_and_valid_absences() {
        let (_dir, state) = setup().await;
        enqueue(&state).await;
        enqueue(&state).await;
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM job_members")
                .fetch_one(&state.db)
                .await
                .unwrap(),
            2
        );
        sqlx::query("INSERT INTO artwork(kind,entity_id,source,bytes,mime,checked_at_ms) SELECT kind,id,'custom',x'89504E470D0A1A0A','image/png',0 FROM name_groups")
            .execute(&state.db)
            .await
            .unwrap();
        let script = Script::default();
        let done = drain(&state, &script).await;
        assert_eq!((done.state.as_str(), done.duplicates), ("completed", 2));
        assert!(script.calls.lock().unwrap().is_empty());
        sqlx::query("DELETE FROM artwork WHERE kind='artists'")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO artwork_misses SELECT 'artists',id,0,? FROM name_groups WHERE kind='artists'")
            .bind(auth::now_ms() + 100_000)
            .execute(&state.db)
            .await
            .unwrap();
        enqueue(&state).await;
        assert!(state.storage.claim("artwork").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn checkpoints_survive_restart_and_cancel_and_maintenance_are_fenced() {
        let (dir, state) = setup().await;
        enqueue(&state).await;
        let lease = state.storage.claim("artwork").await.unwrap().unwrap();
        let config = state.config.clone();
        let id = lease.id.clone();
        state.storage.close().await;
        drop(state);
        let state = AppState::new(storage::open(dir.path()).await.unwrap(), config);
        let lease = state.storage.claim("artwork").await.unwrap().unwrap();
        assert_eq!(lease.id, id);
        sqlx::query("UPDATE archive_state SET maintenance=1")
            .execute(&state.db)
            .await
            .unwrap();
        assert!(step_with(&state, &lease, &Script::default()).await.is_err());
        sqlx::query("UPDATE archive_state SET maintenance=0")
            .execute(&state.db)
            .await
            .unwrap();
        state.storage.cancel(&id).await.unwrap();
        assert!(step_with(&state, &lease, &Script::default()).await.is_err());
        state.storage.retry(&id).await.unwrap();
        let done = drain(
            &state,
            &Script::new(&[
                (ImageProvider::Deezer, "albums", "found"),
                (ImageProvider::Deezer, "artists", "found"),
            ]),
        )
        .await;
        assert_eq!((done.state.as_str(), done.added), ("completed", 2));
    }

    #[tokio::test]
    async fn album_artists_are_primary_credits_not_guests_on_one_song() {
        let bts = ("00000000000000000000b1", "BTS");
        let halsey = ("00000000000000000000b2", "Halsey");
        let mots = ("00000000000000000000c1", "MAP OF THE SOUL : 7");
        let (_dir, state) = setup_with(vec![
            event(1, "Boy With Luv (feat. Halsey)", mots, &[bts, halsey]),
            event(2, "ON", mots, &[bts]),
            event(3, "Black Swan", mots, &[bts]),
        ])
        .await;
        let album = group(&state, "albums").await;
        let artists = crate::library::album_artists(&mut state.db.acquire().await.unwrap(), &album)
            .await
            .unwrap();
        assert_eq!(
            artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            vec!["BTS"]
        );

        // A duo credited on every track of the album is an album artist pair.
        let (_dir, state) = setup_with(vec![
            event(
                4,
                "No Church in the Wild",
                ("00000000000000000000d1", "Watch the Throne"),
                &[
                    ("00000000000000000000e1", "JAY-Z"),
                    ("00000000000000000000e2", "Kanye West"),
                ],
            ),
            event(
                5,
                "Otis",
                ("00000000000000000000d1", "Watch the Throne"),
                &[
                    ("00000000000000000000e1", "JAY-Z"),
                    ("00000000000000000000e2", "Kanye West"),
                ],
            ),
        ])
        .await;
        let album = group(&state, "albums").await;
        let artists = crate::library::album_artists(&mut state.db.acquire().await.unwrap(), &album)
            .await
            .unwrap();
        assert_eq!(
            artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            vec!["JAY-Z", "Kanye West"]
        );
    }

    #[tokio::test]
    #[ignore = "contacts public artwork services; run explicitly, never uses the personal archive"]
    async fn public_artwork_smoke_downloads_album_and_artist() {
        let (_dir, state) = setup_with(vec![event(
            1,
            "Come Together",
            ("0000000000000000000002", "Abbey Road"),
            &[("0000000000000000000003", "The Beatles")],
        )])
        .await;
        for kind in ["albums", "artists"] {
            let id = group(&state, kind).await;
            let started = std::time::Instant::now();
            let result = resolve(&state, kind, &id).await;
            eprintln!("public smoke {kind}: {result:?} in {:?}", started.elapsed());
            assert_eq!(result.unwrap(), ArtworkFetch::Saved);
        }
        // Japanese titles and an album with a featured guest.
        let (_dir, state) = setup_with(vec![
            event(
                2,
                "唱",
                ("00000000000000000000f1", "狂言"),
                &[("00000000000000000000f2", "Ado")],
            ),
            event(
                3,
                "Boy With Luv (feat. Halsey)",
                ("00000000000000000000f3", "MAP OF THE SOUL : PERSONA"),
                &[
                    ("00000000000000000000f4", "BTS"),
                    ("00000000000000000000f5", "Halsey"),
                ],
            ),
        ])
        .await;
        let groups: Vec<(String, String)> =
            sqlx::query_as("SELECT kind,id FROM name_groups ORDER BY kind,id")
                .fetch_all(&state.db)
                .await
                .unwrap();
        for (kind, id) in groups {
            let started = std::time::Instant::now();
            let mut result = resolve(&state, &kind, &id).await;
            // MusicBrainz answers 503 when it is busy: wait as told, once or twice.
            for _ in 0..3 {
                let Err(AppError::ArtworkWait { retry_at_ms }) = result else {
                    break;
                };
                let wait = retry_at_ms
                    .saturating_sub(auth::now_ms())
                    .clamp(1_000, 15_000);
                tokio::time::sleep(Duration::from_millis(wait as u64)).await;
                result = resolve(&state, &kind, &id).await;
            }
            eprintln!(
                "public smoke {kind} {id}: {result:?} in {:?}",
                started.elapsed()
            );
            assert_eq!(result.unwrap(), ArtworkFetch::Saved);
        }
    }

    async fn owner_headers(state: &AppState) -> axum::http::HeaderMap {
        let cookie = auth::random_secret();
        sqlx::query("INSERT INTO sessions(digest,csrf,expires_at_ms) VALUES (?,'csrf',?)")
            .bind(auth::digest(&cookie))
            .bind(auth::now_ms() + 60000)
            .execute(&state.private_db)
            .await
            .unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "cookie",
            axum::http::HeaderValue::from_str(&format!("scrobbl_session={cookie}")).unwrap(),
        );
        headers.insert("x-csrf-token", axum::http::HeaderValue::from_static("csrf"));
        headers
    }

    #[tokio::test]
    async fn backfill_clears_absences_and_requeues_missing_artwork() {
        let (_dir, state) = setup().await;
        let headers = owner_headers(&state).await;
        let now = auth::now_ms();
        sqlx::query("INSERT INTO artwork(kind, entity_id, source, bytes, mime, checked_at_ms) VALUES ('artists', 'has-art', 'custom', X'89504E47', 'image/png', ?)")
            .bind(now)
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO artwork_misses VALUES ('artists','has-art',0,?),('artists','some-artist',0,?)")
            .bind(now + 86_400_000)
            .bind(now + 86_400_000)
            .execute(&state.db)
            .await
            .unwrap();
        let result = backfill(State(state.clone()), headers).await.unwrap().0;
        assert!(result.ok && result.queued == 2);
        assert_eq!(result.queued, result.queued_albums + result.queued_artists);
        let remaining: Vec<String> = sqlx::query_scalar("SELECT entity_id FROM artwork_misses")
            .fetch_all(&state.db)
            .await
            .unwrap();
        assert_eq!(remaining, vec!["has-art".to_string()]);
        assert!(
            backfill(State(state.clone()), axum::http::HeaderMap::new())
                .await
                .is_err()
        );
    }
}
